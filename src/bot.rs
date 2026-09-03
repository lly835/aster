use std::{
    collections::HashSet,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{anyhow, bail, Context, Result};
use rust_decimal::{prelude::ToPrimitive, Decimal};
use tokio::sync::watch;
use tracing::{info, warn};

const SERVER_TIME_RESYNC_SECS: u64 = 300;

use crate::{
    api::{AsterClient, AsterError, Book, OpenOrder, OrderSide, PositionSnapshot, SymbolRules},
    config::RuntimeConfig,
};

#[derive(Debug, Clone)]
struct QuoteTarget {
    price: Decimal,
    quantity: Decimal,
}

#[derive(Debug, Clone)]
struct ManagedOrder {
    order_id: u64,
    client_order_id: String,
    side: OrderSide,
    price: Decimal,
    quantity: Decimal,
    executed_qty: Decimal,
    status: String,
}

#[derive(Debug)]
struct SessionStats {
    started_at: Instant,
    started_at_ms: u64,
    quote_cycles: u64,
    orders_placed: u64,
    orders_cancelled: u64,
    quoted_notional: Decimal,
    executed_notional: Decimal,
    maker_notional: Decimal,
    taker_notional: Decimal,
    commission_abs: Decimal,
    realized_pnl: Decimal,
    managed_order_ids: HashSet<u64>,
    seen_trade_ids: HashSet<u64>,
    last_trade_id: Option<u64>,
}

impl SessionStats {
    fn new() -> Self {
        Self {
            started_at: Instant::now(),
            started_at_ms: unix_millis_u64(),
            quote_cycles: 0,
            orders_placed: 0,
            orders_cancelled: 0,
            quoted_notional: Decimal::ZERO,
            executed_notional: Decimal::ZERO,
            maker_notional: Decimal::ZERO,
            taker_notional: Decimal::ZERO,
            commission_abs: Decimal::ZERO,
            realized_pnl: Decimal::ZERO,
            managed_order_ids: HashSet::new(),
            seen_trade_ids: HashSet::new(),
            last_trade_id: None,
        }
    }
}

pub struct MarketMaker {
    client: AsterClient,
    config: RuntimeConfig,
    rules: SymbolRules,
    book_rx: watch::Receiver<Option<Book>>,

    position: PositionSnapshot,
    buy_order: Option<ManagedOrder>,
    sell_order: Option<ManagedOrder>,

    stats: SessionStats,
    id_sequence: u64,
    stale_orders_cancelled: bool,
}

impl MarketMaker {
    pub fn new(
        client: AsterClient,
        config: RuntimeConfig,
        rules: SymbolRules,
        book_rx: watch::Receiver<Option<Book>>,
        initial_book: &Book,
    ) -> Self {
        let mid = (initial_book.bid + initial_book.ask) / Decimal::from(2_u32);
        Self {
            client,
            config,
            rules,
            book_rx,
            position: PositionSnapshot::flat(mid),
            buy_order: None,
            sell_order: None,
            stats: SessionStats::new(),
            id_sequence: 0,
            stale_orders_cancelled: false,
        }
    }

    pub async fn run(mut self, once: bool) -> Result<()> {
        let run_result = self.run_loop(once).await;
        let cleanup_result = self.shutdown().await;

        match (run_result, cleanup_result) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(run_error), Ok(())) => Err(run_error),
            (Ok(()), Err(cleanup_error)) => Err(cleanup_error),
            (Err(run_error), Err(cleanup_error)) => Err(anyhow!(
                "bot failed: {run_error:#}; cleanup also failed: {cleanup_error:#}"
            )),
        }
    }

    async fn run_loop(&mut self, once: bool) -> Result<()> {
        self.prepare().await?;

        let mut quote_interval =
            tokio::time::interval(Duration::from_millis(self.config.refresh_ms));
        quote_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        let now = Instant::now();
        let mut last_clock_sync = now;
        let mut last_state_refresh = now
            .checked_sub(Duration::from_millis(self.config.position_refresh_ms))
            .unwrap_or(now);
        let mut last_stats_refresh = now;
        let mut last_deadman_heartbeat = now
            .checked_sub(Duration::from_millis(self.config.deadman_heartbeat_ms))
            .unwrap_or(now);
        let mut shutdown_signal = Box::pin(tokio::signal::ctrl_c());

        loop {
            tokio::select! {
                signal = &mut shutdown_signal => {
                    signal.context("failed to listen for Ctrl-C")?;
                    info!("shutdown signal received");
                    break;
                }
                _ = quote_interval.tick() => {
                    let Some(book) = self.book_rx.borrow().clone() else {
                        warn!("no order-book update is available yet");
                        continue;
                    };

                    if book.received_at.elapsed()
                        > Duration::from_millis(self.config.stale_book_ms)
                    {
                        if !self.stale_orders_cancelled {
                            warn!(
                                stale_for_ms = book.received_at.elapsed().as_millis(),
                                "order book is stale; cancelling bot orders and pausing quotes"
                            );
                            self.cancel_managed_orders().await?;
                            self.stale_orders_cancelled = true;
                        }
                        continue;
                    }
                    self.stale_orders_cancelled = false;

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

                    if !self.config.dry_run
                        && last_state_refresh.elapsed()
                            >= Duration::from_millis(
                                self.config.position_refresh_ms,
                            )
                    {
                        self.refresh_live_state(mid).await?;
                        last_state_refresh = Instant::now();
                    }

                    self.enforce_risk_limits(mid)?;
                    self.quote_once(&book).await?;

                    if !self.config.dry_run
                        && last_stats_refresh.elapsed()
                            >= Duration::from_secs(
                                self.config.stats_interval_secs,
                            )
                    {
                        self.refresh_trade_stats().await;
                        self.log_stats(mid);
                        last_stats_refresh = Instant::now();
                    } else if self.config.dry_run
                        && last_stats_refresh.elapsed()
                            >= Duration::from_secs(
                                self.config.stats_interval_secs,
                            )
                    {
                        self.log_stats(mid);
                        last_stats_refresh = Instant::now();
                    }

                    if self.config.deadman_switch_enabled
                        && last_deadman_heartbeat.elapsed()
                            >= Duration::from_millis(
                                self.config.deadman_heartbeat_ms,
                            )
                    {
                        self.client
                            .countdown_cancel_all(
                                &self.config.symbol,
                                self.config.deadman_countdown_ms,
                            )
                            .await
                            .context("failed to refresh exchange dead-man switch")?;
                        last_deadman_heartbeat = Instant::now();
                    }

                    if once {
                        info!("--once completed; stopping");
                        break;
                    }
                }
            }
        }

        Ok(())
    }

    async fn prepare(&mut self) -> Result<()> {
        if self.rules.status != "TRADING" {
            bail!(
                "symbol {} is not in TRADING status (current status: {})",
                self.rules.symbol,
                self.rules.status
            );
        }

        info!(
            symbol = %self.config.symbol,
            environment = ?self.config.environment,
            dry_run = self.config.dry_run,
            quote_notional = %self.config.quote_notional_usd,
            max_position_notional = %self.config.max_position_notional_usd,
            tick_size = %self.rules.tick_size,
            step_size = %self.rules.step_size,
            min_notional = %self.rules.min_notional,
            "starting Aster market maker"
        );

        if self.config.dry_run {
            info!(
                "dry-run mode is enabled; orders will only be logged and no signed API calls will be sent"
            );
            return Ok(());
        }

        if !self.client.has_credentials() {
            bail!("live mode requires Aster API credentials");
        }

        self.client
            .sync_server_time()
            .await
            .context("failed to synchronize server time")?;

        if self
            .client
            .is_hedge_mode()
            .await
            .context("failed to read position mode")?
        {
            bail!(
                "this bot supports one-way position mode only; switch the Aster account out of hedge mode before running it"
            );
        }

        if self.config.startup_cancel_existing_bot_orders {
            self.cancel_all_orders_with_prefix().await?;
        } else {
            self.ensure_no_existing_bot_orders().await?;
        }

        let current_book = self
            .book_rx
            .borrow()
            .clone()
            .ok_or_else(|| anyhow!("no initial book is available"))?;
        let mid = (current_book.bid + current_book.ask) / Decimal::from(2_u32);
        self.refresh_live_state(mid).await?;

        if self.config.deadman_switch_enabled {
            warn!(
                countdown_ms = self.config.deadman_countdown_ms,
                "exchange dead-man switch is enabled; Aster will cancel ALL open orders for this symbol if heartbeats stop"
            );
            self.client
                .countdown_cancel_all(&self.config.symbol, self.config.deadman_countdown_ms)
                .await
                .context("failed to arm exchange dead-man switch")?;
        }

        Ok(())
    }

    async fn refresh_live_state(&mut self, fallback_mark: Decimal) -> Result<()> {
        let open_orders = self
            .client
            .open_orders(&self.config.symbol)
            .await
            .context("failed to refresh open orders")?;

        self.reconcile_open_orders(open_orders).await?;
        self.refresh_position_only(fallback_mark).await
    }

    async fn refresh_position_only(&mut self, fallback_mark: Decimal) -> Result<()> {
        self.position = self
            .client
            .position(&self.config.symbol, fallback_mark)
            .await
            .context("failed to refresh position after order-state change")?;
        Ok(())
    }

    async fn reconcile_open_orders(&mut self, open_orders: Vec<OpenOrder>) -> Result<bool> {
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

        for order in open_orders.into_iter().filter(|order| {
            order
                .client_order_id
                .starts_with(&self.config.client_order_prefix)
        }) {
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

    fn enforce_risk_limits(&self, mark_price: Decimal) -> Result<()> {
        if self.position.unrealized_pnl <= -self.config.max_unrealized_loss_usd {
            bail!(
                "unrealized loss limit reached: uPnL={} USD, configured limit=-{} USD; open position is left unchanged",
                self.position.unrealized_pnl,
                self.config.max_unrealized_loss_usd
            );
        }

        let position_notional = self.position.quantity.abs() * mark_price;
        if position_notional
            > self.config.max_position_notional_usd + self.config.quote_notional_usd
        {
            warn!(
                position_notional = %position_notional,
                max_position_notional = %self.config.max_position_notional_usd,
                "position is already above the configured limit; only risk-reducing quotes will be allowed"
            );
        }

        Ok(())
    }

    async fn quote_once(&mut self, book: &Book) -> Result<()> {
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
        self.place_missing_side(OrderSide::Sell, sell_target)
            .await?;

        Ok(())
    }

    async fn cancel_side_if_needed(
        &mut self,
        side: OrderSide,
        target: Option<&QuoteTarget>,
    ) -> Result<bool> {
        let should_cancel = match (self.current_order(side), target) {
            (Some(_), None) => true,
            (Some(existing), Some(target)) => self.needs_requote(existing, target),
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

    fn compute_targets(&self, book: &Book) -> Result<(Option<QuoteTarget>, Option<QuoteTarget>)> {
        let mid = (book.bid + book.ask) / Decimal::from(2_u32);
        if mid <= Decimal::ZERO {
            bail!("cannot quote on a non-positive mid price");
        }

        let signed_position_notional = self.position.quantity * mid;
        let absolute_position_notional = signed_position_notional.abs();
        let mut inventory_ratio =
            absolute_position_notional / self.config.max_position_notional_usd;
        if inventory_ratio > Decimal::ONE {
            inventory_ratio = Decimal::ONE;
        }

        let skew_ticks = (inventory_ratio * Decimal::from(self.config.inventory_skew_ticks))
            .ceil()
            .to_u64()
            .unwrap_or(self.config.inventory_skew_ticks);
        let skew = self.rules.tick_size * Decimal::from(skew_ticks);

        let mut buy_price =
            book.bid - self.rules.tick_size * Decimal::from(self.config.bid_offset_ticks);
        let mut sell_price =
            book.ask + self.rules.tick_size * Decimal::from(self.config.ask_offset_ticks);

        if self.position.quantity > Decimal::ZERO {
            buy_price -= skew;
            sell_price -= skew;
        } else if self.position.quantity < Decimal::ZERO {
            buy_price += skew;
            sell_price += skew;
        }

        buy_price = floor_to_step(buy_price, self.rules.tick_size);
        sell_price = ceil_to_step(sell_price, self.rules.tick_size);

        if buy_price >= book.ask {
            buy_price = floor_to_step(book.ask - self.rules.tick_size, self.rules.tick_size);
        }
        if sell_price <= book.bid {
            sell_price = ceil_to_step(book.bid + self.rules.tick_size, self.rules.tick_size);
        }

        if self.rules.min_price > Decimal::ZERO {
            buy_price = buy_price.max(self.rules.min_price);
            sell_price = sell_price.max(self.rules.min_price);
        }
        if self.rules.max_price > Decimal::ZERO {
            buy_price = buy_price.min(self.rules.max_price);
            sell_price = sell_price.min(self.rules.max_price);
        }

        if buy_price <= Decimal::ZERO || sell_price <= Decimal::ZERO || buy_price >= sell_price {
            bail!(
                "computed invalid quote prices: buy={}, sell={}, best_bid={}, best_ask={}",
                buy_price,
                sell_price,
                book.bid,
                book.ask
            );
        }

        let max_position_qty = self.config.max_position_notional_usd / mid;
        let buy_capacity = (max_position_qty - self.position.quantity).max(Decimal::ZERO);
        let sell_capacity = (max_position_qty + self.position.quantity).max(Decimal::ZERO);

        let desired_buy_qty = self.config.quote_notional_usd / buy_price;
        let desired_sell_qty = self.config.quote_notional_usd / sell_price;

        let buy_quantity =
            normalize_quantity(desired_buy_qty, buy_capacity, buy_price, &self.rules);
        let sell_quantity =
            normalize_quantity(desired_sell_qty, sell_capacity, sell_price, &self.rules);

        Ok((
            buy_quantity.map(|quantity| QuoteTarget {
                price: buy_price,
                quantity,
            }),
            sell_quantity.map(|quantity| QuoteTarget {
                price: sell_price,
                quantity,
            }),
        ))
    }

    fn needs_requote(&self, existing: &ManagedOrder, target: &QuoteTarget) -> bool {
        let threshold = self.rules.tick_size * Decimal::from(self.config.requote_threshold_ticks);
        (existing.price - target.price).abs() >= threshold
            || (existing.quantity - target.quantity).abs() >= self.rules.step_size
            || existing.executed_qty > Decimal::ZERO
            || existing.status != "NEW"
    }

    async fn place_target(&mut self, side: OrderSide, target: QuoteTarget) -> Result<()> {
        let client_order_id = self.next_client_order_id(side);
        let notional = target.price * target.quantity;

        if self.config.dry_run {
            self.id_sequence = self.id_sequence.saturating_add(1);
            let order = ManagedOrder {
                order_id: self.id_sequence,
                client_order_id,
                side,
                price: target.price,
                quantity: target.quantity,
                executed_qty: Decimal::ZERO,
                status: "NEW".to_owned(),
            };
            info!(
                side = side.as_str(),
                price = %target.price,
                quantity = %target.quantity,
                notional = %notional,
                "DRY RUN: placing GTX post-only order"
            );
            self.stats.orders_placed = self.stats.orders_placed.saturating_add(1);
            self.stats.quoted_notional += notional;
            self.set_current_order(side, Some(order));
            return Ok(());
        }

        match self
            .client
            .place_limit(
                &self.config.symbol,
                side,
                target.quantity,
                target.price,
                &client_order_id,
            )
            .await
        {
            Ok(placed) => {
                let position_may_have_changed = placed.status != "NEW";
                self.stats.managed_order_ids.insert(placed.order_id);
                info!(
                    order_id = placed.order_id,
                    client_order_id = %placed.client_order_id,
                    side = side.as_str(),
                    price = %target.price,
                    quantity = %target.quantity,
                    notional = %notional,
                    status = %placed.status,
                    "placed GTX post-only order"
                );
                self.stats.orders_placed = self.stats.orders_placed.saturating_add(1);
                self.stats.quoted_notional += notional;

                if placed.status == "NEW" || placed.status == "PARTIALLY_FILLED" {
                    self.set_current_order(
                        side,
                        Some(ManagedOrder {
                            order_id: placed.order_id,
                            client_order_id: placed.client_order_id,
                            side,
                            price: target.price,
                            quantity: target.quantity,
                            executed_qty: Decimal::ZERO,
                            status: placed.status,
                        }),
                    );
                }
                if position_may_have_changed {
                    self.refresh_position_only(target.price).await?;
                }
                Ok(())
            }
            Err(error) if error.is_transient_rejection() => {
                warn!(
                    %error,
                    side = side.as_str(),
                    price = %target.price,
                    quantity = %target.quantity,
                    "order was rejected without an unknown execution state; skipping this quote"
                );
                Ok(())
            }
            Err(error) if error.is_rate_limited() => {
                warn!(%error, "Aster rate limit reached; pausing for one second");
                tokio::time::sleep(Duration::from_secs(1)).await;
                Ok(())
            }
            Err(error) if error.is_execution_unknown() => {
                self.recover_unknown_order(side, target, client_order_id, error)
                    .await
            }
            Err(error) => Err(error).context("failed to place order"),
        }
    }

    async fn recover_unknown_order(
        &mut self,
        side: OrderSide,
        target: QuoteTarget,
        client_order_id: String,
        original_error: AsterError,
    ) -> Result<()> {
        warn!(
            %original_error,
            client_order_id = %client_order_id,
            "order execution state is unknown; querying by client order ID and refusing to retry blindly"
        );

        for attempt in 1..=5 {
            tokio::time::sleep(Duration::from_millis(500)).await;
            match self
                .client
                .query_order(&self.config.symbol, &client_order_id)
                .await
            {
                Ok(order) => {
                    let position_may_have_changed =
                        order.executed_qty > Decimal::ZERO || order.status != "NEW";
                    self.stats.managed_order_ids.insert(order.order_id);
                    info!(
                        attempt,
                        order_id = order.order_id,
                        status = %order.status,
                        "recovered order after unknown execution response"
                    );

                    self.stats.orders_placed = self.stats.orders_placed.saturating_add(1);
                    self.stats.quoted_notional += target.price * target.quantity;
                    if order.status == "NEW" || order.status == "PARTIALLY_FILLED" {
                        self.set_current_order(side, Some(managed_from_open(order)));
                    }
                    if position_may_have_changed {
                        self.refresh_position_only(target.price).await?;
                    }
                    return Ok(());
                }
                Err(error) if error.is_no_such_order() => {
                    warn!(
                        attempt,
                        client_order_id = %client_order_id,
                        "order is not visible yet after unknown execution response"
                    );
                }
                Err(error) if error.is_rate_limited() => {
                    warn!(attempt, %error, "rate limited while recovering order state");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                Err(error) => {
                    return Err(error).context("failed while recovering unknown order state");
                }
            }
        }

        bail!(
            "could not determine whether order {} was accepted after an unknown execution response; halting instead of risking a duplicate order",
            client_order_id
        )
    }

    async fn cancel_existing(&mut self, order: &ManagedOrder) -> Result<()> {
        if self.config.dry_run {
            info!(
                order_id = order.order_id,
                client_order_id = %order.client_order_id,
                side = order.side.as_str(),
                price = %order.price,
                quantity = %order.quantity,
                "DRY RUN: cancelling order"
            );
            self.stats.orders_cancelled = self.stats.orders_cancelled.saturating_add(1);
            return Ok(());
        }

        self.cancel_by_client_id(&order.client_order_id).await
    }

    async fn cancel_by_client_id(&mut self, client_order_id: &str) -> Result<()> {
        match self
            .client
            .cancel_order(&self.config.symbol, client_order_id)
            .await
        {
            Ok(()) => {
                self.stats.orders_cancelled = self.stats.orders_cancelled.saturating_add(1);
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
            Err(error) => Err(error).context("failed to cancel existing order"),
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
                    self.stats.orders_cancelled = self.stats.orders_cancelled.saturating_add(1);
                    return Ok(());
                }
                Err(error) if error.is_rate_limited() => {
                    warn!(attempt, %error, "rate limited while verifying cancellation");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                Err(error) => {
                    return Err(error).context("failed while verifying unknown cancel state");
                }
                Ok(order) if order.status != "NEW" && order.status != "PARTIALLY_FILLED" => {
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

    async fn cancel_managed_orders(&mut self) -> Result<()> {
        if let Some(order) = self.buy_order.take() {
            self.cancel_existing(&order).await?;
        }
        if let Some(order) = self.sell_order.take() {
            self.cancel_existing(&order).await?;
        }
        Ok(())
    }

    async fn ensure_no_existing_bot_orders(&self) -> Result<()> {
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
        if self.config.dry_run {
            self.buy_order = None;
            self.sell_order = None;
            return Ok(());
        }

        let orders = self
            .client
            .open_orders(&self.config.symbol)
            .await
            .context("failed to list open orders for cleanup")?;

        let bot_orders = orders
            .into_iter()
            .filter(|order| {
                order
                    .client_order_id
                    .starts_with(&self.config.client_order_prefix)
            })
            .collect::<Vec<_>>();

        if !bot_orders.is_empty() {
            warn!(
                count = bot_orders.len(),
                prefix = %self.config.client_order_prefix,
                "cancelling existing bot-prefixed orders"
            );
        }

        for order in bot_orders {
            self.cancel_by_client_id(&order.client_order_id).await?;
        }

        self.buy_order = None;
        self.sell_order = None;
        Ok(())
    }

    async fn refresh_trade_stats(&mut self) {
        let mut from_id = self.stats.last_trade_id.map(|id| id.saturating_add(1));
        let mut start_time = if from_id.is_none() {
            Some(self.stats.started_at_ms)
        } else {
            None
        };

        for _ in 0..10 {
            let result = self
                .client
                .user_trades(&self.config.symbol, start_time, from_id, 1_000)
                .await;

            let fills = match result {
                Ok(fills) => fills,
                Err(error) => {
                    warn!(%error, "failed to refresh session trade statistics");
                    return;
                }
            };

            if fills.is_empty() {
                return;
            }

            let count = fills.len();
            let mut highest_id = self.stats.last_trade_id;

            for fill in fills {
                highest_id = Some(
                    highest_id
                        .map(|value| value.max(fill.id))
                        .unwrap_or(fill.id),
                );

                if !self.stats.managed_order_ids.contains(&fill.order_id) {
                    continue;
                }
                if !self.stats.seen_trade_ids.insert(fill.id) {
                    continue;
                }

                self.stats.executed_notional += fill.quote_qty;
                if fill.maker {
                    self.stats.maker_notional += fill.quote_qty;
                } else {
                    self.stats.taker_notional += fill.quote_qty;
                }
                self.stats.commission_abs += fill.commission.abs();
                self.stats.realized_pnl += fill.realized_pnl;
            }

            self.stats.last_trade_id = highest_id;
            if count < 1_000 {
                return;
            }

            from_id = highest_id.map(|id| id.saturating_add(1));
            start_time = None;
        }

        warn!(
            "trade-stat pagination reached the 10-page safety limit; session metrics may be incomplete"
        );
    }

    fn log_stats(&self, mark_price: Decimal) {
        let elapsed = self.stats.started_at.elapsed().as_secs_f64();
        let position_notional = self.position.quantity * mark_price;
        let maker_share = if self.stats.executed_notional > Decimal::ZERO {
            (self.stats.maker_notional / self.stats.executed_notional) * Decimal::from(100_u32)
        } else {
            Decimal::ZERO
        };

        info!(
            elapsed_seconds = elapsed,
            quote_cycles = self.stats.quote_cycles,
            orders_placed = self.stats.orders_placed,
            orders_cancelled = self.stats.orders_cancelled,
            quoted_notional = %self.stats.quoted_notional,
            executed_notional = %self.stats.executed_notional,
            maker_notional = %self.stats.maker_notional,
            taker_notional = %self.stats.taker_notional,
            maker_share_percent = %maker_share.round_dp(2),
            commission_abs = %self.stats.commission_abs,
            realized_pnl = %self.stats.realized_pnl,
            position_qty = %self.position.quantity,
            position_notional = %position_notional,
            entry_price = %self.position.entry_price,
            mark_price = %self.position.mark_price,
            unrealized_pnl = %self.position.unrealized_pnl,
            "session statistics"
        );
    }

    async fn shutdown(&mut self) -> Result<()> {
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
                    errors.push(format!("failed to disarm dead-man switch: {error}"));
                }
            } else {
                warn!("bot-order cleanup failed; leaving the exchange dead-man switch armed");
            }
        }

        if !self.config.dry_run {
            self.refresh_trade_stats().await;
            let fallback_mark = self.position.mark_price;
            if let Err(error) = self.refresh_position_only(fallback_mark).await {
                warn!(%error, "failed to refresh final position statistics");
            }
        }
        self.log_stats(self.position.mark_price);

        if errors.is_empty() {
            Ok(())
        } else {
            bail!("{}", errors.join("; "))
        }
    }

    fn next_client_order_id(&mut self, side: OrderSide) -> String {
        self.id_sequence = self.id_sequence.saturating_add(1);
        let timestamp = unix_millis_u64();
        format!(
            "{}{}{:x}{:x}",
            self.config.client_order_prefix,
            side.short(),
            timestamp,
            self.id_sequence
        )
    }

    fn current_order(&self, side: OrderSide) -> Option<&ManagedOrder> {
        match side {
            OrderSide::Buy => self.buy_order.as_ref(),
            OrderSide::Sell => self.sell_order.as_ref(),
        }
    }

    fn take_current_order(&mut self, side: OrderSide) -> Option<ManagedOrder> {
        match side {
            OrderSide::Buy => self.buy_order.take(),
            OrderSide::Sell => self.sell_order.take(),
        }
    }

    fn set_current_order(&mut self, side: OrderSide, order: Option<ManagedOrder>) {
        match side {
            OrderSide::Buy => self.buy_order = order,
            OrderSide::Sell => self.sell_order = order,
        }
    }
}

fn managed_from_open(order: OpenOrder) -> ManagedOrder {
    ManagedOrder {
        order_id: order.order_id,
        client_order_id: order.client_order_id,
        side: order.side,
        price: order.price,
        quantity: order.original_qty,
        executed_qty: order.executed_qty,
        status: order.status,
    }
}

fn normalize_quantity(
    desired: Decimal,
    capacity: Decimal,
    price: Decimal,
    rules: &SymbolRules,
) -> Option<Decimal> {
    if desired <= Decimal::ZERO || capacity <= Decimal::ZERO || price <= Decimal::ZERO {
        return None;
    }

    let minimum_for_notional = if rules.min_notional > Decimal::ZERO {
        ceil_to_step(rules.min_notional / price, rules.step_size)
    } else {
        Decimal::ZERO
    };
    let required = rules.min_qty.max(minimum_for_notional);

    let desired = floor_to_step(desired, rules.step_size).max(required);
    let capacity = floor_to_step(capacity, rules.step_size);
    let quantity = desired.min(capacity).min(rules.max_qty);

    if quantity < required || quantity <= Decimal::ZERO || quantity * price < rules.min_notional {
        None
    } else {
        Some(quantity)
    }
}

fn floor_to_step(value: Decimal, step: Decimal) -> Decimal {
    if step <= Decimal::ZERO {
        return value;
    }
    (value / step).floor() * step
}

fn ceil_to_step(value: Decimal, step: Decimal) -> Decimal {
    if step <= Decimal::ZERO {
        return value;
    }
    (value / step).ceil() * step
}

fn unix_millis_u64() -> u64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    millis.min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use rust_decimal::Decimal;

    use super::{ceil_to_step, floor_to_step, normalize_quantity};
    use crate::api::SymbolRules;

    fn rules() -> SymbolRules {
        SymbolRules {
            symbol: "TEST".to_owned(),
            status: "TRADING".to_owned(),
            tick_size: Decimal::new(1, 2),
            min_price: Decimal::ZERO,
            max_price: Decimal::new(1_000_000, 0),
            step_size: Decimal::new(1, 3),
            min_qty: Decimal::new(1, 3),
            max_qty: Decimal::new(1_000_000, 0),
            min_notional: Decimal::new(5, 0),
        }
    }

    #[test]
    fn rounds_to_exchange_steps() {
        assert_eq!(
            floor_to_step(Decimal::new(1234, 3), Decimal::new(1, 2)),
            Decimal::new(123, 2)
        );
        assert_eq!(
            ceil_to_step(Decimal::new(1234, 3), Decimal::new(1, 2)),
            Decimal::new(124, 2)
        );
    }

    #[test]
    fn raises_quantity_to_minimum_notional() {
        let result = normalize_quantity(
            Decimal::new(1, 3),
            Decimal::new(1, 0),
            Decimal::new(100, 0),
            &rules(),
        )
        .expect("quantity");
        assert_eq!(result, Decimal::new(5, 2));
    }

    #[test]
    fn disables_side_when_risk_capacity_is_too_small() {
        let result = normalize_quantity(
            Decimal::new(1, 0),
            Decimal::new(1, 3),
            Decimal::new(100, 0),
            &rules(),
        );
        assert!(result.is_none());
    }
}
