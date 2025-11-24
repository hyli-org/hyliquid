Reth Bridge Testing (Deposit & Withdraw)
========================================

⚠️ Status
---------
- The new Reth bridge module is wired and exposes deposit/withdraw/status endpoints behind `--reth-bridge`.
- The bridge now generates two-blob Hyli transactions (raw ERC20 transfer + Orderbook deposit/withdraw action) and enqueues the corresponding Orderbook requests; Reth execution and EVM proofs will be handled later inside the prover.
- The embedded collateral ERC20 contract registers under `reth-collateral` by default so it does not conflict with the legacy SMT token contracts.
- Blob construction currently uses stub payloads (raw tx + identity) and submits via the Hyli client; EVM/Orderbook proofs are not yet attached.

How to run
----------
1) Build the workspace so dependencies are cached:
   - `cargo build`
2) Run the server with the new bridge:
   - `cargo run -p server -- --reth-bridge --no-prover --no-bridge=false`
   - Configure your Hyli node URL in the usual config (`config.toml`) or env; the bridge uses the same client as the server.

Deposit flow (manual)
---------------------
1) Craft and sign the ERC20 transfer that moves funds into the vault.
2) POST to the bridge deposit endpoint (amount is in the collateral token’s smallest unit). The bridge will wrap your raw ERC20 tx together with an Orderbook `Deposit` action in the blob:
   ```
   curl -X POST http://localhost:3000/reth_bridge/deposit \
     -H 'content-type: application/json' \
     -d '{
       "identity": "user@test",
       "signed_tx_hex": "0xdeadbeef",
       "amount": 1000000000
     }'
   ```
   Response: `{ "job_id": "job-0", "status": "queued", ... }`
3) Poll status:
   ```
   curl http://localhost:3000/reth_bridge/status/job-0
   ```
   You should see `l1_tx_hash`, `hyli_tx_hash`, and `evm_proof_hex` filled once the job completes (stubbed hashes at present).

Withdraw flow (manual)
----------------------
1) Craft the outbound ERC20 transfer payload (the bridge will pair it with the Orderbook `Withdraw` action that encodes the destination).
2) POST to the withdraw endpoint to capture the Hyli identity, destination, and payload:
   ```
   curl -X POST http://localhost:3000/reth_bridge/withdraw \
     -H 'content-type: application/json' \
     -d '{
       "identity": "user@test",
       "signed_tx_hex": "0xfeedface",
       "destination": { "network": "ethereum-dev", "address": "0xabc123..." },
       "amount": 5000
     }'
   ```
3) Poll status just like deposits. Once completed, the bridge publishes a `PendingWithdraw` to the Orderbook module so the Hyli path can observe that the transfer was issued.

What remains to be production-ready
-----------------------------------
- Extend the embedded Reth harness to deploy/mint ERC20 helpers, expose funded test accounts, and build the final stateless proofs the bridge submits.
- Build real ERC20 + Orderbook blobs (caller/callee-aware) and submit both EVM and Orderbook proofs via the Hyli client.
- Hook withdraw handling to consume settled Orderbook events and craft the outbound ERC20 transfer with matching proof checks.
- Add end-to-end tests that exercise deposit + withdraw round trips against the embedded node. 

Test collateral minter
----------------------
- Private key: `0xac0974bea...a4d1c` (full value below)
- Full private key: `0xac0974bea0bdc7d3a23a59c2474432c3d8a3fcca6a6b8cd056d1c2e98f5a9b58`
- Corresponding address: `0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266`
You can use this well-known dev account whenever you need to mint collateral for local testing.

CLI helpers
-----------
- Seed ERC20 balances by emitting a Hyli blob that wraps a signed `mint` transaction:
  ```
  cargo run -p server --bin seed_reth_collateral -- \
    --private-key 0xac0974bea0bdc7d3a23a59c2474432c3d8a3fcca6a6b8cd056d1c2e98f5a9b58 \
    --recipient 0x70997970C51812dc3A010C7d01b50e0d17dc79C8:1000000000
  ```
  The command signs a mint call, wraps it in a single collateral blob, and submits it to the Hyli node so the prover path can observe the mint.
- Register the collateral contract (only once) so Hyli knows about the `reth` verifier/program id:
  ```
  cargo run -p server --bin register_reth_collateral --
  ```
  This reads `config.toml`, derives the program id from your orderbook name, and submits the register contract call via the Hyli REST API.
- Craft a signed ERC20 transfer that can be POSTed to `/reth_bridge/deposit`:
  ```
  cargo run -p server --bin craft_reth_deposit -- \
    --private-key 0x... \
    --amount 500000000 \
    --identity user@test
  ```
