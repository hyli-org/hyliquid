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

## Key features

- 🕶️ Users, balances, and fills stay offchain and private
- ⚡ Sub-second UX: instant interactions while proofs run asynchronously
- 🔐 Non-custodial by design
- 🧱 All components are auditable, open-source, without black boxes

<!--Read more on our blog: link-->

## Fully Open and RISC-V First

Hyliquid follows the same high-level pattern as [Lighter](https://assets.lighter.xyz/whitepaper.pdf) or dYdX on [StarkEx](https://docs.starkware.co/starkex/index.html) but removes the black boxes: every component is auditable and reproducible.

- **Contracts, backend, prover, and UI** live in this repo with permissive licenses.
- **RISC-V artifacts** are published (elf/orderbook, elf/orderbook_vk), so anyone can verify the binaries we run.
- **No proprietary coordinator**: we rely solely on Hyli's public node APIs and SP1.

## Why Hyli?

Hyli treats proofs as a core primitive, not an add-on. This changes how you build:

- Hyli natively verifies proofs, including the SP1 proofs used for Hyliquid.
- Async proving without rollback complexity.
- Unified execution model. Contracts compile to RISC-V. The same artifacts run in the server, the prover, and onchain settlement.
- Easy to use developer tooling.

Hyliquid stays **private** (no private data published onchain), **performant** (fast-path execution in Rust), and **non-custodial** (proofs anchor state onchain).

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
hy config set build.release true
hy run

# 4. Launch the read-only API and frontend
(cd server-api && bun install && bun dev)
(cd front && bun install && bun dev)
```

Clone it, run your own prover, and use Hyliquid as the blueprint for the next wave of zkVM-native applications on Hyli.

### Monitoring Stack

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

## Developer Experience

The contract logic (orderbook crate) is imported directly by both the server and the prover.

There's no mismatch between "simulated" and "proved" behavior, they run the same code.

- **Single source of truth** – Zero divergence between fast-path and prover execution.
- **Module system** – Hyli's message bus connects the router, database, and prover without ad-hoc Kafka or RPC tiers.
- **Observability** – tracing exports Perfetto traces for block-level profiling.
- **Testing** – Unit tests in contracts/orderbook/test, integration tests in server/, and end-to-end Goose scenarios share the same fixtures.

## End-to-End Flow

<!--replace with image when blog post is published-->

1. **User action** – A trader submits an authenticated request via the frontend. Headers include `x-identity`, `x-public-key`, and `x-signature`, which `AuthHeaders::from_headers` validates before processing.
2. **Fast path execution** – The corresponding handler in `server/src/app.rs` locks the in-memory orderbook state, applies the action (deposit/order/cancel/withdraw), emits events, and updates the state snapshot.
3. **Persistence + job enqueue** – The handler writes a `BlobTransaction` plus `OrderbookProverRequest` to Postgres. This captures the full replay context (events, nonce, user info, private input).
4. **Block detection** – `OrderbookProverModule` listens to Hyli blocks, filters transactions that reference the orderbook’s lane, and batches the associated pending jobs.
5. **Proof generation** – For each pending job, the prover rehydrates the full `FullState`, derives commitment metadata, and calls `ClientSdkProver::prove`, which executes the SP1 zkVM.
6. **Submission + cleanup** – Once the proof returns, the module builds a `ProofTransaction` and sends it via `node_client.send_tx_proof`. Settled transactions are removed from the queue.
7. **Read APIs + UI updates** – The frontend polls `server-api/` to show the latest depth chart, fills, and balances—the same data the prover replays—so UX stays in sync with provable state.

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

## What’s Next

- **Metrics** – We are collecting detailed latency breakdowns (request → fast path, fast path → proof submission, proof submission → settlement) and will publish them soon.
- **Bridging flows** – Tightening the integration between `server`’s bridge module and external networks for seamless deposits/withdrawals.
- **User authentication** – Add/enhance authentication checks on the server-api to ensure only verified users can access endpoints.

[twitter-badge]: https://img.shields.io/twitter/follow/hyli_org
[twitter-url]: https://x.com/hyli_org
[tg-badge]: https://img.shields.io/endpoint?url=https%3A%2F%2Ftg.sumanjay.workers.dev%2Fhyli_org%2F&logo=telegram&label=chat&color=neon
[tg-url]: https://t.me/hyli_org
