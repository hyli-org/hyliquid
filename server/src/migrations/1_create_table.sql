-- Tokens (ex: BTC, USDT).
CREATE TABLE assets (
    asset_id bigserial PRIMARY KEY,
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
    instrument_id bigserial PRIMARY KEY,
    symbol text UNIQUE NOT NULL, -- 'BTC/USDT'
    -- Smallest price increment
    -- ex: for BTC/USDT with price_scale=2, tick_size=5 means min price increment = 0.05 USDT
    -- requires: price % tick_size = 0
    tick_size bigint NOT NULL, -- ex: 1 = 0.01 si scale=2
    -- Smallest tradeable quantity
    -- ex: for BTC with scale=8, qty_step=1000 means min trade qty = 1000 satoshis = 0.0001 BTC
    -- requires: order.qty % qty_step = 0
    qty_step bigint NOT NULL,
    -- number of decimals for price
    -- defines the fixed-point representation (integer = real * 10^price_scale)
    price_scale smallint NOT NULL,
    -- eg. BTC/USDT => base=BTC, quote=USDT
    base_asset_id bigserial NOT NULL REFERENCES assets (asset_id),
    quote_asset_id bigserial NOT NULL REFERENCES assets (asset_id),
    status market_status NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

--------------------
-- Users and Balances
--------------------
CREATE TYPE user_status AS ENUM (
    'active',
    'suspended',
    'closed'
);

CREATE TABLE users (
    user_id bigserial PRIMARY KEY,
    identity TEXT UNIQUE NOT NULL,
    status user_status NOT NULL DEFAULT 'active',
    created_at timestamptz NOT NULL DEFAULT now()
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
    order_id bigserial PRIMARY KEY,
    order_user_signed_id text NOT NULL,
    instrument_id bigserial NOT NULL REFERENCES instruments (instrument_id),
    user_id bigserial NOT NULL REFERENCES users (user_id),
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
-- Filled from contract output: Vec<OrderbookEvent>
-----------------------
CREATE TYPE order_event_type AS ENUM (
    'created',
    'cancelled',
    'executed',
    'updated',
    'rejected',
    'expired'
);

CREATE TABLE order_events (
    event_id bigserial NOT NULL,
    order_id bigserial NOT NULL,
    instrument_id bigserial NOT NULL REFERENCES instruments (instrument_id),
    event_type order_event_type NOT NULL,
    event_time timestamptz NOT NULL DEFAULT now(),
    seq bigint NOT NULL, -- séquence par ordre ou globale
    delta_qty bigint, -- for executed/updated
    payload jsonb NOT NULL DEFAULT '{}'::jsonb,
    PRIMARY KEY (instrument_id, event_time, event_id) -- clé adaptée au partitionnement
)
-- PARTITION BY RANGE (event_time)
;

CREATE TABLE trades (
    trade_id BIGSERIAL,
    instrument_id bigint NOT NULL,
    taker_user_signed_id text NOT NULL, -- taker orders are not necessarily stored in the orders table as they can be instantly fullfilled
    maker_order_id bigserial NOT NULL, -- maker orders are always stored in the orders table
    price bigint NOT NULL,
    qty bigint NOT NULL,
    trade_time timestamptz NOT NULL DEFAULT now(),
    side order_side NOT NULL, -- côté du taker
    PRIMARY KEY (instrument_id, trade_time, trade_id)
)
-- PARTITION BY RANGE (trade_time)
;
