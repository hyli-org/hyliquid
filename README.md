# Hyliquid: private proof-powered orderbook on Hyli

<div align="center">
  <a href="https://hyli.org/">
    <img src="https://github.com/hyli-org/hyli-assets/blob/main/Logos/Logo/HYLI_WORDMARK_ORANGE.png?raw=true" width="320" alt="Hyli">
  </a>

_**Hyliquid is a private, performance, non-custodial trading system built on [Hyli](https://hyli.org)**._

[![Telegram Chat][tg-badge]][tg-url]
[![Twitter][twitter-badge]][twitter-url]

</div>

**Hyliquid** uses Hyli's purpose-built stack to ship a private, performant, non-custodial trading system.

The project is fully open source and auditable for a perfect mix of privacy and compliance.

Hyliquid follows the same high-level pattern as [Lighter](https://assets.lighter.xyz/whitepaper.pdf) or dYdX on [StarkEx](https://docs.starkware.co/starkex/index.html) but removes the black boxes: every component is auditable and reproducible.

## Key features

- 🕶️ Users, balances, and fills stay offchain and private
- ⚡ Sub-second UX: instant interactions while proofs run asynchronously
- 🔐 Non-custodial by design
- 🧱 All components are auditable, open-source, without black boxes

## Why Hyli?

- Hyli natively verifies proofs, including the SP1 proofs used for Hyliquid.
- The [Hylix](https://crates.io/crates/hylix) toolsuite and SDKs makes it easy to deploy & test on a devnet.
- **zkVM-native from day one** – Hyli bakes proof-friendly concepts (lanes, contract identities, blob transactions) directly into its node APIs. We never had to fight the runtime to integrate SP1 proofs.
- **Purpose-built tooling** – The `hy` CLI, module system, and SDKs handle key management, block subscriptions, and proof submission out of the box, reducing boilerplate on the server side.
- **RISC-V everywhere** – Contracts compile to SP1’s RISC-V target, and the same artifacts are consumed by both the fast path and the prover. Developer environments stay portable.
- **No trade-offs** – Hyliquid stays private (no private data published onchain), performant (fast-path execution in Rust), and non-custodial (proofs anchor state on-chain). We never had to pick only two of the three.

## Architecture at a Glance

```bash
Users ──> Frontend ──> server/ (fast path)
                         │
                         │ writes deltas + prover jobs
                         ▼
                    Postgres queue
                         │
                         ▼
                   server/src/prover.rs (async SP1 proving)
                         │
                         ▼
                     Hyli network (settlement)
```

Key ideas:

1. **Fast path server** executes contract logic deterministically in Rust for instant UX, then persists the resulting state change and an `OrderbookProverRequest`.
2. **Asynchronous prover** replays the same request inside SP1, generates the proof, and submits a `ProofTransaction` back to Hyli.
3. **Read-optimized API + frontend** consume the canonical state directly from the database, decoupled from proving latency.

## Component Breakdown

### `contracts/` – The Hyli ZKContract

- The `orderbook` crate defines `ORDERBOOK_ACCOUNT_IDENTITY`, event schemas, and the full transition logic for deposits, order placement, matching, and withdrawals.
- SP1 compiles this contract to RISC-V ELF artifacts (`elf/orderbook`, `elf/orderbook_vk`), which are embedded into both the fast path and the prover.
- Because the same code drives the on-chain state transition and the prover replay, we avoid “shadow logic” bugs.

### `server/` – Fast Path + Database Writer

- `server/src/app.rs` exposes Axum handlers for `deposit`, `create_order`, `cancel_order`, `withdraw`, and `add_session_key`.
- Each handler executes the contract logic locally (using the same state structs as the contract), emits events, and pushes a `DatabaseRequest::WriteEvents` message onto the message bus.
- The database module persists both the serialized blob transaction and the `OrderbookProverRequest`, which contains everything the prover needs: user info, events, action metadata, and nonce.
- This process gives users immediate confirmation and a consistent state snapshot without waiting for a proof to finish.

### `server/src/prover.rs` – Async SP1 Prover

- `OrderbookProverModule` subscribes to `NodeStateEvent::NewBlock` updates via Hyli’s message bus.
- For every new block, it filters transactions that belong to the orderbook’s lane, reloads the corresponding `OrderbookProverRequest` from Postgres, and reconstructs the zkVM context.
- `handle_prover_request` recreates the commitment metadata and calldata (including `ORDERBOOK_ACCOUNT_IDENTITY` blobs) before dispatching `ClientSdkProver::prove`.
- Proof generation happens in detached `tokio::spawn` tasks, ensuring the module keeps up with the block feed. Successful proofs are wrapped into `ProofTransaction`s and submitted via `node_client.send_tx_proof`.
- Settled transactions are deleted from `prover_requests`, keeping the queue lean.

### `server-api/` – Read-Only Surface

- Built with Bun + TypeScript, this service exposes orderbook depth, user portfolio, and trade history endpoints by reading directly from the persisted state tables.
- Because it never mutates state, we can scale it horizontally or cache aggressively without risking stale proofs.

### `front/` – User Interface

- A Vite/Bun frontend that consumes both Hyli RPCs and the read API.
- Users authenticate with Hyli identities, submit signed payloads, and receive instant feedback sourced from the fast-path state.

### `loadtest/` – Reality Checks

- Goose-based scenarios (`maker.rs`, `taker.rs`, etc.) validate throughput on real HTTP flows.

## Privacy + Performance + Non-Custodial (All at Once)

- **Privacy** – Users’ secrets (orders, balances, session keys) stay inside the secured server. Those are never exposed onchain.
- **Performance** – Users see sub-second confirmations because the fast path executes Rust structs, not an on-chain VM. Asynchronous proving amortizes zk costs without blocking UX.
- **Non-custodial** – Final settlement always happens on Hyli. The backend cannot forge state transitions because every change must be proven against the SP1 contract before it becomes canonical.

This trifecta is typically a compromise on other stacks. Hyli’s architecture lets us keep all three without bolting on custom infra.

## End-to-End Flow

1. **User action** – A trader submits an authenticated request via the frontend. Headers include `x-identity`, `x-public-key`, and `x-signature`, which `AuthHeaders::from_headers` validates before processing.
2. **Fast path execution** – The corresponding handler in `server/src/app.rs` locks the in-memory orderbook state, applies the action (deposit/order/cancel/withdraw), emits events, and updates the state snapshot.
3. **Persistence + job enqueue** – The handler writes a `BlobTransaction` plus `OrderbookProverRequest` to Postgres. This captures the full replay context (events, nonce, user info, private input).
4. **Block detection** – `OrderbookProverModule` listens to Hyli blocks, filters transactions that reference the orderbook’s lane, and batches the associated pending jobs.
5. **Proof generation** – For each pending job, the prover rehydrates the full `FullState`, derives commitment metadata, and calls `ClientSdkProver::prove`, which executes the SP1 zkVM.
6. **Submission + cleanup** – Once the proof returns, the module builds a `ProofTransaction` and sends it via `node_client.send_tx_proof`. Settled transactions are removed from the queue.
7. **Read APIs + UI updates** – The frontend polls `server-api/` to show the latest depth chart, fills, and balances—the same data the prover replays—so UX stays in sync with provable state.

## Developer Experience

- **Single source of truth** – The contract logic (`orderbook` crate) is imported directly by both the server and the prover, so there is zero mismatch between “simulated” and “proved” behavior.
- **Module system** – Hyli’s `module_bus_client!` macros give us typed channels between the router, database, and prover. No ad-hoc Kafka, no extra RPC tier.
- **Observability** – `tracing` is wired through the server and prover, and we can export Perfetto traces (`server/app.pftrace`) for block-level profiling.
- **Testing** – We can run unit tests inside `contracts/orderbook/test`, integration tests in `server`, and end-to-end Goose scenarios—all sharing the same fixtures.
- **Tooling parity** – Everything builds with `cargo`, `bun` and leverage [hylix](https://crates.io/crates/hylix) `hy`, so contributors on Linux or macOS can bootstrap quickly without bespoke containers.

## Fully Open and RISC-V First

We openly credit Lighter and Starkex for popularizing the design. Hyliquid applies the same pattern but keeps the entire stack transparent:

- **Contracts, backend, prover, and UI** live in this repo with permissive licenses.
- **RISC-V artifacts** are published (`elf/orderbook`, `elf/orderbook_vk`), so anyone can verify the binaries we run.
- **No proprietary coordinator** – We rely solely on Hyli’s public node APIs and SP1. Anyone can replay proofs or run their own prover cluster.
- **Composable** – Other teams can fork the contracts, swap in alternative matching engines, or integrate different privacy layers while reusing the prover skeleton.

If you wanted to experiment with Lighter-like ideas but were constrained by closed tooling, Hyliquid is your sandbox.

## What’s Next

- **Metrics** – We are collecting detailed latency breakdowns (request → fast path, fast path → proof submission, proof submission → settlement) and will publish them soon.
- **Bridging flows** – Tightening the integration between `server`’s bridge module and external networks for seamless deposits/withdrawals.
- **User authentication** – Add/enhance authentication checks on the server-api to ensure only verified users can access endpoints.

## Getting Started

### Requirements:

- Bun
- Hylix: `cargo install hylix`
- Cargo
- SP1 toolkit

### Run

```bash
# 1. Clone the repo and install rustup + bun
git clone https://github.com/hyli-org/hyliquid
cd hyliquid

# 2. Build the contracts (SP1 RISC-V artifacts)
cargo build -p contracts --release

# 3. Start the devenet & fast-path server + prover
hy devnet up
hy run

# 4. Launch the read-only API and frontend
(cd server-api && bun install && bun dev)
(cd front && bun install && bun dev)
```

Clone it, run your own prover, and use Hyliquid as the blueprint for the next wave of zkVM-native applications on Hyli.

## Monitoring Stack

We ship a ready-to-use Grafana + Prometheus stack that scrapes the server’s `/metrics` endpoint (port `9002` by default) and auto-imports the dashboards located in `grafana/`.

```bash
cd monitoring
docker compose up -d
```

- Prometheus is exposed on `http://localhost:9090`.
- Grafana is exposed on `http://localhost:3001` (default credentials `admin`/`admin`).
- Dashboards **HTTP API Metrics** and **Database Metrics** are provisioned automatically and use the bundled Prometheus data source.
- By default Prometheus scrapes `host.docker.internal:9002`; update `monitoring/prometheus/prometheus.yml` if your server runs elsewhere or on a different port.

Make sure the Hyliquid server is running and reachable from the containers (Linux users may keep the default `host-gateway` mapping, macOS/Windows already provide `host.docker.internal`).

[twitter-badge]: https://img.shields.io/twitter/follow/hyli_org
[twitter-url]: https://x.com/hyli_org
[tg-badge]: https://img.shields.io/endpoint?url=https%3A%2F%2Ftg.sumanjay.workers.dev%2Fhyli_org%2F&logo=telegram&label=chat&color=neon
[tg-url]: https://t.me/hyli_org
