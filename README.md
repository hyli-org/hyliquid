# Hyliquid: Proving Private Orderbooks on Hyli

Hyliquid is our end-to-end demonstration that Hyli’s purpose-built stack for zkVM workloads makes it straightforward to ship private, performant, non-custodial trading systems. The entire project—contracts, backend, prover, APIs, and frontend—is fully open source and runs on a RISC-V friendly toolchain (SP1). If you are familiar with Lighter’s architecture, Hyliquid follows the same high-level pattern but removes the black boxes: every component is auditable and reproducible.

---

## 1. Why Hyli?

- **zkVM-native from day one** – Hyli bakes proof-friendly concepts (lanes, contract identities, blob transactions) directly into its node APIs. We never had to fight the runtime to integrate SP1 proofs.
- **Purpose-built tooling** – The `hy` CLI, module system, and SDKs handle key management, block subscriptions, and proof submission out of the box, reducing boilerplate on the prover side.
- **RISC-V everywhere** – Contracts compile to SP1’s RISC-V target, and the same artifacts are consumed by both the fast path and the prover. Developer environments stay portable.
- **No trade-offs** – Hyliquid stays private (no private data published onchain), performant (fast-path execution in Rust), and non-custodial (proofs anchor state on-chain). We never had to pick only two of the three.

---

## 2. Architecture at a Glance

```
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

---

## 3. Component Breakdown

### 3.1 `contracts/` – The Hyli ZKContract

- The `orderbook` crate defines `ORDERBOOK_ACCOUNT_IDENTITY`, event schemas, and the full transition logic for deposits, order placement, matching, and withdrawals.
- SP1 compiles this contract to RISC-V ELF artifacts (`elf/orderbook`, `elf/orderbook_vk`), which are embedded into both the fast path and the prover.
- Because the same code drives the on-chain state transition and the prover replay, we avoid “shadow logic” bugs.

### 3.2 `server/` – Fast Path + Database Writer

- `server/src/app.rs` exposes Axum handlers for `deposit`, `create_order`, `cancel_order`, `withdraw`, and `add_session_key`.
- Each handler executes the contract logic locally (using the same state structs as the contract), emits events, and pushes a `DatabaseRequest::WriteEvents` message onto the message bus.
- The database module persists both the serialized blob transaction and the `OrderbookProverRequest`, which contains everything the prover needs: user info, events, action metadata, and nonce.
- This process gives users immediate confirmation and a consistent state snapshot without waiting for a proof to finish.

### 3.3 `server/src/prover.rs` – Async SP1 Prover

- `OrderbookProverModule` subscribes to `NodeStateEvent::NewBlock` updates via Hyli’s message bus.
- For every new block, it filters transactions that belong to the orderbook’s lane, reloads the corresponding `OrderbookProverRequest` from Postgres, and reconstructs the zkVM context.
- `handle_prover_request` recreates the commitment metadata and calldata (including `ORDERBOOK_ACCOUNT_IDENTITY` blobs) before dispatching `ClientSdkProver::prove`.
- Proof generation happens in detached `tokio::spawn` tasks, ensuring the module keeps up with the block feed. Successful proofs are wrapped into `ProofTransaction`s and submitted via `node_client.send_tx_proof`.
- Settled transactions are deleted from `prover_requests`, keeping the queue lean and idempotent.

### 3.4 `server-api/` – Read-Only Surface

- Built with Bun + TypeScript, this service exposes orderbook depth, user portfolio, and trade history endpoints by reading directly from the persisted state tables.
- Because it never mutates state, we can scale it horizontally or cache aggressively without risking stale proofs.

### 3.5 `front/` – User Interface

- A Vite/Bun frontend that consumes both Hyli RPCs and the read API.
- Users authenticate with Hyli identities, submit signed payloads, and receive instant feedback sourced from the fast-path state.

### 3.6 `loadtest/` – Reality Checks

- Goose-based scenarios (`maker.rs`, `taker.rs`, etc.) validate throughput on real HTTP flows.
- Recent improvements add better error propagation (`?` instead of `unwrap()`) and production endpoints, so the same suite can benchmark devnet and staging clusters alike.

---

## 4. End-to-End Flow

1. **User action** – A trader submits an authenticated request via the frontend. Headers include `x-identity`, `x-public-key`, and `x-signature`, which `AuthHeaders::from_headers` validates before processing.
2. **Fast path execution** – The corresponding handler in `server/src/app.rs` locks the in-memory orderbook state, applies the action (deposit/order/cancel/withdraw), emits events, and updates the state snapshot.
3. **Persistence + job enqueue** – The handler writes a `BlobTransaction` plus `OrderbookProverRequest` to Postgres. This captures the full replay context (events, nonce, user info, private input).
4. **Block detection** – `OrderbookProverModule` listens to Hyli blocks, filters transactions that reference the orderbook’s lane, and batches the associated pending jobs.
5. **Proof generation** – For each pending job, the prover rehydrates the full `FullState`, derives commitment metadata, and calls `ClientSdkProver::prove`, which executes the SP1 zkVM.
6. **Submission + cleanup** – Once the proof returns, the module builds a `ProofTransaction` and sends it via `node_client.send_tx_proof`. Settled transactions are removed from the queue.
7. **Read APIs + UI updates** – The frontend polls `server-api/` to show the latest depth chart, fills, and balances—the same data the prover replays—so UX stays in sync with provable state.

---

## 5. Privacy + Performance + Non-Custodial (All at Once)

- **Privacy** – Users’ secrets (orders, balances, session keys) stay inside the secured server. Those are never exposed onchain.
- **Performance** – Users see sub-second confirmations because the fast path executes Rust structs, not an on-chain VM. Asynchronous proving amortizes zk costs without blocking UX.
- **Non-custodial** – Final settlement always happens on Hyli. The backend cannot forge state transitions because every change must be proven against the SP1 contract before it becomes canonical.

This trifecta is typically a compromise on other stacks. Hyli’s architecture lets us keep all three without bolting on custom infra.

---

## 6. Developer Experience

- **Single source of truth** – The contract logic (`orderbook` crate) is imported directly by both the server and the prover, so there is zero mismatch between “simulated” and “proved” behavior.
- **Module system** – Hyli’s `module_bus_client!` macros give us typed channels between the router, database, and prover. No ad-hoc Kafka, no extra RPC tier.
- **Observability** – `tracing` is wired through the server and prover, and we can export Perfetto traces (`server/app.pftrace`) for block-level profiling.
- **Testing** – We can run unit tests inside `contracts/orderbook/test`, integration tests in `server`, and end-to-end Goose scenarios—all sharing the same fixtures.
- **Tooling parity** – Everything builds with `cargo`, `bun` and leverage [hylix](https://crates.io/crates/hylix) `hy`, so contributors on Linux or macOS can bootstrap quickly without bespoke containers.

---

## 7. Fully Open and RISC-V First (vs. Lighter)

We openly credit Lighter for popularizing the split between fast execution and asynchronous proving. Hyliquid applies the same pattern but keeps the entire stack transparent:

- **Contracts, backend, prover, and UI** live in this repo with permissive licenses.
- **RISC-V artifacts** are published (`elf/orderbook`, `elf/orderbook_vk`), so anyone can verify the binaries we run.
- **No proprietary coordinator** – We rely solely on Hyli’s public node APIs and SP1. Anyone can replay proofs or run their own prover cluster.
- **Composable** – Other teams can fork the contracts, swap in alternative matching engines, or integrate different privacy layers while reusing the prover skeleton.

If you wanted to experiment with Lighter-like ideas but were constrained by closed tooling, Hyliquid is your sandbox.

---

## 8. What’s Next

- **Metrics** – We are collecting detailed latency breakdowns (request → fast path, fast path → proof submission, proof submission → settlement) and will publish them soon.
- **More markets** – Expanding beyond the initial `ORANJ/HYLLAR` pair to stress-test the state model with multiple orderbooks.
- **Bridging flows** – Tightening the integration between `server`’s bridge module and external networks for seamless deposits/withdrawals.
- **Community contributions** – Issues are tagged for contracts, prover, front, and tooling. We welcome audits, UX polish, additional load tests, and alternative proving strategies (e.g., multi-proof batching).

---

## 9. Getting Started

```bash
# 1. Clone the repo and install rustup + bun
git clone https://github.com/hyli-org/hyliquid
cd hyliquid

# 2. Build the contracts (SP1 RISC-V artifacts)
cargo build -p contracts --release

# 3. Start the fast-path server & prover
hy run --no-bridge

# 4. Launch the read-only API and frontend
(cd server-api && bun install && bun dev)
(cd front && bun install && bun dev)
```

Clone it, run your own prover, and use Hyliquid as the blueprint for the next wave of zkVM-native applications on Hyli.
