#!/usr/bin/env python3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def replace(path: str, old: str, new: str) -> None:
    target = ROOT / path
    text = target.read_text(encoding="utf-8")
    if text.count(old) != 1:
        raise SystemExit(f"unexpected {path} layout for: {old[:80]!r}")
    target.write_text(text.replace(old, new, 1), encoding="utf-8")


replace(
    "src/api.rs",
    '''    pub fn is_no_such_order(&self) -> bool {
        matches!(
            self,
            Self::Api {
                code: -2011 | -2013,
                ..
            }
        )
    }
''',
    '''    pub fn is_no_such_order(&self) -> bool {
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
''',
)

replace(
    "src/api.rs",
    '''        assert!(api_error_from_value(&json!({"code": 200, "msg": "success"})).is_none());
    }
''',
    '''        assert!(api_error_from_value(&json!({"code": 200, "msg": "success"})).is_none());
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
''',
)

replace(
    "src/bot.rs",
    '''        self.position = self
            .client
            .position(&self.config.symbol, fallback_mark)
            .await
            .context("failed to refresh position")?;

        let position_may_have_changed = self.reconcile_open_orders(open_orders).await?;
        if position_may_have_changed {
            self.refresh_position_only(fallback_mark).await?;
        }
        Ok(())
''',
    '''        self.reconcile_open_orders(open_orders).await?;
        self.refresh_position_only(fallback_mark).await
''',
)

replace(
    "src/bot.rs",
    '''            Ok(placed) => {
                self.stats.managed_order_ids.insert(placed.order_id);
''',
    '''            Ok(placed) => {
                let position_may_have_changed = placed.status != "NEW";
                self.stats.managed_order_ids.insert(placed.order_id);
''',
)

replace(
    "src/bot.rs",
    '''                }
                Ok(())
            }
            Err(error) if error.is_transient_rejection() => {
''',
    '''                }
                if position_may_have_changed {
                    self.refresh_position_only(target.price).await?;
                }
                Ok(())
            }
            Err(error) if error.is_transient_rejection() => {
''',
)

replace(
    "src/bot.rs",
    '''                Ok(order) => {
                    self.stats.managed_order_ids.insert(order.order_id);
''',
    '''                Ok(order) => {
                    let position_may_have_changed =
                        order.executed_qty > Decimal::ZERO || order.status != "NEW";
                    self.stats.managed_order_ids.insert(order.order_id);
''',
)

replace(
    "src/bot.rs",
    '''                    if order.status == "NEW" || order.status == "PARTIALLY_FILLED" {
                        self.stats.orders_placed = self.stats.orders_placed.saturating_add(1);
                        self.stats.quoted_notional += target.price * target.quantity;
                        self.set_current_order(side, Some(managed_from_open(order)));
                    }
                    return Ok(());
''',
    '''                    self.stats.orders_placed = self.stats.orders_placed.saturating_add(1);
                    self.stats.quoted_notional += target.price * target.quantity;
                    if order.status == "NEW" || order.status == "PARTIALLY_FILLED" {
                        self.set_current_order(side, Some(managed_from_open(order)));
                    }
                    if position_may_have_changed {
                        self.refresh_position_only(target.price).await?;
                    }
                    return Ok(());
''',
)

replace(
    "src/bot.rs",
    '''                Err(error) => {
                    return Err(error).context("failed while recovering unknown order state");
                }
''',
    '''                Err(error) if error.is_rate_limited() => {
                    warn!(attempt, %error, "rate limited while recovering order state");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                Err(error) => {
                    return Err(error).context("failed while recovering unknown order state");
                }
''',
)

replace(
    "src/bot.rs",
    '''        self.log_stats(self.position.mark_price);

        if errors.is_empty() {
''',
    '''        if !self.config.dry_run {
            self.refresh_trade_stats().await;
            let fallback_mark = self.position.mark_price;
            if let Err(error) = self.refresh_position_only(fallback_mark).await {
                warn!(%error, "failed to refresh final position statistics");
            }
        }
        self.log_stats(self.position.mark_price);

        if errors.is_empty() {
''',
)

replace("README.md", "- Rust 1.82 or newer\n", "- the current stable Rust toolchain\n")
replace(
    "README.md",
    "cargo check --all-targets\ncargo test --all-targets\n",
    "cargo check --locked --all-targets\ncargo test --locked --all-targets\n",
)
replace(
    "README.md",
    "- position and open-order reconciliation currently use REST polling; position is refreshed after cancellation before replacement\n",
    "- position and open-order reconciliation currently use REST polling; open orders are reconciled before the position snapshot used for replacement quotes\n",
)

print("final safety review patch applied")
