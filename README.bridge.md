Bridge Replacement Plan (Orderbook-Only)
========================================

Goal: replace the existing Ethereum bridge with an embedded Reth devnode + ERC20 that feeds deposits/withdrawals directly into the Orderbook contract (no SMT token SP1 contract). The bridge owns both sides: it drives L1 transactions, extracts witnesses, and composes Hyli blobs that call the Orderbook contract for settlement.

What to build
-------------
- New bridge module
  - Implement as a parallel module exposing the same public interface (bus messages and HTTP surface) so it can be swapped in without API changes.
  - Internally depend on the embedded Reth + ERC20 helpers below; keep current module intact for rollback and accessed via `--reth-bridge` flag.

- Embedded Reth node
  - Spin up a local Reth devnode with block-per-tx mining (adapt `NodeBuilder` flow from `../reth-l2-test/crates/testnet/src/bin/deposit_demo.rs`).
  - Enable debug API for `debug_execution_witness_by_block_hash`.
  - Preload a deterministic mnemonic and chain/spec constants; expose RPC handles internally (no external endpoint required).

- ERC20 lifecycle on embedded Reth
  - Auto-deploy a TestERC20 (or canonical collateral token) at bridge boot.
  - Mint funds to test users (or to vault) to cover deposits/withdrawals during local runs.
  - Derive the enforced vault recipient from the Orderbook program id (similar to `program_address_from_program_id` pattern) instead of a static vault address in config.

- Bridge server refactor
  - Replace the current `EthListener`/`EthClient` and claim table plumbing with a bridge driver that submits L1 txns and proofs.
  - Config: drop external RPC URLs and `eth_signer_private_key`; add embedded reth toggles (enable/disable, chain id, mnemonic path, datadir path, funded accounts).
  - Persistence: keep minimal tracking/status rows (e.g., bridge job id, L1 tx hash, Hyli tx hash, status) instead of the existing pending/processed Eth tables.

- Deposit flow (Orderbook-directed, two-blob payload)
  1) User hits a deposit endpoint with their signed ERC20 transfer TX (to the vault address) and the Hyli identity they use on the Orderbook.
  2) Bridge submits the TX to embedded Reth; wait for the block; fetch `ExecutionWitness`; run stateless validation to produce the EVM proof exactly like `deposit_demo/erc20.rs` (stateless input from block + witness).
  3) Bridge crafts a Hyli blob transaction with two blobs (callers/callees set to `None`, matching the deposit demo):
     - Blob 1: structured ERC20 transfer blob containing the raw L1 TX parameters to anchor the L1 action.
     - Blob 2: Orderbook deposit action blob adapted to consume the L1 transfer (amount/identity) and credit the vault balance.
  4) Bridge submits the blob TX and two proofs:
     - Proof A: EVM/stateless proof built from the Reth block witness (as in the deposit demo).
     - Proof B: Orderbook prover output for the deposit action.
  5) On Hyli settlement, forward a `PendingDeposit` to the Orderbook module (fast path already expects this).

- Withdraw flow (Orderbook-directed, two-blob payload with caller/callee)
  1) When the Orderbook settles a withdraw with `network == ethereum-*`, the bridge crafts and submits an ERC20 `transfer` from the vault to the user’s EOA on embedded Reth. The ERC20 blob must include a `caller` (vault identity/program id initialized the same way as in the deposit demo) and no callees.
  2) Build a paired Orderbook withdraw blob that uses a `callee` pointing to the ERC20 program id (initialized like the deposit demo) and encodes the destination/amount.
  3) Collect the block + `ExecutionWitness`, run stateless validation, and submit the resulting Hyli proof tying the settled Orderbook withdraw blob to the L1 transfer.
  4) The Orderbook contract must check that the ERC20 transfer amount matches the withdraw request when consuming the callee blob.
  5) Track status (job id, L1 tx hash, Hyli tx hash) so the frontend can display “in-flight / confirmed / failed” without claim logic.
  6) Hyli-only withdraws continue to use the existing `SmtTokenAction::Transfer` path; cross-chain withdraws use the new Reth flow.

- Orderbook contract changes
  - Adapt the `Deposit` action to consume the ERC20 transfer blob (amount/identity) with `caller/callees == None`; credit the balance to `vault@orderbook` (vault identity derived from the Orderbook contract name) before applying the user deposit.
  - Adapt the `Withdraw` handling to consume a callee blob that references the ERC20 transfer; enforce that the ERC20 transfer amount equals the requested withdraw amount, and that the ERC20 program id matches the initialized value (same derivation as deposit demo).
  - Ensure the contract records the intended L1 destination for withdraws and produces the blob payload the bridge will consume when submitting the L1 tx.
  - Keep the prover path aligned: on-chain execution and SP1 replay must share the same blob parsing; proof verification remains in the Hyli verifier.

- Frontend/API updates
  - Remove the Ethereum “claim” UX; replace with a bridge job status endpoint (submit deposit → poll job id → show L1/Hyli hashes).
  - Withdraw modal should branch: Hyli-only (no bridge) vs. Ethereum (bridge job with L1 tx hash once signed).

- Testing & DX
  - Add an E2E test that boots the server with embedded Reth, runs deposit + withdraw loops, and asserts Orderbook balances and ERC20 balances match.
  - Provide a `make bridge-demo` (or similar) script that seeds accounts, runs the bridge, performs a sample deposit/withdraw, and prints hashes.
  - Document env flags in this README: enable/disable embedded reth, mnemonic overrides, datadir path, default amounts.

Implementation phases
---------------------
1) Scaffolding: embed Reth node + ERC20 deployment/mint helpers; new bridge config fields; stub status storage.
2) Deposit pipeline: RPC -> L1 transfer -> stateless proof -> Hyli blob -> Orderbook deposit call.
3) Withdraw pipeline: Orderbook event -> L1 transfer -> proof -> Hyli confirmation; unify status reporting.
4) Orderbook contract/prover updates for EVM proof verification; ensure SP1 replay consumes the same proof bytes.
5) Frontend/API polish and E2E test harness; remove legacy claim endpoints and database tables.
