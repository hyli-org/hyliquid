CREATE OR REPLACE FUNCTION get_orderbook_grouped_by_ticks(
  p_instrument_symbol text,
  p_levels int DEFAULT 20,
  p_group_ticks int DEFAULT 10          -- largeur d’un niveau = p_group_ticks * tick_size
) RETURNS TABLE(side order_side, price bigint, qty bigint)
LANGUAGE sql AS $$
WITH ins AS (
  SELECT instrument_id, tick_size FROM instruments WHERE symbol=p_instrument_symbol
),
base AS (
  SELECT o.side, o.price, o.qty_remaining AS qty,
         (SELECT tick_size FROM ins) AS tick,
         (SELECT tick_size FROM ins) * p_group_ticks AS width
  FROM orders o, ins
  WHERE o.instrument_id = ins.instrument_id
    AND o.status IN ('open','partially_filled')
    AND o.price IS NOT NULL
  GROUP BY o.side, o.price, o.qty_remaining
),
buck AS (
  SELECT side,
         -- ancre sur plancher de bucket
         (FLOOR(price::numeric / width) * width)::bigint AS bucket_floor,
         width, tick, qty, price
  FROM base
),
lvl AS (
  SELECT side,
         -- prix affiché : top pour bids, bottom pour asks
         CASE WHEN side='bid' THEN bucket_floor + width - tick
              ELSE bucket_floor
         END AS price,
         SUM(qty) AS qty
  FROM buck
  -- ⚠️ on ne peut pas grouper par l'alias "price" ici
  -- equivalent to GROUP BY side, price
  GROUP BY 1, 2
),
limited AS (
  SELECT side, price, qty,
         ROW_NUMBER() OVER (
           PARTITION BY side
           ORDER BY CASE WHEN side='bid' THEN -price ELSE price END
         ) AS rn
  FROM lvl
)
SELECT side, price, qty
FROM limited
WHERE rn <= p_levels
ORDER BY side, CASE WHEN side='bid' THEN -price ELSE price END;
$$;

CREATE INDEX IF NOT EXISTS idx_orders_active_instr_status_price
ON public.orders (instrument_id, status, price)
INCLUDE (side, qty_remaining)
WHERE price IS NOT NULL
  AND status IN ('open','partially_filled');

