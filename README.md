# Charon-RS

A Solana/Pump.fun trading bot written in Rust — a rewrite of the [yunus-0x/charon](https://github.com/yunus-0x/charon) Node.js project.

## Features

- **Multi-signal detection**: Fee claims, graduated tokens, GMGN/Jupiter trending, price dips
- **Strategy-based filtering**: 15+ configurable filter gates (source count, market cap, holders, etc.)
- **LLM-powered screening**: Optional AI candidate selection via OpenAI-compatible endpoints
- **Three execution modes**: `dry_run` → `confirm` → `live`
- **Position management**: Take-profit, stop-loss, and trailing stops
- **Telegram integration**: Commands, alerts, and trade confirmations
- **Post-trade learning**: Automatic lesson generation from closed positions

## Architecture

```
Signal Ingestion → Strategy Gates → Enrichment → LLM Screening → Execution → Position Monitor
     ↓                  ↓              ↓              ↓              ↓              ↓
  fee_claim        min_sources    Jupiter API    MiniMax/GPT    Jupiter Ultra    TP/SL/Trailing
  graduated        market_cap     GMGN API       confidence     Solana SDK       checks
  trending         holders        Twitter                       dry_run/confirm
  price_dip        top_holder                                   live modes
```

## Quick Start

### Prerequisites

- Rust 1.75+ (with `cargo`)
- SQLite3
- A Solana wallet keypair (base58-encoded)
- A Telegram bot token

### Build

```bash
# Standard build (no live trading)
cargo build --release

# With live trading enabled
cargo build --release --features live-trading
```

### Configure

```bash
cp .env.example .env
# Edit .env with your keys and configuration
```

Key environment variables:

| Variable | Required | Description |
|----------|----------|-------------|
| `SIGNAL_SERVER_KEY` | Yes | Charon signal server API key |
| `SOLANA_PRIVATE_KEY` | Yes | Base58-encoded Solana keypair |
| `TELEGRAM_BOT_TOKEN` | Yes | Telegram bot token |
| `TELEGRAM_CHAT_ID` | Yes | Your Telegram chat ID |
| `TRADING_MODE` | No | `dry_run` (default), `confirm`, or `live` |
| `ENABLE_LLM` | No | Enable LLM screening (default: false) |
| `LLM_API_KEY` | No | Required if ENABLE_LLM=true |

### Run

```bash
cargo run --release
```

## Telegram Commands

| Command | Description |
|---------|-------------|
| `/menu` | Show the main menu |
| `/strategy` | View current strategy settings |
| `/stratset <field> <value>` | Update a strategy parameter |
| `/positions` | List open positions |
| `/candidates` | Show approved candidates |
| `/status` | Bot status and wallet info |
| `/lessons` | Recent trade lessons |
| `/reload` | Hot-reload strategy from database |
| `/help` | Show help |

## Strategy Configuration

Strategies are stored in SQLite and can be hot-reloaded without restart. Configurable fields:

- `min_source_count` — Minimum signal source overlap (default: 2)
- `min_fee_claims` — Minimum fee claim count (default: 1)
- `min_market_cap_sol` / `max_market_cap_sol` — Market cap range
- `min_holders` — Minimum holder count (default: 50)
- `max_top_holder_pct` — Maximum top holder concentration (default: 30%)
- `buy_sol` — Buy amount in SOL (default: 0.1)
- `tp_percent` — Take-profit percentage (default: 100%)
- `sl_percent` — Stop-loss percentage (default: 30%)

Example:
```
/stratset buy_sol 0.2
/stratset tp_percent 75
/reload
```

## Project Structure

```
src/
├── main.rs                 # Entry point, async runtime setup
├── config/                 # Configuration loading & validation
├── db/                     # SQLite layer (schema, models, repos)
├── signals/                # Signal sources (HTTP, WebSocket)
├── pipeline/               # Processing pipeline (filter, LLM, orchestration)
├── enrichment/             # Token data enrichment (Jupiter, GMGN, Twitter)
├── execution/              # Trade execution (Jupiter swaps, wallet, positions)
├── telegram/               # Telegram bot (commands, formatting)
├── learning/               # Post-trade analysis
├── utils/                  # Time helpers, retry logic
└── error.rs                # Unified error types
```

## Execution Modes

1. **`dry_run`** — No real transactions. Positions are tracked in SQLite with simulated entries. Safe for testing.
2. **`confirm`** — Candidates are sent to Telegram for manual approval before execution. Requires interactive confirmation.
3. **`live`** — Automatic execution via Jupiter Ultra swaps. Requires `--features live-trading` at compile time for safety.

## Migration from Node.js

| Node.js (Original) | Rust (This Project) |
|--------------------|--------------------|
| `better-sqlite3` | `rusqlite` + `r2d2` pool |
| `@solana/web3.js` v1 | `solana-sdk` / `solana-client` 1.18 |
| `node-telegram-bot-api` | `teloxide` |
| `axios` | `reqwest` |
| `ws` | `tokio-tungstenite` |
| Dynamic objects | Typed structs with `serde` |
| Callbacks / events | `tokio` channels + `async/await` |
| No compile-time checks | Feature flags for live trading safety |

The SQLite schema is compatible with the original Node.js version, allowing you to point `DATABASE_PATH` at an existing database.

## Safety

- **Live trading is behind a compile-time feature flag** — you must explicitly opt in with `--features live-trading`
- **Wallet balance checks** before every swap execution
- **Slippage protection** via Jupiter Ultra (configurable `SLIPPAGE_BPS`)
- **Minimum SOL reserve** to prevent draining the wallet
- **All database queries use parameterized statements** — no SQL injection risk

## License

MIT
