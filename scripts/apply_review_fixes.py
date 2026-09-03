#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def read(path: str) -> str:
    return (ROOT / path).read_text(encoding="utf-8")


def write(path: str, content: str) -> None:
    (ROOT / path).write_text(content, encoding="utf-8")


def replace_once(path: str, old: str, new: str) -> None:
    content = read(path)
    count = content.count(old)
    if count != 1:
        raise RuntimeError(f"{path}: expected one occurrence, found {count}: {old[:80]!r}")
    write(path, content.replace(old, new, 1))


def replace_region(path: str, start: str, end: str, replacement: str) -> None:
    content = read(path)
    start_index = content.find(start)
    if start_index < 0:
        raise RuntimeError(f"{path}: start marker not found: {start!r}")
    end_index = content.find(end, start_index)
    if end_index < 0:
        raise RuntimeError(f"{path}: end marker not found: {end!r}")
    write(path, content[:start_index] + replacement + content[end_index:])


# Cargo: this is a trading process; do not force panic=abort in release builds.
replace_once("Cargo.toml", 'panic = "abort"\n', "")

# Configuration hardening.
replace_once(
    "src/config.rs",
    "use rust_decimal::Decimal;\n",
    "use reqwest::Url;\nuse rust_decimal::Decimal;\n",
)
replace_once(
    "src/config.rs",
    '''    if file.deadman_switch_enabled {
        if file.dry_run {
            bail!("deadman_switch_enabled has no effect in dry-run mode; disable it");
        }
        if file.deadman_countdown_ms < 10_000 {
            bail!("deadman_countdown_ms must be at least 10000");
        }
        if file.deadman_heartbeat_ms == 0
            || file.deadman_heartbeat_ms >= file.deadman_countdown_ms
        {
            bail!(
                "deadman_heartbeat_ms must be greater than zero and smaller than deadman_countdown_ms"
            );
        }
    }
''',
    '''    if file.deadman_switch_enabled {
        if file.dry_run {
            bail!("deadman_switch_enabled has no effect in dry-run mode; disable it");
        }
        if !file.cancel_on_exit {
            bail!("deadman_switch_enabled requires cancel_on_exit = true");
        }
        if file.deadman_countdown_ms < 10_000 {
            bail!("deadman_countdown_ms must be at least 10000");
        }
        if file.deadman_heartbeat_ms == 0
            || file.deadman_heartbeat_ms >= file.deadman_countdown_ms
        {
            bail!(
                "deadman_heartbeat_ms must be greater than zero and smaller than deadman_countdown_ms"
            );
        }
        if file.deadman_countdown_ms
            < file.deadman_heartbeat_ms.saturating_mul(2)
        {
            bail!(
                "deadman_countdown_ms must be at least twice deadman_heartbeat_ms"
            );
        }
    }
''',
)
replace_once(
    "src/config.rs",
    '''    let ws_base_url = file
        .ws_base_url
        .unwrap_or_else(|| file.environment.default_ws_url().to_owned())
        .trim_end_matches('/')
        .to_owned();

    Ok(RuntimeConfig {
''',
    '''    let ws_base_url = file
        .ws_base_url
        .unwrap_or_else(|| file.environment.default_ws_url().to_owned())
        .trim_end_matches('/')
        .to_owned();

    validate_base_url("rest_base_url", &rest_base_url, "https")?;
    validate_base_url("ws_base_url", &ws_base_url, "wss")?;

    Ok(RuntimeConfig {
''',
)
replace_once(
    "src/config.rs",
    '''fn parse_positive_decimal(name: &str, value: &str) -> Result<Decimal> {
''',
    '''fn validate_base_url(name: &str, raw: &str, expected_scheme: &str) -> Result<()> {
    let url = Url::parse(raw)
        .with_context(|| format!("{name} must be a valid URL"))?;
    if url.scheme() != expected_scheme {
        bail!("{name} must use the {expected_scheme} scheme");
    }
    if url.host_str().is_none() {
        bail!("{name} must include a host");
    }
    if !url.username().is_empty() || url.password().is_some() {
        bail!("{name} must not contain embedded credentials");
    }
    if url.query().is_some() || url.fragment().is_some() {
        bail!("{name} must not contain a query string or fragment");
    }
    if url.path() != "/" {
        bail!("{name} must not contain a path");
    }
    Ok(())
}

fn parse_positive_decimal(name: &str, value: &str) -> Result<Decimal> {
''',
)
replace_once(
    "src/config.rs",
    '''mod tests {
    use super::validate_client_order_prefix;
''',
    '''mod tests {
    use super::{validate_base_url, validate_client_order_prefix};
''',
)
replace_once(
    "src/config.rs",
    '''    fn validates_client_order_prefix() {
        assert!(validate_client_order_prefix("armm_").is_ok());
        assert!(validate_client_order_prefix("").is_err());
        assert!(validate_client_order_prefix("bad prefix").is_err());
        assert!(validate_client_order_prefix("1234567890123").is_err());
    }
''',
    '''    fn validates_client_order_prefix() {
        assert!(validate_client_order_prefix("armm_").is_ok());
        assert!(validate_client_order_prefix("").is_err());
        assert!(validate_client_order_prefix("bad prefix").is_err());
        assert!(validate_client_order_prefix("1234567890123").is_err());
    }

    #[test]
    fn validates_base_urls() {
        assert!(validate_base_url("rest", "https://fapi.asterdex.com", "https").is_ok());
        assert!(validate_base_url("ws", "wss://fstream.asterdex.com", "wss").is_ok());
        assert!(validate_base_url("rest", "http://localhost", "https").is_err());
        assert!(validate_base_url("rest", "https://example.com/path", "https").is_err());
        assert!(validate_base_url("ws", "wss://user:pass@example.com", "wss").is_err());
    }
''',
)

# API correctness: parse Aster JSON errors on 4xx, classify 503 only for mutating
# requests, tolerate minimal ACK responses, and expose order IDs in trade fills.
replace_once(
    "src/api.rs",
    '''pub struct TradeFill {
    pub id: u64,
''',
    '''pub struct TradeFill {
    pub id: u64,
    pub order_id: u64,
''',
)
replace_once(
    "src/api.rs",
    '''        let response = self.inner.http.get(url).send().await?;
        decode_response(response).await
''',
    '''        let response = self.inner.http.get(url).send().await?;
        decode_response(response, false).await
''',
)
replace_once(
    "src/api.rs",
    '''        let url = format!("{}{}", self.inner.rest_base_url, path);
        debug!(%method, %path, "sending signed Aster request");

        let request = if method == Method::GET {
''',
    '''        let url = format!("{}{}", self.inner.rest_base_url, path);
        let execution_may_be_unknown = method != Method::GET;
        debug!(%method, %path, "sending signed Aster request");

        let request = if method == Method::GET {
''',
)
replace_once(
    "src/api.rs",
    '''        let response = request.send().await?;
        decode_response(response).await
''',
    '''        let response = request.send().await?;
        decode_response(response, execution_may_be_unknown).await
''',
)
replace_region(
    "src/api.rs",
    "async fn decode_response<T>(",
    "fn api_code(value: &Value) -> Option<i64> {",
    '''async fn decode_response<T>(
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

''',
)
replace_once(
    "src/api.rs",
    '''struct RawPlacedOrder {
    #[serde(rename = "orderId")]
    order_id: u64,
    #[serde(rename = "clientOrderId")]
    client_order_id: String,
    status: String,
}
''',
    '''struct RawPlacedOrder {
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
''',
)
replace_once(
    "src/api.rs",
    '''struct RawTradeFill {
    id: u64,
    #[serde(rename = "quoteQty")]
''',
    '''struct RawTradeFill {
    id: u64,
    #[serde(rename = "orderId")]
    order_id: u64,
    #[serde(rename = "quoteQty")]
''',
)
replace_once(
    "src/api.rs",
    '''        Ok(Self {
            id: value.id,
            quote_qty: parse_decimal("quoteQty", &value.quote_qty)?,
''',
    '''        Ok(Self {
            id: value.id,
            order_id: value.order_id,
            quote_qty: parse_decimal("quoteQty", &value.quote_qty)?,
''',
)
replace_once(
    "src/api.rs",
    '''mod tests {
    use std::collections::BTreeMap;

    use super::encode_form;
''',
    '''mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::{api_error_from_value, encode_form, AsterError};
''',
)
replace_once(
    "src/api.rs",
    '''    fn form_payload_is_ascii_key_sorted() {
        let mut params = BTreeMap::new();
        params.insert("symbol".to_owned(), "BTCUSDT".to_owned());
        params.insert("nonce".to_owned(), "2".to_owned());
        params.insert("price".to_owned(), "1.25".to_owned());

        assert_eq!(
            encode_form(&params).expect("encode"),
            "nonce=2&price=1.25&symbol=BTCUSDT"
        );
    }
''',
    '''    fn form_payload_is_ascii_key_sorted() {
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
''',
)

# Bot state/risk hardening.
replace_once(
    "src/bot.rs",
    '''use crate::{
''',
    '''const SERVER_TIME_RESYNC_SECS: u64 = 300;

use crate::{
''',
)
replace_once(
    "src/bot.rs",
    '''    seen_trade_ids: HashSet<u64>,
    last_trade_id: Option<u64>,
''',
    '''    managed_order_ids: HashSet<u64>,
    seen_trade_ids: HashSet<u64>,
    last_trade_id: Option<u64>,
''',
)
replace_once(
    "src/bot.rs",
    '''            realized_pnl: Decimal::ZERO,
            seen_trade_ids: HashSet::new(),
''',
    '''            realized_pnl: Decimal::ZERO,
            managed_order_ids: HashSet::new(),
            seen_trade_ids: HashSet::new(),
''',
)
replace_once(
    "src/bot.rs",
    '''        let now = Instant::now();
        let mut last_state_refresh = now
''',
    '''        let now = Instant::now();
        let mut last_clock_sync = now;
        let mut last_state_refresh = now
''',
)
replace_once(
    "src/bot.rs",
    '''                    self.stale_orders_cancelled = false;

                    let mid = (book.bid + book.ask) / Decimal::from(2_u32);
''',
    '''                    self.stale_orders_cancelled = false;

                    if !self.config.dry_run
                        && last_clock_sync.elapsed()
                            >= Duration::from_secs(SERVER_TIME_RESYNC_SECS)
                    {
                        self.client
                            .sync_server_time()
                            .await
                            .context("failed to resynchronize server time")?;
                        last_clock_sync = Instant::now();
                    }

                    let mid = (book.bid + book.ask) / Decimal::from(2_u32);
''',
)
replace_once(
    "src/bot.rs",
    '''        if self.config.startup_cancel_existing_bot_orders {
            self.cancel_all_orders_with_prefix().await?;
        }
''',
    '''        if self.config.startup_cancel_existing_bot_orders {
            self.cancel_all_orders_with_prefix().await?;
        } else {
            self.ensure_no_existing_bot_orders().await?;
        }
''',
)
replace_region(
    "src/bot.rs",
    "    async fn refresh_live_state(",
    "    fn enforce_risk_limits(&self, mark_price: Decimal) -> Result<()> {",
    '''    async fn refresh_live_state(&mut self, fallback_mark: Decimal) -> Result<()> {
        let open_orders = self
            .client
            .open_orders(&self.config.symbol)
            .await
            .context("failed to refresh open orders")?;

        self.position = self
            .client
            .position(&self.config.symbol, fallback_mark)
            .await
            .context("failed to refresh position")?;

        let position_may_have_changed =
            self.reconcile_open_orders(open_orders).await?;
        if position_may_have_changed {
            self.refresh_position_only(fallback_mark).await?;
        }
        Ok(())
    }

    async fn refresh_position_only(
        &mut self,
        fallback_mark: Decimal,
    ) -> Result<()> {
        self.position = self
            .client
            .position(&self.config.symbol, fallback_mark)
            .await
            .context("failed to refresh position after order-state change")?;
        Ok(())
    }

    async fn reconcile_open_orders(
        &mut self,
        open_orders: Vec<OpenOrder>,
    ) -> Result<bool> {
        let tracked_buy_id = self
            .buy_order
            .as_ref()
            .map(|order| order.client_order_id.clone());
        let tracked_sell_id = self
            .sell_order
            .as_ref()
            .map(|order| order.client_order_id.clone());

        let mut buy_seen = false;
        let mut sell_seen = false;
        let mut position_may_have_changed = false;
        let mut cancel_ids = Vec::new();

        for order in open_orders
            .into_iter()
            .filter(|order| {
                order
                    .client_order_id
                    .starts_with(&self.config.client_order_prefix)
            })
        {
            let is_tracked_buy = tracked_buy_id
                .as_ref()
                .map(|id| id == &order.client_order_id)
                .unwrap_or(false);
            let is_tracked_sell = tracked_sell_id
                .as_ref()
                .map(|id| id == &order.client_order_id)
                .unwrap_or(false);

            if is_tracked_buy {
                buy_seen = true;
                self.buy_order = Some(managed_from_open(order.clone()));
                if order.executed_qty > Decimal::ZERO {
                    warn!(
                        order_id = order.order_id,
                        client_order_id = %order.client_order_id,
                        executed_qty = %order.executed_qty,
                        "buy order was partially filled; cancelling remainder before replenishing"
                    );
                    position_may_have_changed = true;
                    cancel_ids.push(order.client_order_id);
                    self.buy_order = None;
                }
            } else if is_tracked_sell {
                sell_seen = true;
                self.sell_order = Some(managed_from_open(order.clone()));
                if order.executed_qty > Decimal::ZERO {
                    warn!(
                        order_id = order.order_id,
                        client_order_id = %order.client_order_id,
                        executed_qty = %order.executed_qty,
                        "sell order was partially filled; cancelling remainder before replenishing"
                    );
                    position_may_have_changed = true;
                    cancel_ids.push(order.client_order_id);
                    self.sell_order = None;
                }
            } else {
                warn!(
                    order_id = order.order_id,
                    client_order_id = %order.client_order_id,
                    "found an untracked bot-prefixed order; cancelling it"
                );
                position_may_have_changed = true;
                cancel_ids.push(order.client_order_id);
            }
        }

        if tracked_buy_id.is_some() && !buy_seen {
            self.buy_order = None;
            position_may_have_changed = true;
        }
        if tracked_sell_id.is_some() && !sell_seen {
            self.sell_order = None;
            position_may_have_changed = true;
        }

        for client_order_id in cancel_ids {
            self.cancel_by_client_id(&client_order_id).await?;
        }

        Ok(position_may_have_changed)
    }

''',
)
replace_region(
    "src/bot.rs",
    "    async fn quote_once(&mut self, book: &Book) -> Result<()> {",
    "    fn needs_requote(",
    '''    async fn quote_once(&mut self, book: &Book) -> Result<()> {
        self.stats.quote_cycles = self.stats.quote_cycles.saturating_add(1);

        let (buy_target, sell_target) = self.compute_targets(book)?;
        let buy_cancelled = self
            .cancel_side_if_needed(OrderSide::Buy, buy_target.as_ref())
            .await?;
        let sell_cancelled = self
            .cancel_side_if_needed(OrderSide::Sell, sell_target.as_ref())
            .await?;

        if !self.config.dry_run && (buy_cancelled || sell_cancelled) {
            let mid = (book.bid + book.ask) / Decimal::from(2_u32);
            self.refresh_position_only(mid).await?;
        }

        let (buy_target, sell_target) = self.compute_targets(book)?;
        self.place_missing_side(OrderSide::Buy, buy_target).await?;
        self.place_missing_side(OrderSide::Sell, sell_target).await?;

        Ok(())
    }

    async fn cancel_side_if_needed(
        &mut self,
        side: OrderSide,
        target: Option<&QuoteTarget>,
    ) -> Result<bool> {
        let should_cancel = match (self.current_order(side), target) {
            (Some(_), None) => true,
            (Some(existing), Some(target)) => {
                self.needs_requote(existing, target)
            }
            (None, _) => false,
        };

        if !should_cancel {
            return Ok(false);
        }

        let existing = self
            .take_current_order(side)
            .ok_or_else(|| anyhow!("managed order disappeared before cancellation"))?;
        self.cancel_existing(&existing).await?;
        Ok(true)
    }

    async fn place_missing_side(
        &mut self,
        side: OrderSide,
        target: Option<QuoteTarget>,
    ) -> Result<()> {
        if self.current_order(side).is_some() {
            return Ok(());
        }
        if let Some(target) = target {
            self.place_target(side, target).await?;
        }
        Ok(())
    }

''',
)
replace_once(
    "src/bot.rs",
    '''            Ok(placed) => {
                info!(
''',
    '''            Ok(placed) => {
                self.stats.managed_order_ids.insert(placed.order_id);
                info!(
''',
)
replace_once(
    "src/bot.rs",
    '''                Ok(order) => {
                    info!(
                        attempt,
                        order_id = order.order_id,
                        status = %order.status,
                        "recovered order after unknown execution response"
                    );
''',
    '''                Ok(order) => {
                    self.stats.managed_order_ids.insert(order.order_id);
                    info!(
                        attempt,
                        order_id = order.order_id,
                        status = %order.status,
                        "recovered order after unknown execution response"
                    );
''',
)
replace_region(
    "src/bot.rs",
    "    async fn cancel_by_client_id(",
    "    async fn cancel_managed_orders(&mut self) -> Result<()> {",
    '''    async fn cancel_by_client_id(
        &mut self,
        client_order_id: &str,
    ) -> Result<()> {
        match self
            .client
            .cancel_order(&self.config.symbol, client_order_id)
            .await
        {
            Ok(()) => {
                self.stats.orders_cancelled =
                    self.stats.orders_cancelled.saturating_add(1);
                info!(%client_order_id, "cancelled order");
                Ok(())
            }
            Err(error) if error.is_no_such_order() => {
                info!(
                    %client_order_id,
                    "order was already absent while cancelling"
                );
                Ok(())
            }
            Err(error) if error.is_execution_unknown() => {
                self.recover_unknown_cancel(client_order_id, error).await
            }
            Err(error) => {
                Err(error).context("failed to cancel existing order")
            }
        }
    }

    async fn recover_unknown_cancel(
        &mut self,
        client_order_id: &str,
        original_error: AsterError,
    ) -> Result<()> {
        warn!(
            %original_error,
            %client_order_id,
            "cancel execution state is unknown; verifying order state before retrying"
        );

        for attempt in 1..=5 {
            tokio::time::sleep(Duration::from_millis(500)).await;
            match self
                .client
                .query_order(&self.config.symbol, client_order_id)
                .await
            {
                Err(error) if error.is_no_such_order() => {
                    info!(
                        attempt,
                        %client_order_id,
                        "order is absent after unknown cancel response"
                    );
                    self.stats.orders_cancelled =
                        self.stats.orders_cancelled.saturating_add(1);
                    return Ok(());
                }
                Err(error) if error.is_rate_limited() => {
                    warn!(attempt, %error, "rate limited while verifying cancellation");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                Err(error) => {
                    return Err(error)
                        .context("failed while verifying unknown cancel state");
                }
                Ok(order)
                    if order.status != "NEW"
                        && order.status != "PARTIALLY_FILLED" =>
                {
                    info!(
                        attempt,
                        order_id = order.order_id,
                        status = %order.status,
                        "order is no longer open after unknown cancel response"
                    );
                    return Ok(());
                }
                Ok(order) => {
                    warn!(
                        attempt,
                        order_id = order.order_id,
                        status = %order.status,
                        "order remains open after unknown cancel response; retrying cancellation"
                    );
                    match self
                        .client
                        .cancel_order(&self.config.symbol, client_order_id)
                        .await
                    {
                        Ok(()) => {
                            self.stats.orders_cancelled =
                                self.stats.orders_cancelled.saturating_add(1);
                            return Ok(());
                        }
                        Err(error) if error.is_no_such_order() => {
                            self.stats.orders_cancelled =
                                self.stats.orders_cancelled.saturating_add(1);
                            return Ok(());
                        }
                        Err(error) if error.is_execution_unknown() => {
                            warn!(attempt, %error, "retry cancel response is still unknown");
                        }
                        Err(error) if error.is_rate_limited() => {
                            warn!(attempt, %error, "rate limited while retrying cancellation");
                            tokio::time::sleep(Duration::from_secs(1)).await;
                        }
                        Err(error) => {
                            return Err(error)
                                .context("failed while retrying unknown cancellation");
                        }
                    }
                }
            }
        }

        bail!(
            "could not verify cancellation of order {} after an unknown execution response; halting with the exchange dead-man switch left armed when configured",
            client_order_id
        )
    }

''',
)
replace_once(
    "src/bot.rs",
    '''    async fn cancel_all_orders_with_prefix(&mut self) -> Result<()> {
''',
    '''    async fn ensure_no_existing_bot_orders(&self) -> Result<()> {
        let orders = self
            .client
            .open_orders(&self.config.symbol)
            .await
            .context("failed to inspect existing bot orders")?;
        let count = orders
            .iter()
            .filter(|order| {
                order
                    .client_order_id
                    .starts_with(&self.config.client_order_prefix)
            })
            .count();
        if count > 0 {
            bail!(
                "found {} existing order(s) with prefix {}; enable startup_cancel_existing_bot_orders or choose a fresh prefix",
                count,
                self.config.client_order_prefix
            );
        }
        Ok(())
    }

    async fn cancel_all_orders_with_prefix(&mut self) -> Result<()> {
''',
)
replace_once(
    "src/bot.rs",
    '''                if !self.stats.seen_trade_ids.insert(fill.id) {
                    continue;
                }

                self.stats.executed_notional += fill.quote_qty;
''',
    '''                if !self.stats.managed_order_ids.contains(&fill.order_id) {
                    continue;
                }
                if !self.stats.seen_trade_ids.insert(fill.id) {
                    continue;
                }

                self.stats.executed_notional += fill.quote_qty;
''',
)
replace_region(
    "src/bot.rs",
    "    async fn shutdown(&mut self) -> Result<()> {",
    "    fn next_client_order_id(&mut self, side: OrderSide) -> String {",
    '''    async fn shutdown(&mut self) -> Result<()> {
        let mut errors = Vec::new();
        let mut bot_orders_cleared = !self.config.cancel_on_exit;

        if self.config.cancel_on_exit {
            match self.cancel_all_orders_with_prefix().await {
                Ok(()) => bot_orders_cleared = true,
                Err(error) => {
                    errors.push(format!("failed to cancel bot orders: {error:#}"));
                }
            }
        }

        if self.config.deadman_switch_enabled && !self.config.dry_run {
            if bot_orders_cleared {
                if let Err(error) = self
                    .client
                    .countdown_cancel_all(&self.config.symbol, 0)
                    .await
                {
                    errors.push(format!(
                        "failed to disarm dead-man switch: {error}"
                    ));
                }
            } else {
                warn!(
                    "bot-order cleanup failed; leaving the exchange dead-man switch armed"
                );
            }
        }

        self.log_stats(self.position.mark_price);

        if errors.is_empty() {
            Ok(())
        } else {
            bail!("{}", errors.join("; "))
        }
    }

''',
)

# CI: formatting, linting, locked dependency resolution, and MSRV verification.
write(
    ".github/workflows/ci.yml",
    '''name: Rust CI

on:
  push:
    branches:
      - main
      - "feat/**"
  pull_request:

permissions:
  contents: read

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@1.82.0
        with:
          components: rustfmt, clippy
      - name: Cargo fmt
        run: cargo fmt --all -- --check
      - name: Cargo check
        run: cargo check --locked --all-targets
      - name: Cargo clippy
        run: cargo clippy --locked --all-targets -- -D warnings
      - name: Cargo test
        run: cargo test --locked --all-targets
''',
)

# Documentation corrections and reviewed behavior.
replace_once(
    "README.md",
    "git checkout feat/rust-market-maker\n\n",
    "",
)
replace_once(
    "README.md",
    '''- explicit handling of HTTP 503 as an **unknown execution state**
- session volume, maker/taker, commission and realized-PnL statistics
''',
    '''- decoding of structured Aster API errors even when returned with HTTP 4xx
- explicit recovery for HTTP 503 **unknown execution state** on placement and cancellation
- session volume, maker/taker, commission and realized-PnL statistics restricted to orders placed by this process
''',
)
replace_once(
    "README.md",
    '''# Cancel bot-prefixed orders left by an earlier process before starting.
startup_cancel_existing_bot_orders = true
''',
    '''# Cancel bot-prefixed orders left by an earlier process before starting.
# When false, startup fails instead of silently adopting or cancelling matching orders.
startup_cancel_existing_bot_orders = true
''',
)
replace_once(
    "README.md",
    '''For order placement, this bot:

1. does not blindly retry;
2. queries the unique `clientOrderId` repeatedly;
3. adopts the recovered order when found;
4. halts when it cannot determine the result.

This behavior prefers an interruption over accidentally creating duplicate exposure.
''',
    '''For order placement, this bot:

1. does not blindly retry;
2. queries the unique `clientOrderId` repeatedly;
3. adopts the recovered order when found;
4. halts when it cannot determine the result.

For cancellation, it queries the order first and retries only when the order is still open. If shutdown cleanup cannot be confirmed, an enabled exchange dead-man switch is deliberately left armed.

This behavior prefers an interruption over accidentally creating duplicate exposure or leaving an order live without protection.
''',
)
replace_once(
    "README.md",
    '''- position and open-order reconciliation currently use REST polling
''',
    '''- position and open-order reconciliation currently use REST polling; position is refreshed after cancellation before replacement
''',
)

# Complete the MIT package metadata with an actual license file.
write(
    "LICENSE",
    '''MIT License

Copyright (c) 2026 lly835

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
''',
)

print("review fixes applied")
