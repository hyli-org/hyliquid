# Orderbook Load Testing with Goose

A comprehensive load testing tool for the Hyliquid orderbook system, built with [Goose](https://github.com/tag1consulting/goose) to simulate market maker behavior and generate realistic trading load.

## Features

- 🎯 **Market Maker Simulation**: Places bid/ask quote ladders around a dynamic mid price with random walk
- 🚀 **Taker Orders**: Crosses the spread to guarantee order executions
- 🧹 **Order Cancellation**: Periodic cleanup of old orders to prevent orderbook inflation
- 📊 **Comprehensive Metrics**: Latency distributions (P50/P95/P99), throughput, error rates
- ✅ **SLA Validation**: Configurable checks that fail the test if requirements aren't met
- 🔐 **Authentic Signature**: Reuses the exact signature/auth logic from the production CLI
- 🎲 **Deterministic RNG**: Reproducible tests with configurable seed
- 🔢 **Integer-based**: No floating-point arithmetic; all prices and quantities use integer scales

## Architecture

```
loadtest/
├── src/
│   ├── main.rs              # Entry point and Goose orchestration
│   ├── config.rs            # Configuration parsing (TOML + CLI + env)
│   ├── auth.rs              # User authentication and signing (from tx_sender)
│   ├── state.rs             # Shared state (RNG, order tracking, mid price)
│   ├── http_client.rs       # HTTP wrapper with proper headers and auth
│   ├── metrics.rs           # Metrics aggregation and export
│   ├── checks.rs            # SLA validation
│   └── scenarios/
│       ├── setup.rs         # User initialization (session key, deposits)
│       ├── maker.rs         # Market maker (quote ladders)
│       ├── taker.rs         # Taker (cross spread)
│       └── cancellation.rs  # Periodic order cancellation
├── Cargo.toml
├── loadtest.toml            # Default configuration
└── README.md                # This file
```

## Quick Start

### 1. Build

From the workspace root:

```bash
cargo build --release -p loadtest
```

Or from the `loadtest/` directory:

```bash
cargo build --release
```

### 2. Configure

Edit `loadtest.toml` or use environment variables/CLI flags. Key settings:

```toml
[server]
base_url = "http://localhost:9002"

[instrument]
base_asset = "BTC"
quote_asset = "USDT"

[load]
model = "closed"  # or "open"
users = 10        # Number of virtual users (closed model)
duration = 300    # Test duration in seconds
```

### 3. Run

```bash
# Using default configuration
./target/release/loadtest_goose

# With custom config file
./target/release/loadtest_goose --config my_config.toml

# Override via CLI flags
./target/release/loadtest_goose --users 20 --duration 600

# Override via environment variables
BASE_URL=http://localhost:9002 USERS=50 ./target/release/loadtest_goose

# Dry run (validate config without executing)
./target/release/loadtest_goose --dry-run

# Verbose output
./target/release/loadtest_goose --verbose
```

## Configuration

### Configuration Priority

Configuration is loaded with the following priority (highest to lowest):

1. **CLI flags** (e.g., `--users 20`)
2. **Environment variables** (e.g., `USERS=20`)
3. **TOML file** (e.g., `loadtest.toml`)

### TOML Configuration Reference

#### `[server]` - Server connection

```toml
[server]
base_url = "http://localhost:9002"  # Base URL of the orderbook API
```

#### `[instrument]` - Trading pair configuration

```toml
[instrument]
base_asset = "BTC"     # Base asset symbol
quote_asset = "USDT"   # Quote asset symbol
price_tick = 1         # Minimum price increment (integer)
qty_step = 1           # Minimum quantity increment (integer)
price_scale = 2        # Decimal places for price display
qty_scale = 8          # Decimal places for quantity display
```

**Important**: Prices and quantities are always represented as **integers** internally. For example:

- A price of `50000.50` with `price_scale=2` is stored as `5000050` (50000.50 \* 10^2)
- A quantity of `0.001` with `qty_scale=8` is stored as `100000` (0.001 \* 10^8)

#### `[load]` - Load model

```toml
[load]
model = "closed"               # "closed" (fixed users) or "open" (constant RPS)
users = 10                     # Number of virtual users (closed model)
rps = 50                       # Requests per second target (open model)
duration = 300                 # Test duration in seconds
ramp_users_per_second = 2      # Ramp-up rate (users/second)
ramp_duration = 30             # Ramp-up duration in seconds
```

**Load Models**:

- **Closed**: Fixed number of concurrent users executing scenarios in a loop
- **Open**: Constant arrival rate of requests (approximated in Goose)

#### `[maker]` - Market maker scenario

```toml
[maker]
enabled = true                 # Enable maker scenario
weight = 60                    # Relative weight (% of total load)
ladder_levels = 5              # Number of price levels per side
min_spread_ticks = 10          # Minimum distance from mid (in ticks)
level_spacing_ticks = 5        # Spacing between ladder levels (in ticks)
min_quantity_steps = 10        # Minimum order size (in steps)
max_quantity_steps = 100       # Maximum order size (in steps)
mid_drift_ticks = 2            # Max random drift per cycle (±ticks)
mid_initial = 50000            # Initial mid price (in ticks)
cycle_interval_ms = 2000       # Time between maker cycles (ms)
```

**How it works**:

- Applies a random walk to the mid price each cycle (±`mid_drift_ticks`)
- Places `ladder_levels` bid orders below mid and `ladder_levels` ask orders above mid
- Bid prices: `mid - (min_spread + n * spacing) * price_tick`
- Ask prices: `mid + (min_spread + n * spacing) * price_tick`
- Quantities: random in `[min_quantity_steps, max_quantity_steps] * qty_step`

#### `[taker]` - Taker scenario

```toml
[taker]
enabled = true                 # Enable taker scenario
weight = 30                    # Relative weight (% of total load)
cross_ticks = 1                # Ticks to cross best price (guarantees execution)
min_quantity_steps = 5         # Minimum order size
max_quantity_steps = 50        # Maximum order size
interval_ms = 5000             # Time between taker orders (ms)
```

**How it works**:

- Fetches the current orderbook (best bid/ask)
- Randomly chooses to buy or sell
- **Buy**: Places a bid at `best_ask + cross_ticks * price_tick` (crosses the ask)
- **Sell**: Places an ask at `best_bid - cross_ticks * price_tick` (crosses the bid)
- This **guarantees executions** by crossing the spread

#### `[cancellation]` - Cancellation scenario

```toml
[cancellation]
enabled = true                 # Enable cancellation scenario
weight = 10                    # Relative weight (% of total load)
cancel_percentage = 20         # Percentage of old orders to cancel per cycle
max_tracked_orders = 100       # Maximum orders to track
interval_ms = 10000            # Time between cancellation cycles (ms)
```

**How it works**:

- Tracks orders created by all users in a shared queue
- Every `interval_ms`, cancels the oldest `cancel_percentage`% of tracked orders
- Prevents orderbook inflation during long tests

#### `[http]` - HTTP client settings

```toml
[http]
timeout_ms = 5000              # Request timeout
connect_timeout_ms = 2000      # Connection timeout
max_retries = 3                # Max retries (not implemented yet)
retry_backoff_ms = 100         # Backoff between retries
max_requests_per_second = 0    # Client-side rate limit (0 = unlimited)
```

#### `[user_setup]` - Initial deposits

```toml
[user_setup]
initial_deposit_base = 1000000000    # Initial deposit of base asset (large amount)
initial_deposit_quote = 10000000000  # Initial deposit of quote asset (large amount)
```

**Note**: Each virtual user is initialized with these deposits to ensure they have sufficient balance for the test.

#### `[rng]` - Random number generator

```toml
[rng]
seed = 42  # Seed for deterministic tests (0 = random seed from entropy)
```

#### `[sla]` - Service Level Agreement checks

```toml
[sla]
enabled = true                 # Enable SLA validation
p50_max_ms = 50                # P50 latency must be <= 50ms
p95_max_ms = 100               # P95 latency must be <= 100ms
p99_max_ms = 200               # P99 latency must be <= 200ms
max_error_rate_percent = 1.0   # Error rate must be <= 1%
min_fills = 1                  # At least 1 successful execution
```

**Test fails if any SLA check fails.**

#### `[metrics]` - Metrics and reporting

```toml
[metrics]
export_json = true             # Export summary as JSON
export_csv = true              # Export latencies as CSV
output_dir = "./loadtest_results"  # Output directory
verbose = true                 # Verbose console output
```

### Environment Variables

All configuration can be overridden via environment variables:

| Variable      | Config Path              | Example                 |
| ------------- | ------------------------ | ----------------------- |
| `BASE_URL`    | `server.base_url`        | `http://localhost:9002` |
| `BASE_ASSET`  | `instrument.base_asset`  | `BTC`                   |
| `QUOTE_ASSET` | `instrument.quote_asset` | `USDT`                  |
| `USERS`       | `load.users`             | `20`                    |
| `RPS`         | `load.rps`               | `100`                   |
| `DURATION`    | `load.duration`          | `600`                   |
| `MODEL`       | `load.model`             | `closed`                |
| `SEED`        | `rng.seed`               | `12345`                 |
| `PRICE_TICK`  | `instrument.price_tick`  | `1`                     |
| `QTY_STEP`    | `instrument.qty_step`    | `1`                     |
| `REPORT_DIR`  | `metrics.output_dir`     | `./results`             |

### CLI Flags

```
OPTIONS:
    --config <PATH>          Path to configuration file [default: loadtest.toml]
    --base-url <URL>         Server base URL
    --base-asset <SYMBOL>    Base asset symbol
    --quote-asset <SYMBOL>   Quote asset symbol
    --users <N>              Number of virtual users
    --rps <N>                Requests per second (open model)
    --duration <SECONDS>     Test duration
    --seed <N>               RNG seed for reproducibility
    --model <MODEL>          Load model: closed or open
    --price-tick <N>         Price tick
    --qty-step <N>           Quantity step
    --report-dir <PATH>      Output directory for reports
    --dry-run                Validate config without running test
    --verbose, -v            Verbose output
```

## How It Works

### Authentication

The load testing tool reuses the **exact authentication logic** from the `tx_sender` CLI:

1. Each virtual user has a unique identity: `loadtest_user_{index}`
2. A keypair is derived deterministically from the identity using SHA3-256
3. For signed operations (create_order, cancel_order, withdraw), the signature format is:
   ```
   {identity}:{nonce}:{action}:{params}
   ```
4. The signature is created using ECDSA on the SHA3-256 hash of this data
5. Headers sent with each request:
   - `x-identity`: User identity string
   - `x-public-key`: Hex-encoded public key (65 bytes, uncompressed)
   - `x-signature`: Hex-encoded ECDSA signature (64 bytes)

### Test Flow

1. **Setup Phase** (per user):

   - Add session key (`POST /add_session_key`)
   - Create trading pair (`POST /create_pair`) - first user only
   - Deposit base asset (`POST /deposit`)
   - Deposit quote asset (`POST /deposit`)

2. **Execution Phase** (loop until duration expires):

   - **Maker**: Place quote ladders around dynamic mid price
   - **Taker**: Cross the spread to generate executions
   - **Cancellation**: Clean up old orders

3. **Teardown Phase**:
   - Collect metrics
   - Export to JSON/CSV
   - Validate SLA
   - Print summary

### Scenarios

#### Maker

- Updates mid price with random walk (±`mid_drift_ticks`)
- Places `ladder_levels` limit orders on each side
- Each level: `price = mid ± (spread + level * spacing) * tick`
- Quantity: random in `[min_qty, max_qty] * step`
- Waits `cycle_interval_ms` before repeating

#### Taker

- Fetches orderbook (`GET /api/book/{base}/{quote}`)
- Randomly buys or sells
- Crosses the spread by `cross_ticks` to guarantee execution
- Waits `interval_ms` before next order

#### Cancellation

- Retrieves oldest `cancel_percentage`% of tracked orders
- Cancels each order (`POST /cancel_order`)
- Waits `interval_ms` before next cycle

## Metrics & Reports

After each test run, metrics are exported to `metrics.output_dir` (default: `./loadtest_results/`):

### `summary.json`

Complete test summary including:

```json
{
  "test_start": "2024-01-01T12:00:00Z",
  "test_duration_secs": 300.0,
  "total_requests": 5000,
  "successful_requests": 4950,
  "failed_requests": 50,
  "requests_per_second": 16.67,
  "error_rate_percent": 1.0,
  "latencies": {
    "min_ms": 5,
    "max_ms": 150,
    "mean_ms": 42.5,
    "p50_ms": 40,
    "p90_ms": 75,
    "p95_ms": 90,
    "p99_ms": 120
  },
  "endpoints": [...]
}
```

### `latencies.csv`

Per-endpoint latency data:

```csv
timestamp,endpoint,status_code,latency_ms
2024-01-01T12:00:00Z,create_order,200,45
2024-01-01T12:00:01Z,cancel_order,200,32
...
```

### Console Output

At the end of the test, a human-readable summary is printed:

```
================================================================================
📊 LOAD TEST SUMMARY
================================================================================

⏱️  Test Duration: 300.00s
📈 Total Requests: 5000
✅ Successful: 4950
❌ Failed: 50
📊 RPS: 16.67
💢 Error Rate: 1.00%

🚀 LATENCY METRICS (milliseconds)
--------------------------------------------------------------------------------
  Min:  5ms
  Mean: 42.50ms
  P50:  40ms
  P90:  75ms
  P95:  90ms
  P99:  120ms
  Max:  150ms

✅ SLA VALIDATION PASSED
  ✓ P50 latency: 40ms <= 50ms
  ✓ P95 latency: 90ms <= 100ms
  ✓ P99 latency: 120ms <= 200ms
  ✓ Error rate: 1.00% <= 1.00%
  ✓ Minimum fills: 100 >= 1
================================================================================
```

## SLA Validation

The tool can automatically fail the test if SLA requirements are not met. Configure thresholds in `[sla]`:

```toml
[sla]
enabled = true
p50_max_ms = 50
p95_max_ms = 100
p99_max_ms = 200
max_error_rate_percent = 1.0
min_fills = 1
```

If any threshold is exceeded, the test exits with code 1.

## Examples

### Basic Load Test (10 users, 5 minutes)

```bash
./target/release/loadtest_goose \
  --users 10 \
  --duration 300 \
  --base-url http://localhost:9002
```

### High-Throughput Test (Open Model, 100 RPS)

```bash
./target/release/loadtest_goose \
  --model open \
  --rps 100 \
  --duration 600
```

### Stress Test with Tight SLA

Edit `loadtest.toml`:

```toml
[load]
model = "closed"
users = 50
duration = 1800  # 30 minutes

[sla]
enabled = true
p95_max_ms = 50  # Strict P95
max_error_rate_percent = 0.1  # Very low error tolerance
min_fills = 100  # Expect at least 100 executions
```

```bash
./target/release/loadtest_goose
```

### Deterministic Test (Reproducible)

```bash
./target/release/loadtest_goose --seed 42
```

### Custom Instrument

```bash
./target/release/loadtest_goose \
  --base-asset ETH \
  --quote-asset USDC \
  --price-tick 10 \
  --qty-step 1000
```

## Troubleshooting

### High Error Rate

- **Insufficient balance**: Increase `initial_deposit_base` and `initial_deposit_quote`
- **Rate limiting**: Reduce `users` or `rps`, or increase `cycle_interval_ms`
- **Server overload**: Check server logs and reduce load

### No Executions

- **Maker and taker not enabled**: Ensure both `maker.enabled` and `taker.enabled` are `true`
- **Taker interval too high**: Reduce `taker.interval_ms`
- **Orderbook empty**: Check that maker is placing orders successfully

### Nonce Errors

- Each user maintains its own nonce counter
- The nonce is fetched from the server before each signed operation
- If nonce errors persist, check server-side nonce management

### Connection Timeouts

- Increase `http.timeout_ms` and `http.connect_timeout_ms`
- Check network connectivity to server
- Verify server is running and accessible

## Advanced Usage

### Running Against Multiple Environments

```bash
# Development
BASE_URL=http://localhost:9002 ./target/release/loadtest_goose

# Staging
BASE_URL=http://staging.example.com ./target/release/loadtest_goose

# Production (NOT RECOMMENDED)
# The tool will warn you if the URL contains "prod" or "production"
```

### Custom Scenario Weights

Adjust the relative frequency of each scenario:

```toml
[maker]
weight = 70  # 70% of operations are maker

[taker]
weight = 20  # 20% are taker

[cancellation]
weight = 10  # 10% are cancellations
```

### Disabling Scenarios

```toml
[maker]
enabled = false  # No maker orders

[taker]
enabled = true   # Only taker orders

[cancellation]
enabled = false  # No cancellations
```

## Development

### Running Tests

```bash
cargo test
```

### Running with Debug Logs

```bash
RUST_LOG=debug ./target/release/loadtest_goose --verbose
```

### Project Structure

- `auth.rs`: Signature creation (SHA3 + ECDSA)
- `config.rs`: Configuration parsing and validation
- `state.rs`: Shared state (RNG, order tracker, mid price)
- `http_client.rs`: HTTP wrapper with Goose integration
- `scenarios/*.rs`: Goose transaction scenarios
- `metrics.rs`: Metrics aggregation and export
- `checks.rs`: SLA validation
- `main.rs`: Orchestration and Goose setup

## License

This project is part of the Hyliquid workspace.

## Support

For issues or questions, please contact the Hyliquid team or file an issue in the repository.
