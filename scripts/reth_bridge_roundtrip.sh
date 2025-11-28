#!/usr/bin/env bash
set -euo pipefail

# Basic end-to-end helper for the embedded Reth bridge:
# 1) Mint collateral to a recipient.
# 2) Submit a deposit to the Hyli server.
# 3) Submit a withdraw to the same/different address.

SERVER_URL="${SERVER_URL:-http://localhost:9002}"
ORDERBOOK_CN="${ORDERBOOK_CN:-orderbook}"
IDENTITY="${IDENTITY:-user@${ORDERBOOK_CN}}"
COLLATERAL_CN="${COLLATERAL_CN:-reth-collateral-${ORDERBOOK_CN}}"

# Seeded recipient and withdraw destination; override via env vars if needed.
RECIPIENT="${RECIPIENT:-0x70997970C51812dc3A010C7d01b50e0d17dc79C8:1000000000}"
WITHDRAW_ADDR="${WITHDRAW_ADDR:-0x70997970C51812dc3A010C7d01b50e0d17dc79C8}"
WITHDRAW_AMOUNT="${WITHDRAW_AMOUNT:-1000000000}"

echo "Minting collateral to ${RECIPIENT%:*}..."
cargo run -p server --bin seed_reth_collateral -- \
  --orderbook-cn "$ORDERBOOK_CN" \
  --contract-name "$COLLATERAL_CN" \
  --recipient "$RECIPIENT"

echo "Submitting deposit_reth_bridge..."
cargo run -p server --bin craft_reth_deposit -- \
  --orderbook-cn "$ORDERBOOK_CN" \
  --collateral-token-cn "$COLLATERAL_CN" \
  --amount 1000000000000000000 \
  --identity "$IDENTITY" \
  --server-url "$SERVER_URL"

echo "Submitting withdraw_reth_bridge..."
cargo run -p server --bin craft_reth_withdraw -- \
  --orderbook-cn "$ORDERBOOK_CN" \
  --eth-address "$WITHDRAW_ADDR" \
  --amount "$WITHDRAW_AMOUNT" \
  --identity "$IDENTITY" \
  --server-url "$SERVER_URL"
