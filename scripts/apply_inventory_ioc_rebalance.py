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
        raise RuntimeError(
            f"{path}: expected exactly one match, found {count}: {old[:120]!r}"
        )
    write(path, content.replace(old, new, 1))


def insert_before(path: str, marker: str, content_to_insert: str) -> None:
    content = read(path)
    count = content.count(marker)
    if count != 1:
        raise RuntimeError(
            f"{path}: expected exactly one marker, found {count}: {marker[:120]!r}"
        )
    write(path, content.replace(marker, content_to_insert + marker, 1))


# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------
replace_once(
    "src/config.rs",
    '''    max_unrealized_loss_usd: String,

    refresh_ms: u64,
''',
    '''    max_unrealized_loss_usd: String,

    taker_rebalance_enabled: bool,
    taker_rebalance_trigger_notional_usd: String,
    taker_rebalance_target_notional_usd: String,
    taker_rebalance_max_order_notional_usd: String,
    taker_rebalance_max_position_age_secs: u64,
    taker_rebalance_cooldown_secs: u64,
    taker_rebalance_max_slippage_bps: u64,

    refresh_ms: u64,
''',
)
replace_once(
    "src/config.rs",
    '''            max_unrealized_loss_usd: "5".to_owned(),

            refresh_ms: 1_000,
''',
    '''            max_unrealized_loss_usd: "5".to_owned(),

            taker_rebalance_enabled: true,
            taker_rebalance_trigger_notional_usd: "30".to_owned(),
            taker_rebalance_target_notional_usd: "5".to_owned(),
            taker_rebalance_max_order_notional_usd: "20".to_owned(),
            taker_rebalance_max_position_age_secs: 180,
            taker_rebalance_cooldown_secs: 30,
            taker_rebalance_max_slippage_bps: 5,

            refresh_ms: 1_000,
''',
)
replace_once(
    "src/config.rs",
    '''    pub max_unrealized_loss_usd: Decimal,

    pub refresh_ms: u64,
''',
    '''    pub max_unrealized_loss_usd: Decimal,

    pub taker_rebalance_enabled: bool,
    pub taker_rebalance_trigger_notional_usd: Decimal,
    pub taker_rebalance_target_notional_usd: Decimal,
    pub taker_rebalance_max_order_notional_usd: Decimal,
    pub taker_rebalance_max_position_age_secs: u64,
    pub taker_rebalance_cooldown_secs: u64,
    pub taker_rebalance_max_slippage_bps: u64,

    pub refresh_ms: u64,
''',
)
replace_once(
    "src/config.rs",
    '''    let max_unrealized_loss_usd =
        parse_positive_decimal("max_unrealized_loss_usd", &file.max_unrealized_loss_usd)?;

    if file.symbol.trim().is_empty() {
''',
    '''    let max_unrealized_loss_usd =
        parse_positive_decimal("max_unrealized_loss_usd", &file.max_unrealized_loss_usd)?;
    let taker_rebalance_trigger_notional_usd = parse_positive_decimal(
        "taker_rebalance_trigger_notional_usd",
        &file.taker_rebalance_trigger_notional_usd,
    )?;
    let taker_rebalance_target_notional_usd = parse_nonnegative_decimal(
        "taker_rebalance_target_notional_usd",
        &file.taker_rebalance_target_notional_usd,
    )?;
    let taker_rebalance_max_order_notional_usd = parse_positive_decimal(
        "taker_rebalance_max_order_notional_usd",
        &file.taker_rebalance_max_order_notional_usd,
    )?;

    if file.symbol.trim().is_empty() {
''',
)
replace_once(
    "src/config.rs",
    '''    if max_position_notional_usd < quote_notional_usd {
        bail!("max_position_notional_usd must be at least quote_notional_usd");
    }

    validate_client_order_prefix(&file.client_order_prefix)?;
''',
    '''    if max_position_notional_usd < quote_notional_usd {
        bail!("max_position_notional_usd must be at least quote_notional_usd");
    }
    if file.taker_rebalance_enabled {
        if taker_rebalance_target_notional_usd
            >= taker_rebalance_trigger_notional_usd
        {
            bail!(
                "taker_rebalance_target_notional_usd must be smaller than taker_rebalance_trigger_notional_usd"
            );
        }
        if taker_rebalance_trigger_notional_usd > max_position_notional_usd {
            bail!(
                "taker_rebalance_trigger_notional_usd must not exceed max_position_notional_usd"
            );
        }
        if taker_rebalance_max_order_notional_usd > max_position_notional_usd {
            bail!(
                "taker_rebalance_max_order_notional_usd must not exceed max_position_notional_usd"
            );
        }
        if file.taker_rebalance_max_position_age_secs == 0 {
            bail!("taker_rebalance_max_position_age_secs must be greater than zero");
        }
        if file.taker_rebalance_cooldown_secs == 0 {
            bail!("taker_rebalance_cooldown_secs must be greater than zero");
        }
        if file.taker_rebalance_max_slippage_bps > 100 {
            bail!("taker_rebalance_max_slippage_bps must not exceed 100 bps");
        }
    }

    validate_client_order_prefix(&file.client_order_prefix)?;
''',
)
replace_once(
    "src/config.rs",
    '''        max_unrealized_loss_usd,

        refresh_ms: file.refresh_ms,
''',
    '''        max_unrealized_loss_usd,

        taker_rebalance_enabled: file.taker_rebalance_enabled,
        taker_rebalance_trigger_notional_usd,
        taker_rebalance_target_notional_usd,
        taker_rebalance_max_order_notional_usd,
        taker_rebalance_max_position_age_secs: file.taker_rebalance_max_position_age_secs,
        taker_rebalance_cooldown_secs: file.taker_rebalance_cooldown_secs,
        taker_rebalance_max_slippage_bps: file.taker_rebalance_max_slippage_bps,

        refresh_ms: file.refresh_ms,
''',
)
insert_before(
    "src/config.rs",
    '''fn validate_client_order_prefix(prefix: &str) -> Result<()> {
''',
    '''fn parse_nonnegative_decimal(name: &str, value: &str) -> Result<Decimal> {
    let parsed =
        Decimal::from_str(value).with_context(|| format!("{name} must be a decimal string"))?;
    if parsed < Decimal::ZERO {
        bail!("{name} must not be negative");
    }
    Ok(parsed)
}

''',
)

# ---------------------------------------------------------------------------
# API: add an explicitly reduce-only LIMIT+IOC path.
# ---------------------------------------------------------------------------
replace_once(
    "src/api.rs",
    '''pub struct PlacedOrder {
    pub order_id: u64,
    pub client_order_id: String,
    pub status: String,
}
''',
    '''pub struct PlacedOrder {
    pub order_id: u64,
    pub client_order_id: String,
    pub status: String,
    pub executed_qty: Decimal,
    pub avg_price: Decimal,
}
''',
)
replace_once(
    "src/api.rs",
    '''        Ok(PlacedOrder {
            order_id: response.order_id,
            client_order_id: response.client_order_id,
            status: response.status,
        })
    }

    pub async fn cancel_order(
''',
    '''        Ok(PlacedOrder {
            order_id: response.order_id,
            client_order_id: response.client_order_id,
            status: response.status,
            executed_qty: parse_decimal("executedQty", &response.executed_qty)?,
            avg_price: parse_decimal("avgPrice", &response.avg_price)?,
        })
    }

    pub async fn place_reduce_only_ioc_limit(
        &self,
        symbol: &str,
        side: OrderSide,
        quantity: Decimal,
        price: Decimal,
        client_order_id: &str,
    ) -> Result<PlacedOrder, AsterError> {
        let mut params = BTreeMap::new();
        params.insert("newClientOrderId".to_owned(), client_order_id.to_owned());
        params.insert("newOrderRespType".to_owned(), "RESULT".to_owned());
        params.insert("price".to_owned(), decimal_to_api(price));
        params.insert("quantity".to_owned(), decimal_to_api(quantity));
        params.insert("reduceOnly".to_owned(), "true".to_owned());
        params.insert("side".to_owned(), side.as_str().to_owned());
        params.insert("stpMode".to_owned(), "EXPIRE_BOTH".to_owned());
        params.insert("symbol".to_owned(), symbol.to_owned());
        params.insert("timeInForce".to_owned(), "IOC".to_owned());
        params.insert("type".to_owned(), "LIMIT".to_owned());

        let response: RawPlacedOrder = self
            .signed_request(Method::POST, "/fapi/v3/order", params)
            .await?;
        Ok(PlacedOrder {
            order_id: response.order_id,
            client_order_id: response.client_order_id,
            status: response.status,
            executed_qty: parse_decimal("executedQty", &response.executed_qty)?,
            avg_price: parse_decimal("avgPrice", &response.avg_price)?,
        })
    }

    pub async fn cancel_order(
''',
)
replace_once(
    "src/api.rs",
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
    '''struct RawPlacedOrder {
    #[serde(rename = "orderId")]
    order_id: u64,
    #[serde(rename = "clientOrderId")]
    client_order_id: String,
    #[serde(default = "default_new_order_status")]
    status: String,
    #[serde(rename = "executedQty", default = "default_zero_decimal_string")]
    executed_qty: String,
    #[serde(rename = "avgPrice", default = "default_zero_decimal_string")]
    avg_price: String,
}

fn default_new_order_status() -> String {
    "NEW".to_owned()
}

fn default_zero_decimal_string() -> String {
    "0".to_owned()
}
''',
)

# ---------------------------------------------------------------------------
# Bot state and maker-first, risk-driven IOC rebalancing.
# ---------------------------------------------------------------------------
insert_before(
    "src/bot.rs",
    '''#[derive(Debug)]
struct SessionStats {
''',
    '''#[derive(Debug, Clone, Copy)]
enum RebalanceReason {
    NotionalThreshold,
    PositionAge,
}

impl RebalanceReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::NotionalThreshold => "notional_threshold",
            Self::PositionAge => "position_age",
        }
    }
}

''',
)
replace_once(
    "src/bot.rs",
    '''    realized_pnl: Decimal,
    managed_order_ids: HashSet<u64>,
''',
    '''    realized_pnl: Decimal,
    taker_rebalance_attempts: u64,
    taker_rebalance_requested_notional: Decimal,
    managed_order_ids: HashSet<u64>,
''',
)
replace_once(
    "src/bot.rs",
    '''            realized_pnl: Decimal::ZERO,
            managed_order_ids: HashSet::new(),
''',
    '''            realized_pnl: Decimal::ZERO,
            taker_rebalance_attempts: 0,
            taker_rebalance_requested_notional: Decimal::ZERO,
            managed_order_ids: HashSet::new(),
''',
)
replace_once(
    "src/bot.rs",
    '''    id_sequence: u64,
    stale_orders_cancelled: bool,
}
''',
    '''    id_sequence: u64,
    stale_orders_cancelled: bool,
    inventory_since: Option<Instant>,
    inventory_direction: i8,
    last_taker_rebalance: Option<Instant>,
}
''',
)
replace_once(
    "src/bot.rs",
    '''            id_sequence: 0,
            stale_orders_cancelled: false,
        }
''',
    '''            id_sequence: 0,
            stale_orders_cancelled: false,
            inventory_since: None,
            inventory_direction: 0,
            last_taker_rebalance: None,
        }
''',
)
replace_once(
    "src/bot.rs",
    '''                    self.enforce_risk_limits(mid)?;
                    self.quote_once(&book).await?;
''',
    '''                    self.enforce_risk_limits(mid)?;
                    let rebalanced = self.maybe_rebalance_inventory(&book).await?;
                    if !rebalanced {
                        self.quote_once(&book).await?;
                    }
''',
)
replace_once(
    "src/bot.rs",
    '''            max_position_notional = %self.config.max_position_notional_usd,
            tick_size = %self.rules.tick_size,
''',
    '''            max_position_notional = %self.config.max_position_notional_usd,
            taker_rebalance_enabled = self.config.taker_rebalance_enabled,
            taker_rebalance_trigger_notional = %self.config.taker_rebalance_trigger_notional_usd,
            taker_rebalance_target_notional = %self.config.taker_rebalance_target_notional_usd,
            tick_size = %self.rules.tick_size,
''',
)
replace_once(
    "src/bot.rs",
    '''    async fn refresh_position_only(&mut self, fallback_mark: Decimal) -> Result<()> {
        self.position = self
            .client
            .position(&self.config.symbol, fallback_mark)
            .await
            .context("failed to refresh position after order-state change")?;
        Ok(())
    }
''',
    '''    async fn refresh_position_only(&mut self, fallback_mark: Decimal) -> Result<()> {
        let position = self
            .client
            .position(&self.config.symbol, fallback_mark)
            .await
            .context("failed to refresh position after order-state change")?;
        self.observe_inventory(position.quantity);
        self.position = position;
        Ok(())
    }
''',
)
insert_before(
    "src/bot.rs",
    '''    fn enforce_risk_limits(&self, mark_price: Decimal) -> Result<()> {
''',
    '''    fn observe_inventory(&mut self, quantity: Decimal) {
        let direction = position_direction(quantity);
        if direction == 0 {
            self.inventory_since = None;
            self.inventory_direction = 0;
        } else if direction != self.inventory_direction || self.inventory_since.is_none() {
            self.inventory_since = Some(Instant::now());
            self.inventory_direction = direction;
        }
    }

    fn rebalance_reason(&self, mark_price: Decimal) -> Option<RebalanceReason> {
        if !self.config.taker_rebalance_enabled || self.position.quantity == Decimal::ZERO {
            return None;
        }

        let position_notional = self.position.quantity.abs() * mark_price;
        if position_notional <= self.config.taker_rebalance_target_notional_usd {
            return None;
        }
        if position_notional >= self.config.taker_rebalance_trigger_notional_usd {
            return Some(RebalanceReason::NotionalThreshold);
        }

        let age = self.inventory_since?.elapsed();
        if age
            >= Duration::from_secs(self.config.taker_rebalance_max_position_age_secs)
        {
            return Some(RebalanceReason::PositionAge);
        }
        None
    }

    fn taker_rebalance_in_cooldown(&self) -> bool {
        self.last_taker_rebalance
            .map(|last| {
                last.elapsed()
                    < Duration::from_secs(self.config.taker_rebalance_cooldown_secs)
            })
            .unwrap_or(false)
    }

    async fn maybe_rebalance_inventory(&mut self, book: &Book) -> Result<bool> {
        let mid = (book.bid + book.ask) / Decimal::from(2_u32);
        let Some(initial_reason) = self.rebalance_reason(mid) else {
            return Ok(false);
        };
        if self.taker_rebalance_in_cooldown() {
            return Ok(false);
        }

        self.last_taker_rebalance = Some(Instant::now());
        warn!(
            reason = initial_reason.as_str(),
            position_qty = %self.position.quantity,
            position_notional = %(self.position.quantity.abs() * mid),
            "inventory rebalance trigger reached; cancelling maker quotes before reduce-only IOC"
        );

        self.cancel_managed_orders().await?;
        if !self.config.dry_run {
            self.refresh_position_only(mid).await?;
        }

        let Some(reason) = self.rebalance_reason(mid) else {
            info!("inventory changed while maker orders were being cancelled; IOC rebalance is no longer needed");
            return Ok(true);
        };

        let side = if self.position.quantity > Decimal::ZERO {
            OrderSide::Sell
        } else {
            OrderSide::Buy
        };
        let Some(quantity) = compute_rebalance_quantity(
            self.position.quantity,
            mid,
            self.config.taker_rebalance_target_notional_usd,
            self.config.taker_rebalance_max_order_notional_usd,
            &self.rules,
        ) else {
            warn!(
                position_qty = %self.position.quantity,
                min_qty = %self.rules.min_qty,
                "inventory exceeds the configured target but is too small for a valid reduce-only order"
            );
            return Ok(true);
        };
        let price = aggressive_ioc_price(
            side,
            book,
            self.config.taker_rebalance_max_slippage_bps,
            &self.rules,
        )
        .ok_or_else(|| anyhow!("could not compute a valid price-protected IOC limit"))?;
        let client_order_id = self.next_client_order_id(side);
        let requested_notional = quantity * mid;

        self.stats.taker_rebalance_attempts =
            self.stats.taker_rebalance_attempts.saturating_add(1);
        self.stats.taker_rebalance_requested_notional += requested_notional;

        if self.config.dry_run {
            info!(
                reason = reason.as_str(),
                side = side.as_str(),
                price = %price,
                quantity = %quantity,
                requested_notional = %requested_notional,
                reduce_only = true,
                time_in_force = "IOC",
                "DRY RUN: inventory would be reduced with a price-protected IOC limit"
            );
            return Ok(true);
        }

        match self
            .client
            .place_reduce_only_ioc_limit(
                &self.config.symbol,
                side,
                quantity,
                price,
                &client_order_id,
            )
            .await
        {
            Ok(placed) => {
                self.stats.managed_order_ids.insert(placed.order_id);
                self.stats.orders_placed = self.stats.orders_placed.saturating_add(1);
                info!(
                    reason = reason.as_str(),
                    order_id = placed.order_id,
                    client_order_id = %placed.client_order_id,
                    side = side.as_str(),
                    limit_price = %price,
                    quantity = %quantity,
                    executed_qty = %placed.executed_qty,
                    avg_price = %placed.avg_price,
                    status = %placed.status,
                    "submitted reduce-only IOC inventory rebalance"
                );

                if placed.status == "NEW" || placed.status == "PARTIALLY_FILLED" {
                    warn!(
                        order_id = placed.order_id,
                        status = %placed.status,
                        "IOC response was not final; cancelling any possible remainder"
                    );
                    self.cancel_by_client_id(&placed.client_order_id).await?;
                }
                self.refresh_position_only(mid).await?;
                Ok(true)
            }
            Err(error) if error.is_transient_rejection() => {
                warn!(%error, "reduce-only IOC was rejected; retaining the current position");
                self.refresh_position_only(mid).await?;
                Ok(true)
            }
            Err(error) if error.is_rate_limited() => {
                warn!(%error, "rate limited while submitting reduce-only IOC; pausing for one second");
                tokio::time::sleep(Duration::from_secs(1)).await;
                self.refresh_position_only(mid).await?;
                Ok(true)
            }
            Err(error) if error.is_execution_unknown() => {
                self.recover_unknown_rebalance(&client_order_id, mid, error)
                    .await?;
                Ok(true)
            }
            Err(error) => Err(error).context("failed to submit reduce-only IOC rebalance"),
        }
    }

    async fn recover_unknown_rebalance(
        &mut self,
        client_order_id: &str,
        fallback_mark: Decimal,
        original_error: AsterError,
    ) -> Result<()> {
        warn!(
            %original_error,
            %client_order_id,
            "IOC execution state is unknown; querying by client order ID"
        );

        for attempt in 1..=5 {
            tokio::time::sleep(Duration::from_millis(500)).await;
            match self
                .client
                .query_order(&self.config.symbol, client_order_id)
                .await
            {
                Ok(order) => {
                    self.stats.managed_order_ids.insert(order.order_id);
                    self.stats.orders_placed = self.stats.orders_placed.saturating_add(1);
                    info!(
                        attempt,
                        order_id = order.order_id,
                        status = %order.status,
                        executed_qty = %order.executed_qty,
                        "recovered reduce-only IOC after unknown execution response"
                    );
                    if order.status == "NEW" || order.status == "PARTIALLY_FILLED" {
                        self.cancel_by_client_id(client_order_id).await?;
                    }
                    self.refresh_position_only(fallback_mark).await?;
                    return Ok(());
                }
                Err(error) if error.is_no_such_order() => {
                    warn!(attempt, %client_order_id, "IOC order is not visible yet");
                }
                Err(error) if error.is_rate_limited() => {
                    warn!(attempt, %error, "rate limited while recovering IOC state");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                Err(error) => {
                    return Err(error).context("failed while recovering IOC execution state");
                }
            }
        }

        bail!(
            "could not determine whether reduce-only IOC order {} executed; halting instead of risking another aggressive order",
            client_order_id
        )
    }

''',
)
replace_once(
    "src/bot.rs",
    '''        let maker_share = if self.stats.executed_notional > Decimal::ZERO {
''',
    '''        let inventory_age_seconds = self
            .inventory_since
            .map(|started| started.elapsed().as_secs())
            .unwrap_or(0);
        let maker_share = if self.stats.executed_notional > Decimal::ZERO {
''',
)
replace_once(
    "src/bot.rs",
    '''            realized_pnl = %self.stats.realized_pnl,
            position_qty = %self.position.quantity,
''',
    '''            realized_pnl = %self.stats.realized_pnl,
            taker_rebalance_attempts = self.stats.taker_rebalance_attempts,
            taker_rebalance_requested_notional = %self.stats.taker_rebalance_requested_notional,
            position_qty = %self.position.quantity,
            inventory_age_seconds,
''',
)
insert_before(
    "src/bot.rs",
    '''fn normalize_quantity(
''',
    '''fn position_direction(quantity: Decimal) -> i8 {
    if quantity > Decimal::ZERO {
        1
    } else if quantity < Decimal::ZERO {
        -1
    } else {
        0
    }
}

fn compute_rebalance_quantity(
    position_quantity: Decimal,
    mark_price: Decimal,
    target_notional: Decimal,
    max_order_notional: Decimal,
    rules: &SymbolRules,
) -> Option<Decimal> {
    if position_quantity == Decimal::ZERO
        || mark_price <= Decimal::ZERO
        || max_order_notional <= Decimal::ZERO
    {
        return None;
    }

    let absolute_position = position_quantity.abs();
    let target_quantity = target_notional / mark_price;
    let excess_quantity = (absolute_position - target_quantity).max(Decimal::ZERO);
    let max_order_quantity = max_order_notional / mark_price;
    let quantity = floor_to_step(
        excess_quantity
            .min(max_order_quantity)
            .min(absolute_position)
            .min(rules.max_qty),
        rules.step_size,
    );

    if quantity < rules.min_qty || quantity <= Decimal::ZERO {
        None
    } else {
        Some(quantity)
    }
}

fn aggressive_ioc_price(
    side: OrderSide,
    book: &Book,
    max_slippage_bps: u64,
    rules: &SymbolRules,
) -> Option<Decimal> {
    let slippage =
        Decimal::from(max_slippage_bps) / Decimal::from(10_000_u32);
    let mut price = match side {
        OrderSide::Buy => ceil_to_step(
            book.ask * (Decimal::ONE + slippage),
            rules.tick_size,
        ),
        OrderSide::Sell => floor_to_step(
            book.bid * (Decimal::ONE - slippage),
            rules.tick_size,
        ),
    };

    if rules.min_price > Decimal::ZERO {
        price = price.max(rules.min_price);
    }
    if rules.max_price > Decimal::ZERO {
        price = price.min(rules.max_price);
    }

    if price <= Decimal::ZERO {
        None
    } else {
        Some(price)
    }
}

''',
)
replace_once(
    "src/bot.rs",
    '''    use super::{ceil_to_step, floor_to_step, normalize_quantity};
    use crate::api::SymbolRules;
''',
    '''    use std::time::Instant;

    use super::{
        aggressive_ioc_price, ceil_to_step, compute_rebalance_quantity, floor_to_step,
        normalize_quantity,
    };
    use crate::api::{Book, OrderSide, SymbolRules};
''',
)
insert_before(
    "src/bot.rs",
    '''    #[test]
    fn disables_side_when_risk_capacity_is_too_small() {
''',
    '''    #[test]
    fn rebalance_quantity_moves_toward_target_and_respects_order_cap() {
        let quantity = compute_rebalance_quantity(
            Decimal::new(500, 3),
            Decimal::new(100, 0),
            Decimal::new(10, 0),
            Decimal::new(20, 0),
            &rules(),
        )
        .expect("rebalance quantity");
        assert_eq!(quantity, Decimal::new(200, 3));
    }

    #[test]
    fn aggressive_ioc_price_is_bounded_by_configured_slippage() {
        let book = Book {
            bid: Decimal::new(10_000, 2),
            ask: Decimal::new(10_010, 2),
            received_at: Instant::now(),
        };
        let sell = aggressive_ioc_price(OrderSide::Sell, &book, 5, &rules())
            .expect("sell price");
        let buy = aggressive_ioc_price(OrderSide::Buy, &book, 5, &rules())
            .expect("buy price");
        assert!(sell <= book.bid);
        assert!(sell >= Decimal::new(9_995, 2));
        assert!(buy >= book.ask);
        assert!(buy <= Decimal::new(10_016, 2));
    }

''',
)

# ---------------------------------------------------------------------------
# Example config and documentation.
# ---------------------------------------------------------------------------
replace_once(
    "config.example.toml",
    '''max_unrealized_loss_usd = "5"

# Quote cadence and state polling.
''',
    '''max_unrealized_loss_usd = "5"

# Maker-first inventory control. When the net position reaches the notional
# threshold, or remains above the target for too long, the bot cancels its
# maker quotes and sends one reduce-only LIMIT+IOC order with price protection.
taker_rebalance_enabled = true
taker_rebalance_trigger_notional_usd = "30"
taker_rebalance_target_notional_usd = "5"
taker_rebalance_max_order_notional_usd = "20"
taker_rebalance_max_position_age_secs = 180
taker_rebalance_cooldown_secs = 30
taker_rebalance_max_slippage_bps = 5

# Quote cadence and state polling.
''',
)
replace_once(
    "README.md",
    '''It maintains at most one post-only bid and one post-only ask for a single symbol, applies inventory-based quote skew, enforces a maximum position and unrealized-loss limit, and only manages orders carrying its configured client-order prefix.
''',
    '''It maintains at most one post-only bid and one post-only ask for a single symbol, applies inventory-based quote skew, and can use a price-protected, reduce-only `LIMIT + IOC` order when inventory becomes too large or remains open too long. It enforces a maximum position and unrealized-loss limit and only manages orders carrying its configured client-order prefix.
''',
)
replace_once(
    "README.md",
    '''- inventory skew and per-side position-cap calculations
- startup recovery by cancelling only bot-prefixed orders
''',
    '''- inventory skew and per-side position-cap calculations
- maker-first inventory reduction through configurable, reduce-only `LIMIT + IOC` orders
- startup recovery by cancelling only bot-prefixed orders
''',
)
replace_once(
    "README.md",
    '''max_unrealized_loss_usd = "5"

refresh_ms = 1000
''',
    '''max_unrealized_loss_usd = "5"

taker_rebalance_enabled = true
taker_rebalance_trigger_notional_usd = "30"
taker_rebalance_target_notional_usd = "5"
taker_rebalance_max_order_notional_usd = "20"
taker_rebalance_max_position_age_secs = 180
taker_rebalance_cooldown_secs = 30
taker_rebalance_max_slippage_bps = 5

refresh_ms = 1000
''',
)
insert_before(
    "README.md",
    '''### Requote threshold
''',
    '''### Maker-first IOC inventory rebalancing

When enabled, the normal path remains post-only Maker quoting. A Taker order is used only to reduce an existing net position when either:

- absolute position notional reaches `taker_rebalance_trigger_notional_usd`; or
- the position stays above `taker_rebalance_target_notional_usd` for `taker_rebalance_max_position_age_secs`.

Before the aggressive order, the bot cancels its tracked Maker quotes and refreshes the position. It then submits one opposite-side `LIMIT + IOC` order with `reduceOnly=true`. The order quantity moves the position toward the configured target, is capped by `taker_rebalance_max_order_notional_usd`, and can never intentionally increase or reverse exposure. `taker_rebalance_max_slippage_bps` bounds the worst acceptable limit price, while `taker_rebalance_cooldown_secs` prevents repeated aggressive orders in a tight loop.

This is inventory-risk control, not a target-volume or target-Maker/Taker-ratio engine. It may produce no Taker volume when inventory is naturally balanced, and it does not guarantee campaign points, the screenshot's 83.8%/16.2% split, or profitability.

''',
)
replace_once(
    "README.md",
    '''- no automatic position liquidation or market close
''',
    '''- no market orders; aggressive inventory reduction uses price-protected, reduce-only IOC limits
- no automatic full liquidation after an unrealized-loss stop
''',
)

print("maker-first IOC inventory rebalancing patch applied")
