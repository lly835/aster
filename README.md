# Aster Rust Market Maker

A conservative Rust market-making bot for the **Aster Futures API v3**.

It maintains at most one post-only bid and one post-only ask for a single symbol, applies inventory-based quote skew, and can use a price-protected, reduce-only `LIMIT + IOC` order when inventory becomes too large or remains open too long. It enforces a maximum position and unrealized-loss limit and only manages orders carrying its configured client-order prefix.

This project is for legitimate market making and execution research. It does **not** implement self-trading, multi-account matching, wash trading, or any mechanism intended to fabricate volume. Running the bot does not guarantee campaign eligibility or profit.

## Safety defaults

The checked-in configuration is deliberately safe:

- Aster **testnet**
- `dry_run = true`
- USD 10 per side
- USD 50 maximum absolute position
- USD 5 maximum unrealized loss
- `GTX` post-only orders
- per-order `stpMode=EXPIRE_BOTH`
- one-way position mode required
- startup and shutdown cleanup restricted to the configured client-order prefix
- no automatic market close when a risk limit is hit

A risk-limit stop cancels the bot's orders but leaves any existing position open for manual review.

## Implemented behavior

- Aster v3 EIP-712 signing with the API-wallet key
- monotonic microsecond nonce generation
- Aster server-time synchronization
- REST exchange-filter discovery (`tickSize`, `stepSize`, quantity and notional minimums)
- real-time `bookTicker` WebSocket with reconnect and stale-market protection
- one bid plus one ask, both `LIMIT + GTX`
- self-trade prevention via `EXPIRE_BOTH`
- inventory skew and per-side position-cap calculations
- maker-first inventory reduction through configurable, reduce-only `LIMIT + IOC` orders
- startup recovery by cancelling only bot-prefixed orders
- partial-fill detection and replenishment
- decoding of structured Aster API errors even when returned with HTTP 4xx
- explicit recovery for HTTP 503 **unknown execution state** on placement and cancellation
- session volume, maker/taker, commission and realized-PnL statistics restricted to orders placed by this process
- optional Aster exchange dead-man switch
- `--list-symbols`, `--check-auth`, and `--once` commands

## Requirements

- the current stable Rust toolchain
- an Aster Pro API Wallet / Agent for signed requests
- a system clock synchronized with NTP
- an Aster account using **one-way position mode**

Only EVM API-wallet signing is implemented in this version.

## Quick start

```bash
git clone https://github.com/lly835/aster.git
cd aster
cp config.example.toml config.toml
cp .env.example .env
```

Build and test:

```bash
cargo check --locked --all-targets
cargo test --locked --all-targets
```

### 1. Dry run

No credentials are required for dry-run mode:

```bash
cargo run --release -- --config config.toml
```

One quote calculation followed by shutdown:

```bash
cargo run --release -- --config config.toml --once
```

Dry run consumes public market data but never sends a signed order, cancel, account, or position request.

### 2. Find the exact API symbol

Do not guess the API symbol from the website label. Query `exchangeInfo`:

```bash
cargo run --release -- \
  --config config.toml \
  --list-symbols \
  --symbol-filter USD1
```

Then copy the exact symbol into `config.toml`:

```toml
symbol = "THE_EXACT_SYMBOL"
```

### 3. Configure credentials

Create an Aster **Pro API Wallet / Agent** with perpetual-trading permission. Put the API-wallet credentials in `.env`:

```dotenv
ASTER_USER_ADDRESS=0xYourMainAccountAddress
ASTER_SIGNER_ADDRESS=0xYourApiWalletAddress
ASTER_SIGNER_PRIVATE_KEY=0xYourApiWalletPrivateKey
RUST_LOG=info
```

`ASTER_SIGNER_PRIVATE_KEY` must belong to `ASTER_SIGNER_ADDRESS`.

Do not put the main/login-wallet private key in this project. `.env` and `config.toml` are ignored by Git.

Verify signed access without placing an order:

```bash
cargo run --release -- \
  --config config.toml \
  --check-auth
```

This checks server time, public market data, position mode, position access and open-order access.

### 4. Testnet live mode

Review every setting, then change:

```toml
environment = "testnet"
dry_run = false
live_trading_ack = "I_UNDERSTAND_LIVE_TRADING"
```

Start with a very small notional:

```toml
quote_notional_usd = "5"
max_position_notional_usd = "20"
max_unrealized_loss_usd = "2"
```

Run:

```bash
cargo run --release -- --config config.toml
```

### 5. Mainnet

Only after testnet behavior has been verified:

```toml
environment = "mainnet"
dry_run = false
live_trading_ack = "I_UNDERSTAND_LIVE_TRADING"
```

The mainnet defaults are:

```text
REST:      https://fapi.asterdex.com
WebSocket: wss://fstream.asterdex.com
EIP-712 signing chainId: 1666
```

The testnet defaults are:

```text
REST:      https://fapi.asterdex-testnet.com
WebSocket: wss://fstream.asterdex-testnet.com
EIP-712 signing chainId: 714
```

Both URLs can be overridden in `config.toml`, because Aster has used different testnet WebSocket hostnames in different documentation revisions.

## Configuration

```toml
environment = "testnet"
symbol = "BTCUSDT"

quote_notional_usd = "10"
max_position_notional_usd = "50"
max_unrealized_loss_usd = "5"

taker_rebalance_enabled = true
taker_rebalance_trigger_notional_usd = "30"
taker_rebalance_target_notional_usd = "5"
taker_rebalance_max_order_notional_usd = "20"
taker_rebalance_max_position_age_secs = 180
taker_rebalance_cooldown_secs = 30
taker_rebalance_max_slippage_bps = 5

refresh_ms = 1000
position_refresh_ms = 2000
stats_interval_secs = 10
stale_book_ms = 5000

bid_offset_ticks = 0
ask_offset_ticks = 0
inventory_skew_ticks = 3
requote_threshold_ticks = 1

client_order_prefix = "armm_"

dry_run = true
live_trading_ack = ""

# Cancel bot-prefixed orders left by an earlier process before starting.
# When false, startup fails instead of silently adopting or cancelling matching orders.
startup_cancel_existing_bot_orders = true
cancel_on_exit = true

deadman_switch_enabled = false
deadman_countdown_ms = 120000
deadman_heartbeat_ms = 30000
```

### Quote size

`quote_notional_usd` is the approximate quote-currency notional **per side**, not total account capital. The actual amount may be raised to satisfy the symbol's minimum quantity or minimum notional.

### Position cap

Before placing each side, the bot calculates the largest possible full-fill quantity that would keep the resulting position within `max_position_notional_usd`. A side is disabled when the remaining risk capacity is smaller than the exchange minimum.

### Inventory skew

At zero inventory, quotes begin at:

```text
buy  = best bid - bid_offset_ticks
sell = best ask + ask_offset_ticks
```

As long inventory grows, both quotes move downward. As short inventory grows, both quotes move upward. The maximum shift is `inventory_skew_ticks` at the configured position cap.

### Maker-first IOC inventory rebalancing

When explicitly enabled, the normal path remains post-only Maker quoting. The internal compatibility default is disabled, so an older `config.toml` that omits these fields will not silently begin sending Taker orders after an upgrade. The checked-in example opts in visibly. A Taker order is used only to reduce an existing net position when either:

- absolute position notional reaches `taker_rebalance_trigger_notional_usd`; or
- the position stays above `taker_rebalance_target_notional_usd` for `taker_rebalance_max_position_age_secs`.

Before the aggressive order, the bot cancels its tracked Maker quotes and refreshes the position. It then submits one opposite-side `LIMIT + IOC` order with `reduceOnly=true`. The order quantity moves the position toward the configured target, is capped by `taker_rebalance_max_order_notional_usd`, and can never intentionally increase or reverse exposure. `taker_rebalance_max_slippage_bps` bounds the worst acceptable limit price, while `taker_rebalance_cooldown_secs` prevents repeated aggressive orders in a tight loop.

If inventory still requires rebalancing during the configured cooldown, the bot pauses Maker quoting instead of reopening both sides and potentially increasing exposure.

This is inventory-risk control, not a target-volume or target-Maker/Taker-ratio engine. It may produce no Taker volume when inventory is naturally balanced, and it does not guarantee campaign points, the screenshot's 83.8%/16.2% split, or profitability.

### Requote threshold

The bot keeps an existing order unless its target changes by at least `requote_threshold_ticks`, its desired quantity changes by at least one `stepSize`, it becomes partially filled, or it is no longer `NEW`.

### Stale-market protection

If no valid WebSocket book update arrives within `stale_book_ms`, the bot cancels its tracked orders and pauses quoting until fresh data returns.

### Client-order prefix

The bot only treats an order as its own when `clientOrderId` starts with `client_order_prefix`. Use a unique prefix when multiple bot instances share an account.

### Dead-man switch warning

Aster's `countdownCancelAll` endpoint is **symbol-wide**. When enabled, it can cancel manual orders and orders created by other software on the same symbol. It is disabled by default.

## Unknown execution responses

Aster documents HTTP 503 as an unknown execution state: a request may have succeeded even though no final response was returned.

For order placement, this bot:

1. does not blindly retry;
2. queries the unique `clientOrderId` repeatedly;
3. adopts the recovered order when found;
4. halts when it cannot determine the result.

For cancellation, it queries the order first and retries only when the order is still open. If shutdown cleanup cannot be confirmed, an enabled exchange dead-man switch is deliberately left armed.

This behavior prefers an interruption over accidentally creating duplicate exposure or leaving an order live without protection.

## Operational limitations

- one symbol per process
- one-way position mode only
- EVM API-wallet signing only
- position and open-order reconciliation currently use REST polling; open orders are reconciled before the position snapshot used for replacement quotes
- no market orders; aggressive inventory reduction uses price-protected, reduce-only IOC limits
- no automatic full liquidation after an unrealized-loss stop
- no cross-exchange hedge
- no profitability or airdrop-reward guarantee
- user-trade statistics fetch at most ten 1,000-record pages per refresh

For production deployment, use a process supervisor, persistent metrics, alerting, a dedicated API wallet, IP restrictions where supported, and conservative account-level limits.
