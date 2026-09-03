#!/usr/bin/env python3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def replace(path: str, old: str, new: str) -> None:
    target = ROOT / path
    text = target.read_text(encoding="utf-8")
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected one match, found {count}: {old[:120]!r}")
    target.write_text(text.replace(old, new, 1), encoding="utf-8")


# Existing config files that predate this feature must not silently start
# submitting taker orders after an upgrade. New example configs opt in
# explicitly, but serde's fallback default remains disabled.
replace(
    "src/config.rs",
    '''            taker_rebalance_enabled: true,
            taker_rebalance_trigger_notional_usd: "30".to_owned(),
''',
    '''            taker_rebalance_enabled: false,
            taker_rebalance_trigger_notional_usd: "30".to_owned(),
''',
)

replace(
    "src/config.rs",
    '''    #[test]
    fn validates_client_order_prefix() {
''',
    '''    #[test]
    fn taker_rebalancing_is_disabled_when_omitted() {
        assert!(!super::FileConfig::default().taker_rebalance_enabled);
    }

    #[test]
    fn validates_client_order_prefix() {
''',
)

# Inventory age applies only while the position is above the configured
# target. Dropping below target resets the timer, even if the sign is unchanged.
replace(
    "src/bot.rs",
    '''        self.observe_inventory(position.quantity);
        self.position = position;
''',
    '''        self.observe_inventory(position.quantity, position.mark_price);
        self.position = position;
''',
)

replace(
    "src/bot.rs",
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
''',
    '''    fn observe_inventory(&mut self, quantity: Decimal, mark_price: Decimal) {
        let direction = position_direction(quantity);
        let at_or_below_target = quantity.abs() * mark_price
            <= self.config.taker_rebalance_target_notional_usd;

        if direction == 0 || at_or_below_target {
            self.inventory_since = None;
            self.inventory_direction = direction;
        } else if direction != self.inventory_direction || self.inventory_since.is_none() {
            self.inventory_since = Some(Instant::now());
            self.inventory_direction = direction;
        }
    }
''',
)

# If a rebalance was attempted but inventory remains above the trigger, do not
# reopen two-sided maker quotes during the cooldown and accidentally add to the
# position. Pause quoting until the next permitted risk-reducing attempt.
replace(
    "src/bot.rs",
    '''        if self.taker_rebalance_in_cooldown() {
            return Ok(false);
        }
''',
    '''        if self.taker_rebalance_in_cooldown() {
            if self.buy_order.is_some() || self.sell_order.is_some() {
                warn!(
                    position_qty = %self.position.quantity,
                    "inventory still requires rebalancing during cooldown; cancelling maker quotes and pausing new quotes"
                );
                self.cancel_managed_orders().await?;
            }
            return Ok(true);
        }
''',
)

replace(
    "README.md",
    '''When enabled, the normal path remains post-only Maker quoting. A Taker order is used only to reduce an existing net position when either:
''',
    '''When explicitly enabled, the normal path remains post-only Maker quoting. The internal compatibility default is disabled, so an older `config.toml` that omits these fields will not silently begin sending Taker orders after an upgrade. The checked-in example opts in visibly. A Taker order is used only to reduce an existing net position when either:
''',
)

replace(
    "README.md",
    '''This is inventory-risk control, not a target-volume or target-Maker/Taker-ratio engine. It may produce no Taker volume when inventory is naturally balanced, and it does not guarantee campaign points, the screenshot's 83.8%/16.2% split, or profitability.
''',
    '''If inventory still requires rebalancing during the configured cooldown, the bot pauses Maker quoting instead of reopening both sides and potentially increasing exposure.

This is inventory-risk control, not a target-volume or target-Maker/Taker-ratio engine. It may produce no Taker volume when inventory is naturally balanced, and it does not guarantee campaign points, the screenshot's 83.8%/16.2% split, or profitability.
''',
)

print("IOC rebalance review fixes applied")
