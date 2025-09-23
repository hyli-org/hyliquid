-- Génére p_n ordres LIMIT autour d’un mid (entier),
-- avec une distribution de statuts (open/partial/filled).
-- Écrit un event 'created' pour tous, et un 'executed' si qty_filled > 0.
CREATE OR REPLACE FUNCTION seed_limit_orders_int_v2(
  p_instrument_symbol text,
  p_n                  integer,
  p_mid_price_int      bigint,
  p_price_range_ticks  integer DEFAULT 200,     -- amplitude ± en ticks
  p_min_qty_steps      integer DEFAULT 1,
  p_max_qty_steps      integer DEFAULT 100,
  p_side_bias_bid      double precision DEFAULT 0.5,     -- 0.7 => 70% bid
  p_ratio_open         double precision DEFAULT 0.6,     -- 60% open
  p_ratio_partial      double precision DEFAULT 0.25,    -- 25% partial
  p_ratio_filled       double precision DEFAULT 0.15     -- 15% filled
) RETURNS void
LANGUAGE plpgsql
AS $$
DECLARE
  v_instrument_id bigint;
  v_tick_size     bigint;
  v_qty_step      bigint;
  v_user_ids      bigint[];
BEGIN
  -- Sanity ratios
  IF (p_ratio_open + p_ratio_partial + p_ratio_filled) <= 0.0 THEN
    RAISE EXCEPTION 'Ratios must be > 0';
  END IF;

  -- Récupère les paramètres de l’instrument (non ambigu)
  SELECT i.instrument_id, i.tick_size, i.qty_step
    INTO v_instrument_id, v_tick_size, v_qty_step
  FROM instruments i
  WHERE i.symbol = p_instrument_symbol;

  IF v_instrument_id IS NULL THEN
    RAISE EXCEPTION 'Instrument % not found', p_instrument_symbol;
  END IF;

  -- Liste des users
  SELECT array_agg(u.user_id ORDER BY u.user_id)
    INTO v_user_ids
  FROM users u;

  IF v_user_ids IS NULL OR array_length(v_user_ids,1) = 0 THEN
    RAISE EXCEPTION 'No users in table users';
  END IF;

  WITH params AS (
    SELECT
      v_instrument_id AS p_ins_id,
      v_tick_size     AS p_tick,
      v_qty_step      AS p_qstep,
      p_mid_price_int AS p_mid,
      p_price_range_ticks AS p_rng,
      p_min_qty_steps AS p_smin,
      p_max_qty_steps AS p_smax,
      p_side_bias_bid AS p_side_bias,
      p_ratio_open    AS p_r_open,
      p_ratio_partial AS p_r_part,
      p_ratio_filled  AS p_r_fill
  ),
  gens AS (
    SELECT
      p.p_ins_id AS ord_instrument_id,
      -- user aléatoire
      (SELECT v_user_ids[1 + floor(random()*array_length(v_user_ids,1))::int])::bigint AS ord_user_id,
      -- incrementing order id
      md5(g::text)::text AS ord_user_signed_id,
      -- bid/ask biaisé
      (CASE WHEN random() < p.p_side_bias THEN 'bid' ELSE 'ask' END)::order_side AS ord_side,
      'limit'::order_type AS ord_type,
      -- prix = mid ± k*tick (>= tick)
      GREATEST(p.p_tick,
               p.p_mid + ((floor(random() * (2*p.p_rng + 1))::int - p.p_rng) * p.p_tick)
      )::bigint AS ord_price,
      -- qty = n_steps * qty_step
      ((p.p_smin + floor(random()*(p.p_smax - p.p_smin + 1))::int) * p.p_qstep)::bigint AS ord_qty,
      random() AS rdraw,
      p.*
    FROM params p, generate_series(1, p_n) g
  ),
  statusd AS (
    SELECT
      g.ord_instrument_id,
      g.ord_user_id,
      g.ord_signed_id,
      g.ord_side,
      g.ord_type,
      g.ord_price,
      g.ord_qty,
      -- ratios normalisés
      (p_r_open/(p_r_open+p_r_part+p_r_fill)) AS w_open,
      (p_r_part/(p_r_open+p_r_part+p_r_fill)) AS w_part,
      (p_r_fill/(p_r_open+p_r_part+p_r_fill)) AS w_fill,
      g.rdraw,
      g.p_qstep
    FROM gens g
  ),
  labeled AS (
    SELECT
      s.ord_instrument_id,
      s.ord_user_id,
      s.ord_signed_id,
      s.ord_side,
      s.ord_type,
      s.ord_price,
      s.ord_qty,
      CASE
        WHEN s.rdraw < s.w_fill                    THEN 'filled'
        WHEN s.rdraw < (s.w_fill + s.w_part)       THEN 'partially_filled'
        ELSE 'open'
      END::order_status AS ord_status,
      s.p_qstep
    FROM statusd s
  ),
  with_fill AS (
    SELECT
      l.ord_instrument_id,
      l.ord_user_id,
      l.ord_signed_id,
      l.ord_side,
      l.ord_type,
      l.ord_price,
      l.ord_qty,
      l.ord_status,
      CASE
        WHEN l.ord_status = 'open' THEN 0
        WHEN l.ord_status = 'filled' THEN l.ord_qty
        ELSE
          -- partial: nombre entier de steps strictement entre 0 et qty
          (
            GREATEST(
              1,
              LEAST(
                (l.ord_qty / l.p_qstep) - 1,
                (1 + floor(random() * GREATEST(1, (l.ord_qty / l.p_qstep) - 1)))::int
              )
            ) * l.p_qstep
          )
      END::bigint AS ord_qty_filled
    FROM labeled l
  ),
  ins_orders AS (
    INSERT INTO orders(
      instrument_id, user_id, order_signed_id, side, type, price, qty, qty_filled, status
    )
    SELECT
      wf.ord_instrument_id,
      wf.ord_user_id,
      wf.ord_signed_id,
      wf.ord_side,
      wf.ord_type,
      wf.ord_price,
      wf.ord_qty,
      wf.ord_qty_filled,
      wf.ord_status
    FROM with_fill wf
    RETURNING orders.order_id AS ret_order_id,
              orders.instrument_id AS ret_instrument_id,
              orders.qty AS ret_qty,
              orders.qty_filled AS ret_qty_filled
  )
  -- events: created + executed (si filled>0)
  INSERT INTO order_events(order_id, instrument_id, event_type, seq, delta_qty, payload)
  SELECT io.ret_order_id, io.ret_instrument_id, 'created'::order_event_type, 1, NULL, '{}'::jsonb
  FROM ins_orders io
  UNION ALL
  SELECT io.ret_order_id, io.ret_instrument_id, 'executed'::order_event_type, 2, io.ret_qty_filled,
         jsonb_build_object('note','seed_fill')
  FROM ins_orders io
  WHERE io.ret_qty_filled > 0;

END$$;


CREATE OR REPLACE FUNCTION seed_limit_orders_v2(
  p_instrument_symbol text,
  p_n                  integer,
  p_mid_price          numeric,          -- ex: 27650.00
  p_price_range_ticks  integer DEFAULT 200,
  p_min_qty_steps      integer DEFAULT 1,
  p_max_qty_steps      integer DEFAULT 100,
  p_side_bias_bid      double precision DEFAULT 0.5,
  p_ratio_open         double precision DEFAULT 0.6,
  p_ratio_partial      double precision DEFAULT 0.25,
  p_ratio_filled       double precision DEFAULT 0.15
) RETURNS void
LANGUAGE plpgsql
AS $$
DECLARE
  v_ps smallint;
  v_mid_int bigint;
BEGIN
  SELECT price_scale INTO v_ps FROM instruments WHERE symbol=p_instrument_symbol;
  IF v_ps IS NULL THEN RAISE EXCEPTION 'Instrument % not found', p_instrument_symbol; END IF;

  v_mid_int := round(p_mid_price * power(10::numeric, v_ps))::bigint;

  PERFORM seed_limit_orders_int_v2(
    p_instrument_symbol, p_n, v_mid_int,
    p_price_range_ticks, p_min_qty_steps, p_max_qty_steps, p_side_bias_bid,
    p_ratio_open, p_ratio_partial, p_ratio_filled
  );
END$$;

-- SELECT seed_limit_orders_v2(
--   'BTC/USDT',
--   30_000,         -- qty orders
--   27649.00,       -- mid price
--   249,            -- ± ticks
--   0, 2000,        -- steps min/max
--   0.6,            -- bias bid
--   0.4, 0.3, 0.3   -- open / partial / filled
-- );
