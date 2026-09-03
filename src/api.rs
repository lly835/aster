use std::{
    collections::BTreeMap,
    str::FromStr,
    sync::{
        atomic::{AtomicI64, AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use ethers_core::types::transaction::eip712::TypedData;
use ethers_signers::{LocalWallet, Signer};
use futures_util::{SinkExt, StreamExt};
use reqwest::{header, Method};
use rust_decimal::Decimal;
use serde::{de::DeserializeOwned, Deserialize};
use serde_json::{json, Value};
use thiserror::Error;
use tokio::sync::watch;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, info, warn};

use crate::config::Credentials;

#[derive(Debug, Error)]
pub enum AsterError {
    #[error("API credentials are required for this operation")]
    MissingCredentials,

    #[error("failed to initialize signer: {0}")]
    SignerInit(String),

    #[error("failed to sign request: {0}")]
    Signing(String),

    #[error("request transport error: {0}")]
    Transport(#[from] reqwest::Error),

    #[error("invalid JSON response: {0}")]
    Json(#[from] serde_json::Error),

    #[error("HTTP {status}: {body}")]
    Http { status: u16, body: String },

    #[error("Aster API error {code}: {message}")]
    Api { code: i64, message: String },

    #[error("request returned an unknown execution state: {0}")]
    ExecutionUnknown(String),

    #[error("invalid API response: {0}")]
    InvalidResponse(String),
}

impl AsterError {
    pub fn is_no_such_order(&self) -> bool {
        match self {
            Self::Api { code: -2013, .. } => true,
            Self::Api {
                code: -2011,
                message,
            } => {
                let message = message.to_ascii_lowercase();
                message.contains("unknown order") || message.contains("does not exist")
            }
            _ => false,
        }
    }

    pub fn is_transient_rejection(&self) -> bool {
        matches!(
            self,
            Self::Api {
                code: -2010 | -2020,
                ..
            }
        )
    }

    pub fn is_rate_limited(&self) -> bool {
        matches!(
            self,
            Self::Http {
                status: 418 | 429,
                ..
            }
        ) || matches!(self, Self::Api { code: -1003, .. })
    }

    pub fn is_execution_unknown(&self) -> bool {
        matches!(self, Self::ExecutionUnknown(_))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderSide {
    Buy,
    Sell,
}

impl OrderSide {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Buy => "BUY",
            Self::Sell => "SELL",
        }
    }

    pub fn short(self) -> char {
        match self {
            Self::Buy => 'b',
            Self::Sell => 's',
        }
    }
}

#[derive(Debug, Clone)]
pub struct SymbolRules {
    pub symbol: String,
    pub status: String,
    pub tick_size: Decimal,
    pub min_price: Decimal,
    pub max_price: Decimal,
    pub step_size: Decimal,
    pub min_qty: Decimal,
    pub max_qty: Decimal,
    pub min_notional: Decimal,
}

#[derive(Debug, Clone)]
pub struct Book {
    pub bid: Decimal,
    pub ask: Decimal,
    pub received_at: Instant,
}

#[derive(Debug, Clone)]
pub struct PositionSnapshot {
    pub quantity: Decimal,
    pub entry_price: Decimal,
    pub mark_price: Decimal,
    pub unrealized_pnl: Decimal,
}

impl PositionSnapshot {
    pub fn flat(mark_price: Decimal) -> Self {
        Self {
            quantity: Decimal::ZERO,
            entry_price: Decimal::ZERO,
            mark_price,
            unrealized_pnl: Decimal::ZERO,
        }
    }
}

#[derive(Debug, Clone)]
pub struct OpenOrder {
    pub order_id: u64,
    pub client_order_id: String,
    pub side: OrderSide,
    pub price: Decimal,
    pub original_qty: Decimal,
    pub executed_qty: Decimal,
    pub status: String,
}

#[derive(Debug, Clone)]
pub struct PlacedOrder {
    pub order_id: u64,
    pub client_order_id: String,
    pub status: String,
}

#[derive(Debug, Clone)]
pub struct TradeFill {
    pub id: u64,
    pub order_id: u64,
    pub quote_qty: Decimal,
    pub commission: Decimal,
    pub realized_pnl: Decimal,
    pub maker: bool,
}

#[derive(Clone)]
pub struct AsterClient {
    inner: Arc<ClientInner>,
}

struct ClientInner {
    http: reqwest::Client,
    rest_base_url: String,
    chain_id: u64,
    credentials: Option<Credentials>,
    wallet: Option<LocalWallet>,
    clock_offset_micros: AtomicI64,
    last_nonce: AtomicU64,
}

impl AsterClient {
    pub fn new(
        rest_base_url: String,
        chain_id: u64,
        credentials: Option<Credentials>,
    ) -> Result<Self, AsterError> {
        let wallet = match credentials.as_ref() {
            Some(credentials) => {
                let key = if credentials.signer_private_key.starts_with("0x") {
                    credentials.signer_private_key.clone()
                } else {
                    format!("0x{}", credentials.signer_private_key)
                };
                let wallet = LocalWallet::from_str(&key)
                    .map_err(|error| AsterError::SignerInit(error.to_string()))?
                    .with_chain_id(chain_id);

                let derived_address = format!("{:#x}", wallet.address());
                if !derived_address.eq_ignore_ascii_case(&credentials.signer_address) {
                    return Err(AsterError::SignerInit(format!(
                        "ASTER_SIGNER_PRIVATE_KEY derives {}, but ASTER_SIGNER_ADDRESS is {}",
                        derived_address, credentials.signer_address
                    )));
                }
                Some(wallet)
            }
            None => None,
        };

        let http = reqwest::Client::builder()
            .user_agent("aster-rust-market-maker/0.1.0")
            .timeout(Duration::from_secs(15))
            .build()?;

        Ok(Self {
            inner: Arc::new(ClientInner {
                http,
                rest_base_url,
                chain_id,
                credentials,
                wallet,
                clock_offset_micros: AtomicI64::new(0),
                last_nonce: AtomicU64::new(0),
            }),
        })
    }

    pub fn has_credentials(&self) -> bool {
        self.inner.credentials.is_some()
    }

    pub async fn sync_server_time(&self) -> Result<i64, AsterError> {
        let before = unix_millis_i64();
        let response: TimeResponse = self.unsigned_get("/fapi/v3/time", BTreeMap::new()).await?;
        let after = unix_millis_i64();
        let midpoint = before + ((after - before) / 2);
        let offset_ms = response.server_time - midpoint;
        self.inner
            .clock_offset_micros
            .store(offset_ms.saturating_mul(1_000), Ordering::SeqCst);
        info!(offset_ms, "synchronized Aster server clock");
        Ok(offset_ms)
    }

    pub async fn fetch_symbol_rules(&self, symbol: &str) -> Result<SymbolRules, AsterError> {
        let value: Value = self
            .unsigned_get("/fapi/v3/exchangeInfo", BTreeMap::new())
            .await?;
        parse_symbol_rules(&value, symbol)
    }

    pub async fn list_symbols(&self, filter: Option<&str>) -> Result<Vec<String>, AsterError> {
        let value: Value = self
            .unsigned_get("/fapi/v3/exchangeInfo", BTreeMap::new())
            .await?;
        let symbols = value
            .get("symbols")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                AsterError::InvalidResponse("exchangeInfo response is missing symbols".to_owned())
            })?;

        let needle = filter.map(str::to_ascii_uppercase);
        let mut result = symbols
            .iter()
            .filter_map(|item| {
                let symbol = item.get("symbol")?.as_str()?;
                let status = item.get("status").and_then(Value::as_str).unwrap_or("");
                if status != "TRADING" {
                    return None;
                }
                if let Some(needle) = needle.as_ref() {
                    if !symbol.to_ascii_uppercase().contains(needle) {
                        return None;
                    }
                }
                Some(symbol.to_owned())
            })
            .collect::<Vec<_>>();
        result.sort_unstable();
        Ok(result)
    }

    pub async fn book_ticker(&self, symbol: &str) -> Result<Book, AsterError> {
        let mut params = BTreeMap::new();
        params.insert("symbol".to_owned(), symbol.to_owned());
        let response: RestBookTicker = self
            .unsigned_get("/fapi/v3/ticker/bookTicker", params)
            .await?;
        let bid = parse_decimal("bidPrice", &response.bid_price)?;
        let ask = parse_decimal("askPrice", &response.ask_price)?;
        validate_book(bid, ask)?;
        Ok(Book {
            bid,
            ask,
            received_at: Instant::now(),
        })
    }

    pub async fn is_hedge_mode(&self) -> Result<bool, AsterError> {
        let response: PositionMode = self
            .signed_request(Method::GET, "/fapi/v3/positionSide/dual", BTreeMap::new())
            .await?;
        Ok(response.dual_side_position)
    }

    pub async fn position(
        &self,
        symbol: &str,
        fallback_mark_price: Decimal,
    ) -> Result<PositionSnapshot, AsterError> {
        let mut params = BTreeMap::new();
        params.insert("symbol".to_owned(), symbol.to_owned());
        let positions: Vec<RawPosition> = self
            .signed_request(Method::GET, "/fapi/v3/positionRisk", params)
            .await?;

        let Some(position) = positions
            .into_iter()
            .find(|position| position.symbol == symbol)
        else {
            return Ok(PositionSnapshot::flat(fallback_mark_price));
        };

        let quantity = parse_decimal("positionAmt", &position.position_amt)?;
        let entry_price = parse_decimal("entryPrice", &position.entry_price)?;
        let mark_price =
            parse_decimal("markPrice", &position.mark_price).unwrap_or(fallback_mark_price);
        let unrealized_pnl = parse_decimal("unRealizedProfit", &position.unrealized_profit)?;

        Ok(PositionSnapshot {
            quantity,
            entry_price,
            mark_price,
            unrealized_pnl,
        })
    }

    pub async fn open_orders(&self, symbol: &str) -> Result<Vec<OpenOrder>, AsterError> {
        let mut params = BTreeMap::new();
        params.insert("symbol".to_owned(), symbol.to_owned());
        let orders: Vec<RawOpenOrder> = self
            .signed_request(Method::GET, "/fapi/v3/openOrders", params)
            .await?;
        orders.into_iter().map(TryInto::try_into).collect()
    }

    pub async fn query_order(
        &self,
        symbol: &str,
        client_order_id: &str,
    ) -> Result<OpenOrder, AsterError> {
        let mut params = BTreeMap::new();
        params.insert("origClientOrderId".to_owned(), client_order_id.to_owned());
        params.insert("symbol".to_owned(), symbol.to_owned());
        let order: RawOpenOrder = self
            .signed_request(Method::GET, "/fapi/v3/order", params)
            .await?;
        order.try_into()
    }

    pub async fn place_limit(
        &self,
        symbol: &str,
        side: OrderSide,
        quantity: Decimal,
        price: Decimal,
        client_order_id: &str,
    ) -> Result<PlacedOrder, AsterError> {
        let mut params = BTreeMap::new();
        params.insert("newClientOrderId".to_owned(), client_order_id.to_owned());
        params.insert("newOrderRespType".to_owned(), "ACK".to_owned());
        params.insert("price".to_owned(), decimal_to_api(price));
        params.insert("quantity".to_owned(), decimal_to_api(quantity));
        params.insert("side".to_owned(), side.as_str().to_owned());
        params.insert("stpMode".to_owned(), "EXPIRE_BOTH".to_owned());
        params.insert("symbol".to_owned(), symbol.to_owned());
        params.insert("timeInForce".to_owned(), "GTX".to_owned());
        params.insert("type".to_owned(), "LIMIT".to_owned());

        let response: RawPlacedOrder = self
            .signed_request(Method::POST, "/fapi/v3/order", params)
            .await?;
        Ok(PlacedOrder {
            order_id: response.order_id,
            client_order_id: response.client_order_id,
            status: response.status,
        })
    }

    pub async fn cancel_order(
        &self,
        symbol: &str,
        client_order_id: &str,
    ) -> Result<(), AsterError> {
        let mut params = BTreeMap::new();
        params.insert("origClientOrderId".to_owned(), client_order_id.to_owned());
        params.insert("symbol".to_owned(), symbol.to_owned());
        let _: Value = self
            .signed_request(Method::DELETE, "/fapi/v3/order", params)
            .await?;
        Ok(())
    }

    pub async fn countdown_cancel_all(
        &self,
        symbol: &str,
        countdown_ms: u64,
    ) -> Result<(), AsterError> {
        let mut params = BTreeMap::new();
        params.insert("countdownTime".to_owned(), countdown_ms.to_string());
        params.insert("symbol".to_owned(), symbol.to_owned());
        let _: Value = self
            .signed_request(Method::POST, "/fapi/v3/countdownCancelAll", params)
            .await?;
        Ok(())
    }

    pub async fn user_trades(
        &self,
        symbol: &str,
        start_time_ms: Option<u64>,
        from_id: Option<u64>,
        limit: u64,
    ) -> Result<Vec<TradeFill>, AsterError> {
        let mut params = BTreeMap::new();
        if let Some(from_id) = from_id {
            params.insert("fromId".to_owned(), from_id.to_string());
        } else if let Some(start_time_ms) = start_time_ms {
            params.insert("startTime".to_owned(), start_time_ms.to_string());
        }
        params.insert("limit".to_owned(), limit.to_string());
        params.insert("symbol".to_owned(), symbol.to_owned());

        let trades: Vec<RawTradeFill> = self
            .signed_request(Method::GET, "/fapi/v3/userTrades", params)
            .await?;
        trades.into_iter().map(TryInto::try_into).collect()
    }

    async fn unsigned_get<T>(
        &self,
        path: &str,
        params: BTreeMap<String, String>,
    ) -> Result<T, AsterError>
    where
        T: DeserializeOwned,
    {
        let query = encode_form(&params)?;
        let mut url = format!("{}{}", self.inner.rest_base_url, path);
        if !query.is_empty() {
            url.push('?');
            url.push_str(&query);
        }

        let response = self.inner.http.get(url).send().await?;
        decode_response(response, false).await
    }

    async fn signed_request<T>(
        &self,
        method: Method,
        path: &str,
        mut params: BTreeMap<String, String>,
    ) -> Result<T, AsterError>
    where
        T: DeserializeOwned,
    {
        let credentials = self
            .inner
            .credentials
            .as_ref()
            .ok_or(AsterError::MissingCredentials)?;
        let wallet = self
            .inner
            .wallet
            .as_ref()
            .ok_or(AsterError::MissingCredentials)?;

        params.insert("nonce".to_owned(), self.next_nonce().to_string());
        params.insert("signer".to_owned(), credentials.signer_address.clone());
        params.insert("user".to_owned(), credentials.user_address.clone());

        let sign_payload = encode_form(&params)?;
        let signature = sign_payload_eip712(wallet, self.inner.chain_id, &sign_payload).await?;
        params.insert("signature".to_owned(), signature);
        let final_payload = encode_form(&params)?;

        let url = format!("{}{}", self.inner.rest_base_url, path);
        let execution_may_be_unknown = method != Method::GET;
        debug!(%method, %path, "sending signed Aster request");

        let request = if method == Method::GET {
            self.inner
                .http
                .request(method, format!("{url}?{final_payload}"))
        } else {
            self.inner
                .http
                .request(method, url)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(final_payload)
        };

        let response = request.send().await?;
        decode_response(response, execution_may_be_unknown).await
    }

    fn next_nonce(&self) -> u64 {
        let now =
            unix_micros_i128() + i128::from(self.inner.clock_offset_micros.load(Ordering::SeqCst));
        let adjusted_now = now.max(0).min(i128::from(u64::MAX)) as u64;

        loop {
            let previous = self.inner.last_nonce.load(Ordering::SeqCst);
            let candidate = adjusted_now.max(previous.saturating_add(1));
            if self
                .inner
                .last_nonce
                .compare_exchange(previous, candidate, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                return candidate;
            }
        }
    }
}

pub async fn run_book_ticker_stream(
    ws_base_url: String,
    symbol: String,
    tx: watch::Sender<Option<Book>>,
) {
    let stream_name = format!("{}@bookTicker", symbol.to_ascii_lowercase());
    let url = format!("{}/ws/{}", ws_base_url.trim_end_matches('/'), stream_name);
    let mut backoff_secs = 1_u64;

    loop {
        info!(%url, "connecting to Aster book ticker stream");
        match connect_async(url.as_str()).await {
            Ok((mut stream, _)) => {
                backoff_secs = 1;
                info!(%symbol, "Aster book ticker stream connected");

                while let Some(message) = stream.next().await {
                    match message {
                        Ok(Message::Text(text)) => {
                            match serde_json::from_str::<WsBookTicker>(text.as_ref()) {
                                Ok(update) => {
                                    let parsed = (|| -> Result<Book, AsterError> {
                                        let bid = parse_decimal("b", &update.bid_price)?;
                                        let ask = parse_decimal("a", &update.ask_price)?;
                                        validate_book(bid, ask)?;
                                        Ok(Book {
                                            bid,
                                            ask,
                                            received_at: Instant::now(),
                                        })
                                    })();

                                    match parsed {
                                        Ok(book) => {
                                            if tx.send(Some(book)).is_err() {
                                                return;
                                            }
                                        }
                                        Err(error) => {
                                            warn!(%error, "discarding invalid book ticker update");
                                        }
                                    }
                                }
                                Err(error) => {
                                    warn!(%error, "failed to decode book ticker update");
                                }
                            }
                        }
                        Ok(Message::Ping(payload)) => {
                            if let Err(error) = stream.send(Message::Pong(payload)).await {
                                warn!(%error, "failed to reply to websocket ping");
                                break;
                            }
                        }
                        Ok(Message::Close(frame)) => {
                            warn!(?frame, "Aster websocket closed");
                            break;
                        }
                        Ok(Message::Binary(_)) | Ok(Message::Pong(_)) | Ok(Message::Frame(_)) => {}
                        Err(error) => {
                            warn!(%error, "Aster websocket error");
                            break;
                        }
                    }
                }
            }
            Err(error) => {
                warn!(%error, "failed to connect to Aster websocket");
            }
        }

        if tx.is_closed() {
            return;
        }

        warn!(backoff_secs, "reconnecting Aster websocket after backoff");
        tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
        backoff_secs = (backoff_secs * 2).min(30);
    }
}

fn parse_symbol_rules(value: &Value, requested_symbol: &str) -> Result<SymbolRules, AsterError> {
    let symbols = value
        .get("symbols")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            AsterError::InvalidResponse("exchangeInfo response is missing symbols".to_owned())
        })?;

    let symbol = symbols
        .iter()
        .find(|item| {
            item.get("symbol")
                .and_then(Value::as_str)
                .map(|symbol| symbol == requested_symbol)
                .unwrap_or(false)
        })
        .ok_or_else(|| {
            AsterError::InvalidResponse(format!(
                "symbol {requested_symbol} is not present in exchangeInfo"
            ))
        })?;

    let filters = symbol
        .get("filters")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            AsterError::InvalidResponse(format!("symbol {requested_symbol} has no filters"))
        })?;

    let price_filter = find_filter(filters, "PRICE_FILTER")?;
    let lot_filter = find_filter(filters, "LOT_SIZE")?;
    let min_notional_filter = filters.iter().find(|filter| {
        filter
            .get("filterType")
            .and_then(Value::as_str)
            .map(|value| value == "MIN_NOTIONAL")
            .unwrap_or(false)
    });

    let tick_size = decimal_from_value(price_filter, "tickSize")?;
    let min_price = decimal_from_value(price_filter, "minPrice")?;
    let max_price = decimal_from_value(price_filter, "maxPrice")?;
    let step_size = decimal_from_value(lot_filter, "stepSize")?;
    let min_qty = decimal_from_value(lot_filter, "minQty")?;
    let max_qty = decimal_from_value(lot_filter, "maxQty")?;
    let min_notional = min_notional_filter
        .and_then(|filter| filter.get("notional").or_else(|| filter.get("minNotional")))
        .and_then(Value::as_str)
        .map(|value| parse_decimal("MIN_NOTIONAL", value))
        .transpose()?
        .unwrap_or(Decimal::ZERO);

    if tick_size <= Decimal::ZERO || step_size <= Decimal::ZERO {
        return Err(AsterError::InvalidResponse(format!(
            "symbol {requested_symbol} has a non-positive tickSize or stepSize"
        )));
    }

    Ok(SymbolRules {
        symbol: requested_symbol.to_owned(),
        status: symbol
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        tick_size,
        min_price,
        max_price,
        step_size,
        min_qty,
        max_qty,
        min_notional,
    })
}

fn find_filter<'a>(filters: &'a [Value], filter_type: &str) -> Result<&'a Value, AsterError> {
    filters
        .iter()
        .find(|filter| {
            filter
                .get("filterType")
                .and_then(Value::as_str)
                .map(|value| value == filter_type)
                .unwrap_or(false)
        })
        .ok_or_else(|| {
            AsterError::InvalidResponse(format!("exchangeInfo is missing {filter_type}"))
        })
}

fn decimal_from_value(value: &Value, field: &str) -> Result<Decimal, AsterError> {
    let raw = value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| AsterError::InvalidResponse(format!("response is missing {field}")))?;
    parse_decimal(field, raw)
}

fn parse_decimal(field: &str, raw: &str) -> Result<Decimal, AsterError> {
    Decimal::from_str(raw).map_err(|error| {
        AsterError::InvalidResponse(format!("invalid decimal in {field}: {raw} ({error})"))
    })
}

fn validate_book(bid: Decimal, ask: Decimal) -> Result<(), AsterError> {
    if bid <= Decimal::ZERO || ask <= Decimal::ZERO || bid >= ask {
        return Err(AsterError::InvalidResponse(format!(
            "invalid book ticker bid={bid}, ask={ask}"
        )));
    }
    Ok(())
}

fn encode_form(params: &BTreeMap<String, String>) -> Result<String, AsterError> {
    serde_urlencoded::to_string(params).map_err(|error| {
        AsterError::InvalidResponse(format!("failed to encode request params: {error}"))
    })
}

async fn sign_payload_eip712(
    wallet: &LocalWallet,
    chain_id: u64,
    payload: &str,
) -> Result<String, AsterError> {
    let typed_data: TypedData = serde_json::from_value(json!({
        "types": {
            "EIP712Domain": [
                {"name": "name", "type": "string"},
                {"name": "version", "type": "string"},
                {"name": "chainId", "type": "uint256"},
                {"name": "verifyingContract", "type": "address"}
            ],
            "Message": [
                {"name": "msg", "type": "string"}
            ]
        },
        "primaryType": "Message",
        "domain": {
            "name": "AsterSignTransaction",
            "version": "1",
            "chainId": chain_id,
            "verifyingContract": "0x0000000000000000000000000000000000000000"
        },
        "message": {
            "msg": payload
        }
    }))?;

    let signature = wallet
        .sign_typed_data(&typed_data)
        .await
        .map_err(|error| AsterError::Signing(error.to_string()))?;
    Ok(signature.to_string())
}

async fn decode_response<T>(
    response: reqwest::Response,
    execution_may_be_unknown: bool,
) -> Result<T, AsterError>
where
    T: DeserializeOwned,
{
    let status = response.status();
    let body = response.text().await?;

    if status.as_u16() == 503 && execution_may_be_unknown {
        return Err(AsterError::ExecutionUnknown(body));
    }

    let parsed_value = if body.trim().is_empty() {
        Ok(Value::Null)
    } else {
        serde_json::from_str::<Value>(&body)
    };

    if let Ok(value) = &parsed_value {
        if let Some(error) = api_error_from_value(value) {
            return Err(error);
        }
    }

    if !status.is_success() {
        return Err(AsterError::Http {
            status: status.as_u16(),
            body,
        });
    }

    let value = parsed_value?;
    serde_json::from_value(value).map_err(AsterError::from)
}

fn api_error_from_value(value: &Value) -> Option<AsterError> {
    let code = api_code(value)?;
    if code >= 0 {
        return None;
    }
    let message = value
        .get("msg")
        .and_then(Value::as_str)
        .unwrap_or("unknown API error")
        .to_owned();
    Some(AsterError::Api { code, message })
}

fn api_code(value: &Value) -> Option<i64> {
    match value.get("code") {
        Some(Value::Number(number)) => number.as_i64(),
        Some(Value::String(raw)) => raw.parse().ok(),
        _ => None,
    }
}

fn decimal_to_api(value: Decimal) -> String {
    value.normalize().to_string()
}

fn unix_millis_i64() -> i64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    millis.min(i64::MAX as u128) as i64
}

fn unix_micros_i128() -> i128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as i128
}

#[derive(Debug, Deserialize)]
struct TimeResponse {
    #[serde(rename = "serverTime")]
    server_time: i64,
}

#[derive(Debug, Deserialize)]
struct RestBookTicker {
    #[serde(rename = "bidPrice")]
    bid_price: String,
    #[serde(rename = "askPrice")]
    ask_price: String,
}

#[derive(Debug, Deserialize)]
struct WsBookTicker {
    #[serde(rename = "b")]
    bid_price: String,
    #[serde(rename = "a")]
    ask_price: String,
}

#[derive(Debug, Deserialize)]
struct PositionMode {
    #[serde(rename = "dualSidePosition")]
    dual_side_position: bool,
}

#[derive(Debug, Deserialize)]
struct RawPosition {
    symbol: String,
    #[serde(rename = "positionAmt")]
    position_amt: String,
    #[serde(rename = "entryPrice")]
    entry_price: String,
    #[serde(rename = "markPrice")]
    mark_price: String,
    #[serde(
        rename = "unRealizedProfit",
        alias = "unrealizedProfit",
        alias = "unRealizedPnl"
    )]
    unrealized_profit: String,
}

#[derive(Debug, Deserialize)]
struct RawOpenOrder {
    #[serde(rename = "orderId")]
    order_id: u64,
    #[serde(rename = "clientOrderId")]
    client_order_id: String,
    side: String,
    price: String,
    #[serde(rename = "origQty")]
    original_qty: String,
    #[serde(rename = "executedQty")]
    executed_qty: String,
    status: String,
}

impl TryFrom<RawOpenOrder> for OpenOrder {
    type Error = AsterError;

    fn try_from(value: RawOpenOrder) -> Result<Self, Self::Error> {
        let side = match value.side.as_str() {
            "BUY" => OrderSide::Buy,
            "SELL" => OrderSide::Sell,
            other => {
                return Err(AsterError::InvalidResponse(format!(
                    "unknown order side {other}"
                )))
            }
        };

        Ok(Self {
            order_id: value.order_id,
            client_order_id: value.client_order_id,
            side,
            price: parse_decimal("price", &value.price)?,
            original_qty: parse_decimal("origQty", &value.original_qty)?,
            executed_qty: parse_decimal("executedQty", &value.executed_qty)?,
            status: value.status,
        })
    }
}

#[derive(Debug, Deserialize)]
struct RawPlacedOrder {
    #[serde(rename = "orderId")]
    order_id: u64,
    #[serde(rename = "clientOrderId")]
    client_order_id: String,
    #[serde(default = "default_new_order_status")]
    status: String,
}

fn default_new_order_status() -> String {
    "NEW".to_owned()
}

#[derive(Debug, Deserialize)]
struct RawTradeFill {
    id: u64,
    #[serde(rename = "orderId")]
    order_id: u64,
    #[serde(rename = "quoteQty")]
    quote_qty: String,
    commission: String,
    #[serde(rename = "realizedPnl")]
    realized_pnl: String,
    maker: bool,
}

impl TryFrom<RawTradeFill> for TradeFill {
    type Error = AsterError;

    fn try_from(value: RawTradeFill) -> Result<Self, Self::Error> {
        Ok(Self {
            id: value.id,
            order_id: value.order_id,
            quote_qty: parse_decimal("quoteQty", &value.quote_qty)?,
            commission: parse_decimal("commission", &value.commission)?,
            realized_pnl: parse_decimal("realizedPnl", &value.realized_pnl)?,
            maker: value.maker,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::{api_error_from_value, encode_form, AsterError};

    #[test]
    fn form_payload_is_ascii_key_sorted() {
        let mut params = BTreeMap::new();
        params.insert("symbol".to_owned(), "BTCUSDT".to_owned());
        params.insert("nonce".to_owned(), "2".to_owned());
        params.insert("price".to_owned(), "1.25".to_owned());

        assert_eq!(
            encode_form(&params).expect("encode"),
            "nonce=2&price=1.25&symbol=BTCUSDT"
        );
    }

    #[test]
    fn recognizes_api_errors_independent_of_http_status() {
        let error = api_error_from_value(&json!({"code": -2013, "msg": "Order does not exist."}))
            .expect("API error");
        assert!(matches!(error, AsterError::Api { code: -2013, .. }));
        assert!(api_error_from_value(&json!({"code": 200, "msg": "success"})).is_none());
    }

    #[test]
    fn classifies_no_such_order_conservatively() {
        assert!(AsterError::Api {
            code: -2013,
            message: "Order does not exist.".to_owned(),
        }
        .is_no_such_order());
        assert!(AsterError::Api {
            code: -2011,
            message: "Unknown order sent.".to_owned(),
        }
        .is_no_such_order());
        assert!(!AsterError::Api {
            code: -2011,
            message: "Cancellation rejected by risk controls.".to_owned(),
        }
        .is_no_such_order());
    }
}
