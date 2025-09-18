CREATE OR REPLACE FUNCTION seed_limit_orders_int (p_instrument_symbol text, p_n integer, p_mid_price_int bigint, p_price_range_ticks integer DEFAULT 200, -- amplitude autour du mid en ticks
p_min_qty_steps integer DEFAULT 1, p_max_qty_steps integer DEFAULT 100, p_side_bias_bid double precision DEFAULT 0.5 -- 0.7 => 70% de bids
)
    RETURNS void
    LANGUAGE plpgsql
    AS $$
DECLARE
    v_instrument_id bigint;
    v_tick_size bigint;
    v_qty_step bigint;
    v_user_ids bigint[];
BEGIN
    SELECT
        instrument_id,
        tick_size,
        qty_step INTO v_instrument_id,
        v_tick_size,
        v_qty_step
    FROM
        instruments
    WHERE
        symbol = p_instrument_symbol;
    IF v_instrument_id IS NULL THEN
        RAISE EXCEPTION 'Instrument % introuvable', p_instrument_symbol;
    END IF;
    SELECT
        array_agg(user_id) INTO v_user_ids
    FROM
        users;
    IF v_user_ids IS NULL OR array_length(v_user_ids, 1) = 0 THEN
        RAISE EXCEPTION 'Aucun user présent (table users vide)';
    END IF;
    WITH gens AS (
        SELECT
            v_instrument_id AS instrument_id,
            v_user_ids[1 + floor(random() * array_length(v_user_ids, 1))::int] AS user_id,
            CASE WHEN random() < p_side_bias_bid THEN
                'bid'::order_side
            ELSE
                'ask'::order_side
            END AS side,
            'limit'::order_type AS type,
            GREATEST (v_tick_size, p_mid_price_int + ((floor(random() * (2 * p_price_range_ticks + 1))::int - p_price_range_ticks) * v_tick_size))::bigint AS price,
            ((p_min_qty_steps + floor(random() * (p_max_qty_steps - p_min_qty_steps + 1))::int) * v_qty_step)::bigint AS qty
        FROM
            generate_series(1, p_n)
),
ins AS (
INSERT INTO orders (instrument_id, user_id, side, type, price, qty, status)
    SELECT
        instrument_id,
        user_id,
        side,
        type,
        price,
        qty,
        'open'::order_status
    FROM
        gens
    RETURNING
        order_id,
        instrument_id)
INSERT INTO order_events (order_id, instrument_id, event_type, seq, delta_qty)
SELECT
    order_id,
    instrument_id,
    'created'::order_event_type,
    1,
    NULL
FROM
    ins;
END
$$;

-- Wrapper for seed_limit_orders_int with human price
CREATE OR REPLACE FUNCTION seed_limit_orders (p_instrument_symbol text, p_n integer, p_mid_price numeric, -- ex: 27650.00
p_price_range_ticks integer DEFAULT 200, p_min_qty_steps integer DEFAULT 1, p_max_qty_steps integer DEFAULT 100, p_side_bias_bid double precision DEFAULT 0.5)
    RETURNS void
    LANGUAGE plpgsql
    AS $$
DECLARE
    v_ps smallint;
    v_mid_int bigint;
BEGIN
    SELECT
        price_scale INTO v_ps
    FROM
        instruments
    WHERE
        symbol = p_instrument_symbol;
    IF v_ps IS NULL THEN
        RAISE EXCEPTION 'Instrument % introuvable', p_instrument_symbol;
    END IF;
    -- mid_int = round(p_mid_price * 10^price_scale)
    v_mid_int := round(p_mid_price * power(10::numeric, v_ps))::bigint;
    PERFORM
        seed_limit_orders_int (p_instrument_symbol, p_n, v_mid_int, p_price_range_ticks, p_min_qty_steps, p_max_qty_steps, p_side_bias_bid);
END
$$;

-- Simulate order insertions at a given rate (p_rps) for a given duration (p_seconds)
CREATE OR REPLACE PROCEDURE pump_orders (p_instrument_symbol text -- instrument,
p_rps integer, -- orders par seconde (insertés par lots)
p_seconds integer, -- durée totale
p_mid_price numeric, p_price_range_ticks integer DEFAULT 200, p_min_qty_steps integer DEFAULT 1, p_max_qty_steps integer DEFAULT 100, p_side_bias_bid double precision DEFAULT 0.5)
LANGUAGE plpgsql
AS $$
DECLARE
    i int;
BEGIN
    FOR i IN 1..p_seconds LOOP
        PERFORM
            seed_limit_orders (p_instrument_symbol, p_rps, p_mid_price, p_price_range_ticks, p_min_qty_steps, p_max_qty_steps, p_side_bias_bid);
        PERFORM
            pg_sleep(1);
        -- rafales 1/s
    END LOOP;
END
$$;

