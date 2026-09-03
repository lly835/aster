use std::{env, fs, path::Path, str::FromStr};

use anyhow::{bail, Context, Result};
use reqwest::Url;
use rust_decimal::Decimal;
use serde::Deserialize;

const LIVE_ACK: &str = "I_UNDERSTAND_LIVE_TRADING";

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Environment {
    Testnet,
    Mainnet,
}

impl Environment {
    pub fn chain_id(self) -> u64 {
        match self {
            Self::Testnet => 714,
            Self::Mainnet => 1666,
        }
    }

    fn default_rest_url(self) -> &'static str {
        match self {
            Self::Testnet => "https://fapi.asterdex-testnet.com",
            Self::Mainnet => "https://fapi.asterdex.com",
        }
    }

    fn default_ws_url(self) -> &'static str {
        match self {
            Self::Testnet => "wss://fstream.asterdex-testnet.com",
            Self::Mainnet => "wss://fstream.asterdex.com",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FileConfig {
    environment: Environment,
    rest_base_url: Option<String>,
    ws_base_url: Option<String>,

    symbol: String,
    quote_notional_usd: String,
    max_position_notional_usd: String,
    max_unrealized_loss_usd: String,

    taker_rebalance_enabled: bool,
    taker_rebalance_trigger_notional_usd: String,
    taker_rebalance_target_notional_usd: String,
    taker_rebalance_max_order_notional_usd: String,
    taker_rebalance_max_position_age_secs: u64,
    taker_rebalance_cooldown_secs: u64,
    taker_rebalance_max_slippage_bps: u64,

    refresh_ms: u64,
    position_refresh_ms: u64,
    stats_interval_secs: u64,
    stale_book_ms: u64,

    bid_offset_ticks: u64,
    ask_offset_ticks: u64,
    inventory_skew_ticks: u64,
    requote_threshold_ticks: u64,

    client_order_prefix: String,
    dry_run: bool,
    live_trading_ack: String,
    startup_cancel_existing_bot_orders: bool,
    cancel_on_exit: bool,

    deadman_switch_enabled: bool,
    deadman_countdown_ms: u64,
    deadman_heartbeat_ms: u64,
}

impl Default for FileConfig {
    fn default() -> Self {
        Self {
            environment: Environment::Testnet,
            rest_base_url: None,
            ws_base_url: None,

            symbol: "BTCUSDT".to_owned(),
            quote_notional_usd: "10".to_owned(),
            max_position_notional_usd: "50".to_owned(),
            max_unrealized_loss_usd: "5".to_owned(),

            taker_rebalance_enabled: true,
            taker_rebalance_trigger_notional_usd: "30".to_owned(),
            taker_rebalance_target_notional_usd: "5".to_owned(),
            taker_rebalance_max_order_notional_usd: "20".to_owned(),
            taker_rebalance_max_position_age_secs: 180,
            taker_rebalance_cooldown_secs: 30,
            taker_rebalance_max_slippage_bps: 5,

            refresh_ms: 1_000,
            position_refresh_ms: 2_000,
            stats_interval_secs: 10,
            stale_book_ms: 5_000,

            bid_offset_ticks: 0,
            ask_offset_ticks: 0,
            inventory_skew_ticks: 3,
            requote_threshold_ticks: 1,

            client_order_prefix: "armm_".to_owned(),
            dry_run: true,
            live_trading_ack: String::new(),
            startup_cancel_existing_bot_orders: true,
            cancel_on_exit: true,

            deadman_switch_enabled: false,
            deadman_countdown_ms: 120_000,
            deadman_heartbeat_ms: 30_000,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub environment: Environment,
    pub rest_base_url: String,
    pub ws_base_url: String,

    pub symbol: String,
    pub quote_notional_usd: Decimal,
    pub max_position_notional_usd: Decimal,
    pub max_unrealized_loss_usd: Decimal,

    pub taker_rebalance_enabled: bool,
    pub taker_rebalance_trigger_notional_usd: Decimal,
    pub taker_rebalance_target_notional_usd: Decimal,
    pub taker_rebalance_max_order_notional_usd: Decimal,
    pub taker_rebalance_max_position_age_secs: u64,
    pub taker_rebalance_cooldown_secs: u64,
    pub taker_rebalance_max_slippage_bps: u64,

    pub refresh_ms: u64,
    pub position_refresh_ms: u64,
    pub stats_interval_secs: u64,
    pub stale_book_ms: u64,

    pub bid_offset_ticks: u64,
    pub ask_offset_ticks: u64,
    pub inventory_skew_ticks: u64,
    pub requote_threshold_ticks: u64,

    pub client_order_prefix: String,
    pub dry_run: bool,
    pub startup_cancel_existing_bot_orders: bool,
    pub cancel_on_exit: bool,

    pub deadman_switch_enabled: bool,
    pub deadman_countdown_ms: u64,
    pub deadman_heartbeat_ms: u64,
}

#[derive(Debug, Clone)]
pub struct Credentials {
    pub user_address: String,
    pub signer_address: String,
    pub signer_private_key: String,
}

pub fn load(path: &Path) -> Result<RuntimeConfig> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read config file {}", path.display()))?;
    let file: FileConfig =
        toml::from_str(&raw).with_context(|| format!("invalid TOML in {}", path.display()))?;

    let quote_notional_usd =
        parse_positive_decimal("quote_notional_usd", &file.quote_notional_usd)?;
    let max_position_notional_usd =
        parse_positive_decimal("max_position_notional_usd", &file.max_position_notional_usd)?;
    let max_unrealized_loss_usd =
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
        bail!("symbol cannot be empty");
    }
    if file.refresh_ms < 250 {
        bail!("refresh_ms must be at least 250 ms");
    }
    if file.position_refresh_ms < file.refresh_ms {
        bail!("position_refresh_ms must be greater than or equal to refresh_ms");
    }
    if file.stats_interval_secs == 0 {
        bail!("stats_interval_secs must be greater than zero");
    }
    if file.stale_book_ms <= file.refresh_ms {
        bail!("stale_book_ms must be greater than refresh_ms");
    }
    if file.requote_threshold_ticks == 0 {
        bail!("requote_threshold_ticks must be at least 1");
    }
    if max_position_notional_usd < quote_notional_usd {
        bail!("max_position_notional_usd must be at least quote_notional_usd");
    }
    if file.taker_rebalance_enabled {
        if taker_rebalance_target_notional_usd >= taker_rebalance_trigger_notional_usd {
            bail!(
                "taker_rebalance_target_notional_usd must be smaller than taker_rebalance_trigger_notional_usd"
            );
        }
        if taker_rebalance_trigger_notional_usd > max_position_notional_usd {
            bail!("taker_rebalance_trigger_notional_usd must not exceed max_position_notional_usd");
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

    if !file.dry_run && file.live_trading_ack != LIVE_ACK {
        bail!(
            "live trading is disabled: set live_trading_ack = \"{}\" after reviewing the configuration",
            LIVE_ACK
        );
    }

    if file.deadman_switch_enabled {
        if file.dry_run {
            bail!("deadman_switch_enabled has no effect in dry-run mode; disable it");
        }
        if !file.cancel_on_exit {
            bail!("deadman_switch_enabled requires cancel_on_exit = true");
        }
        if file.deadman_countdown_ms < 10_000 {
            bail!("deadman_countdown_ms must be at least 10000");
        }
        if file.deadman_heartbeat_ms == 0 || file.deadman_heartbeat_ms >= file.deadman_countdown_ms
        {
            bail!(
                "deadman_heartbeat_ms must be greater than zero and smaller than deadman_countdown_ms"
            );
        }
        if file.deadman_countdown_ms < file.deadman_heartbeat_ms.saturating_mul(2) {
            bail!("deadman_countdown_ms must be at least twice deadman_heartbeat_ms");
        }
    }

    let rest_base_url = file
        .rest_base_url
        .unwrap_or_else(|| file.environment.default_rest_url().to_owned())
        .trim_end_matches('/')
        .to_owned();
    let ws_base_url = file
        .ws_base_url
        .unwrap_or_else(|| file.environment.default_ws_url().to_owned())
        .trim_end_matches('/')
        .to_owned();

    validate_base_url("rest_base_url", &rest_base_url, "https")?;
    validate_base_url("ws_base_url", &ws_base_url, "wss")?;

    Ok(RuntimeConfig {
        environment: file.environment,
        rest_base_url,
        ws_base_url,

        symbol: file.symbol.trim().to_ascii_uppercase(),
        quote_notional_usd,
        max_position_notional_usd,
        max_unrealized_loss_usd,

        taker_rebalance_enabled: file.taker_rebalance_enabled,
        taker_rebalance_trigger_notional_usd,
        taker_rebalance_target_notional_usd,
        taker_rebalance_max_order_notional_usd,
        taker_rebalance_max_position_age_secs: file.taker_rebalance_max_position_age_secs,
        taker_rebalance_cooldown_secs: file.taker_rebalance_cooldown_secs,
        taker_rebalance_max_slippage_bps: file.taker_rebalance_max_slippage_bps,

        refresh_ms: file.refresh_ms,
        position_refresh_ms: file.position_refresh_ms,
        stats_interval_secs: file.stats_interval_secs,
        stale_book_ms: file.stale_book_ms,

        bid_offset_ticks: file.bid_offset_ticks,
        ask_offset_ticks: file.ask_offset_ticks,
        inventory_skew_ticks: file.inventory_skew_ticks,
        requote_threshold_ticks: file.requote_threshold_ticks,

        client_order_prefix: file.client_order_prefix,
        dry_run: file.dry_run,
        startup_cancel_existing_bot_orders: file.startup_cancel_existing_bot_orders,
        cancel_on_exit: file.cancel_on_exit,

        deadman_switch_enabled: file.deadman_switch_enabled,
        deadman_countdown_ms: file.deadman_countdown_ms,
        deadman_heartbeat_ms: file.deadman_heartbeat_ms,
    })
}

pub fn credentials_from_env() -> Result<Option<Credentials>> {
    let user = env::var("ASTER_USER_ADDRESS").ok();
    let signer = env::var("ASTER_SIGNER_ADDRESS").ok();
    let private_key = env::var("ASTER_SIGNER_PRIVATE_KEY").ok();

    let supplied = [
        user.as_ref()
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false),
        signer
            .as_ref()
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false),
        private_key
            .as_ref()
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false),
    ];

    if !supplied.iter().any(|value| *value) {
        return Ok(None);
    }
    if !supplied.iter().all(|value| *value) {
        bail!(
            "API credentials are incomplete; set ASTER_USER_ADDRESS, ASTER_SIGNER_ADDRESS, and ASTER_SIGNER_PRIVATE_KEY together"
        );
    }

    Ok(Some(Credentials {
        user_address: user.expect("checked").trim().to_owned(),
        signer_address: signer.expect("checked").trim().to_owned(),
        signer_private_key: private_key.expect("checked").trim().to_owned(),
    }))
}

fn validate_base_url(name: &str, raw: &str, expected_scheme: &str) -> Result<()> {
    let url = Url::parse(raw).with_context(|| format!("{name} must be a valid URL"))?;
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
    let parsed =
        Decimal::from_str(value).with_context(|| format!("{name} must be a decimal string"))?;
    if parsed <= Decimal::ZERO {
        bail!("{name} must be greater than zero");
    }
    Ok(parsed)
}

fn parse_nonnegative_decimal(name: &str, value: &str) -> Result<Decimal> {
    let parsed =
        Decimal::from_str(value).with_context(|| format!("{name} must be a decimal string"))?;
    if parsed < Decimal::ZERO {
        bail!("{name} must not be negative");
    }
    Ok(parsed)
}

fn validate_client_order_prefix(prefix: &str) -> Result<()> {
    if prefix.is_empty() || prefix.len() > 12 {
        bail!("client_order_prefix must contain 1 to 12 characters");
    }
    if !prefix
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        bail!("client_order_prefix may contain only ASCII letters, digits, '_' and '-'");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{validate_base_url, validate_client_order_prefix};

    #[test]
    fn validates_client_order_prefix() {
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
}
