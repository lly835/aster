mod api;
mod bot;
mod config;

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use api::{run_book_ticker_stream, AsterClient};
use bot::MarketMaker;
use clap::Parser;
use config::{credentials_from_env, load};
use tokio::sync::watch;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(
    name = "aster-rust-market-maker",
    version,
    about = "Conservative post-only market maker for Aster Futures API v3"
)]
struct Cli {
    /// Path to the TOML configuration file.
    #[arg(long, default_value = "config.toml")]
    config: PathBuf,

    /// Run one quote cycle and exit. In live mode, any bot order is cancelled during shutdown.
    #[arg(long)]
    once: bool,

    /// Print currently tradable symbols and exit.
    #[arg(long)]
    list_symbols: bool,

    /// Optional case-insensitive substring used with --list-symbols, for example USD1.
    #[arg(long)]
    symbol_filter: Option<String>,

    /// Verify credentials, server time, position mode, position and open-order access, then exit.
    #[arg(long)]
    check_auth: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    init_tracing();

    let cli = Cli::parse();
    let config = load(&cli.config)?;
    let credentials = credentials_from_env()?;

    if !config.dry_run && credentials.is_none() {
        bail!(
            "live mode requires ASTER_USER_ADDRESS, ASTER_SIGNER_ADDRESS, and ASTER_SIGNER_PRIVATE_KEY"
        );
    }

    let client = AsterClient::new(
        config.rest_base_url.clone(),
        config.environment.chain_id(),
        credentials,
    )
    .context("failed to initialize Aster client")?;

    if cli.list_symbols {
        let symbols = client
            .list_symbols(cli.symbol_filter.as_deref())
            .await
            .context("failed to list Aster symbols")?;
        if symbols.is_empty() {
            warn!("no tradable symbols matched the requested filter");
        } else {
            for symbol in symbols {
                println!("{symbol}");
            }
        }
        return Ok(());
    }

    if cli.check_auth {
        if !client.has_credentials() {
            bail!("--check-auth requires Aster API credentials in the environment");
        }

        client
            .sync_server_time()
            .await
            .context("server-time check failed")?;
        let book = client
            .book_ticker(&config.symbol)
            .await
            .context("public book-ticker check failed")?;
        let hedge_mode = client
            .is_hedge_mode()
            .await
            .context("position-mode check failed")?;
        let mid = (book.bid + book.ask) / rust_decimal::Decimal::from(2_u32);
        let position = client
            .position(&config.symbol, mid)
            .await
            .context("position check failed")?;
        let open_orders = client
            .open_orders(&config.symbol)
            .await
            .context("open-order check failed")?;

        println!("Authentication successful");
        println!("symbol: {}", config.symbol);
        println!("hedge mode: {hedge_mode}");
        println!("position quantity: {}", position.quantity);
        println!("mark price: {}", position.mark_price);
        println!("unrealized PnL: {}", position.unrealized_pnl);
        println!("open orders on symbol: {}", open_orders.len());
        return Ok(());
    }

    let rules = client
        .fetch_symbol_rules(&config.symbol)
        .await
        .with_context(|| {
            format!(
                "failed to load exchange filters for {}",
                config.symbol
            )
        })?;
    let initial_book = client
        .book_ticker(&config.symbol)
        .await
        .with_context(|| {
            format!(
                "failed to load initial book ticker for {}",
                config.symbol
            )
        })?;

    let (book_tx, book_rx) = watch::channel(Some(initial_book.clone()));
    let websocket_task = tokio::spawn(run_book_ticker_stream(
        config.ws_base_url.clone(),
        config.symbol.clone(),
        book_tx,
    ));

    info!(
        best_bid = %initial_book.bid,
        best_ask = %initial_book.ask,
        "loaded initial Aster book ticker"
    );

    let bot = MarketMaker::new(
        client,
        config,
        rules,
        book_rx,
        &initial_book,
    );
    let result = bot.run(cli.once).await;

    websocket_task.abort();
    let _ = websocket_task.await;

    result
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .init();
}
