--------------------
-- Used to track history of the database
-- Used to get state of the database at a given tx
-- Commits
--------------------
CREATE TABLE commits (
  commit_id      bigserial PRIMARY KEY,
  tx_hash        text NOT NULL,
  authored_at    timestamptz NOT NULL DEFAULT now(),
  message        text NOT NULL DEFAULT ''
);

-- Tokens (ex: BTC, USDT).
CREATE TABLE assets (
    asset_id bigserial PRIMARY KEY,
    -- 'bitcoin', 'tether', ...
    contract_name text UNIQUE NOT NULL,
    -- 'BTC', 'USDT', ...
    symbol text UNIQUE NOT NULL,
    -- number of decimals: scale=8 for BTC, 6 for USDT, etc.
    -- defines the fixed-point representation (integer = real * 10^scale)
    scale smallint NOT NULL CHECK (scale BETWEEN 0 AND 18),
    -- Smallest tradable
    -- ex: for BTC with scale=8, step=1000 means min trade qty = 1000 satoshis = 0.0001 BTC
    -- requires: balance % step = 0
    step bigint NOT NULL DEFAULT 1,
    status text NOT NULL DEFAULT 'active',
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TYPE market_status AS ENUM (
    'active',
    'halted',
    'closed'
);

-- Exemple: BTC/USDT
CREATE TABLE instruments (
    commit_id bigint NOT NULL REFERENCES commits (commit_id),
    instrument_id bigserial PRIMARY KEY,
    symbol text UNIQUE NOT NULL, -- 'BTC/USDT'
    -- Smallest price increment
    -- requires: price % tick_size = 0
    tick_size bigint NOT NULL,
    -- Smallest tradeable quantity
    -- ex: for BTC with scale=8, qty_step=1000 means min trade qty = 1000 satoshis = 0.0001 BTC
    -- requires: order.qty % qty_step = 0
    qty_step bigint NOT NULL,
    -- eg. BTC/USDT => base=BTC, quote=USDT
    base_asset_id bigint NOT NULL REFERENCES assets (asset_id),
    quote_asset_id bigint NOT NULL REFERENCES assets (asset_id),
    status market_status NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

--------------------
-- Live view of current state
-- Users and Balances
--------------------
CREATE TYPE user_status AS ENUM (
    'active',
    'suspended',
    'closed'
);

CREATE TABLE users (
    user_id bigserial PRIMARY KEY,
    commit_id bigint NOT NULL REFERENCES commits (commit_id),
    identity TEXT UNIQUE NOT NULL,
    salt BYTEA NOT NULL,
    nonce bigint NOT NULL DEFAULT 0,
    status user_status NOT NULL DEFAULT 'active',
    created_at timestamptz NOT NULL DEFAULT now()
);

-- Append only, latest line (max commit_id) has all session keys for a user
CREATE TABLE user_session_keys (
    user_id bigint NOT NULL REFERENCES users (user_id),
    commit_id bigint NOT NULL REFERENCES commits (commit_id),
    session_keys BYTEA[] NOT NULL,
    PRIMARY KEY (user_id, session_keys)
);

CREATE TABLE balances (
    user_id bigint NOT NULL REFERENCES users (user_id),
    asset_id bigint NOT NULL REFERENCES assets (asset_id),
    total bigint NOT NULL DEFAULT 0, -- quantity owned
    reserved bigint NOT NULL DEFAULT 0, -- quantity reserved for open orders
    available bigint GENERATED ALWAYS AS (total - reserved) STORED,
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, asset_id),
    CHECK (total >= 0 AND reserved >= 0 AND total >= reserved)
)
WITH (
    fillfactor = 90
);

--------------------
-- Orders
--------------------
CREATE TYPE order_side AS ENUM (
    'bid', -- buy
    'ask' -- sell
);

-- helper to get the other side
CREATE OR REPLACE FUNCTION get_other_side(p_side order_side) RETURNS order_side
LANGUAGE sql AS $$
SELECT CASE WHEN p_side = 'bid' THEN 'ask'::order_side ELSE 'bid'::order_side END;
$$;

CREATE TYPE order_type AS ENUM (
    'limit',
    'market',
    'stop_limit',
    'stop_market'
);

CREATE TYPE order_status AS ENUM (
    'open',
    'partially_filled',
    'filled',
    'cancelled',
    'rejected'
);

CREATE TABLE orders (
    order_id text NOT NULL PRIMARY KEY,
    user_id bigint NOT NULL REFERENCES users (user_id),
    instrument_id bigint NOT NULL REFERENCES instruments (instrument_id),
    side order_side NOT NULL,
    type order_type NOT NULL,
    price bigint, -- fixed-point (nullable for market)
    qty bigint NOT NULL,
    qty_filled bigint NOT NULL DEFAULT 0,
    qty_remaining bigint GENERATED ALWAYS AS (qty - qty_filled) STORED,
    status order_status NOT NULL DEFAULT 'open',
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
)
WITH (
    fillfactor = 90
);

-----------------------
-- Events, append-only & partitioned
-- Filled from contract output
-----------------------
CREATE TABLE order_events (
    commit_id bigint NOT NULL REFERENCES commits (commit_id),
    event_id bigserial PRIMARY KEY,
    order_id text NOT NULL,
    user_id bigint NOT NULL,
    instrument_id bigint NOT NULL REFERENCES instruments (instrument_id),
    side order_side NOT NULL,
    type order_type NOT NULL,
    price bigint NOT NULL,
    qty bigint NOT NULL,
    qty_filled bigint NOT NULL,
    status order_status NOT NULL,
    event_time timestamptz NOT NULL DEFAULT now()
)
-- PARTITION BY RANGE (event_time)
;

CREATE TABLE trade_events (
    commit_id bigint NOT NULL REFERENCES commits (commit_id),
    trade_id bigserial PRIMARY KEY,
    maker_order_id text NOT NULL,
    taker_order_id text NOT NULL,
    maker_user_id bigint NOT NULL,
    taker_user_id bigint NOT NULL,
    instrument_id bigint NOT NULL REFERENCES instruments (instrument_id),
    price bigint NOT NULL,
    qty bigint NOT NULL,
    side order_side NOT NULL, -- côté du taker
    trade_time timestamptz NOT NULL DEFAULT now()
)
-- PARTITION BY RANGE (trade_time)
;

CREATE TYPE balance_event_kind AS ENUM (
  'deposit', 'withdrawal',
  'reserve_inc', 'reserve_dec',
  'transfer',
  'settlement'
);

CREATE TABLE balance_events (
  commit_id   bigint NOT NULL REFERENCES commits(commit_id),
  event_id    bigserial PRIMARY KEY,
  user_id     bigint NOT NULL,
  asset_id    bigint NOT NULL REFERENCES assets (asset_id),
  total       bigint NOT NULL DEFAULT 0,
  reserved    bigint NOT NULL DEFAULT 0,
  kind        balance_event_kind NOT NULL,
  ref_order_id text DEFAULT NULL,
  ref_trade_signed_id text DEFAULT NULL,
  event_time  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE prover_requests (
  commit_id bigint NOT NULL REFERENCES commits (commit_id),
  tx_hash text NOT NULL,
  request bytea NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE user_events_nonces (
  commit_id bigint NOT NULL REFERENCES commits (commit_id),
  user_id bigint NOT NULL REFERENCES users (user_id),
  nonce bigint NOT NULL DEFAULT 0,
  created_at timestamptz NOT NULL DEFAULT now()
);

--------------------
-- Bridge module persistence
--------------------
CREATE TABLE bridge_metadata (
  id SMALLINT PRIMARY KEY DEFAULT 0 CHECK (id = 0),
  eth_last_block BIGINT NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE bridge_eth_address_bindings (
  eth_address BYTEA PRIMARY KEY,
  user_identity TEXT NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX bridge_eth_address_bindings_identity_idx
  ON bridge_eth_address_bindings (user_identity);

CREATE TABLE bridge_eth_pending_txs (
  tx_hash BYTEA PRIMARY KEY,
  block_number BIGINT NOT NULL,
  from_address BYTEA NOT NULL,
  to_address BYTEA NOT NULL,
  amount BYTEA NOT NULL,
  timestamp BIGINT NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('pending', 'confirmed')),
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX bridge_eth_pending_from_idx
  ON bridge_eth_pending_txs (from_address);

CREATE TABLE bridge_eth_processed_txs (
  tx_hash BYTEA PRIMARY KEY,
  processed_at timestamptz NOT NULL DEFAULT now()
);
