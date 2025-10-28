import { Pool } from "pg";
import { Asset, Instrument, Order, Trade, User } from "@/types";

/**
 * Database query helpers
 */
export class DatabaseQueries {
  private pool: Pool;

  constructor(pool: Pool) {
    this.pool = pool;
  }

  async getAllAssets(): Promise<Asset[]> {
    const result = await this.pool.query(
      "SELECT * FROM assets ORDER BY symbol"
    );
    return result.rows.map((row) => ({
      ...row,
      asset_id: parseInt(row.asset_id, 10),
      scale: parseInt(row.scale, 10),
      step: parseInt(row.step, 10),
    }));
  }

  async getAllInstruments(): Promise<Instrument[]> {
    const result = await this.pool.query(
      "SELECT * FROM instruments ORDER BY symbol"
    );
    return result.rows.map((row) => ({
      ...row,
      instrument_id: parseInt(row.instrument_id, 10),
      base_asset_id: parseInt(row.base_asset_id, 10),
      quote_asset_id: parseInt(row.quote_asset_id, 10),
      tick_size: parseInt(row.tick_size, 10),
      qty_step: parseInt(row.qty_step, 10),
      commit_id: parseInt(row.commit_id, 10),
    }));
  }

  async getInstrument(symbol: string): Promise<Instrument | null> {
    const result = await this.pool.query(
      "SELECT * FROM instruments WHERE symbol = $1",
      [symbol]
    );
    return result.rows[0] || null;
  }

  async getAllUsers(): Promise<Array<{ identity: string; user_id: number }>> {
    const result = await this.pool.query("SELECT identity, user_id FROM users");
    return result.rows;
  }

  async getUserByIdentity(
    identity: string
  ): Promise<{ user_id: number } | null> {
    const result = await this.pool.query(
      "SELECT user_id FROM users WHERE identity = $1",
      [identity]
    );
    return result.rows[0] || null;
  }

  async getUserById(userId: number): Promise<User> {
    const result = await this.pool.query(
      "SELECT * FROM users WHERE user_id = $1",
      [userId]
    );
    return result.rows[0] || null;
  }

  async getUserBalances(userId: number): Promise<
    Array<{
      symbol: string;
      total: number;
      reserved: number;
      available: number;
    }>
  > {
    const result = await this.pool.query(
      `
      SELECT 
        assets.symbol, balances.total, balances.reserved, balances.available 
      FROM 
        balances
      JOIN 
        assets ON balances.asset_id = assets.asset_id
      WHERE 
        balances.user_id = $1
    `,
      [userId]
    );
    return result.rows.map((row) => ({
      symbol: row.symbol,
      total: parseInt(row.total, 10),
      reserved: parseInt(row.reserved, 10),
      available: parseInt(row.available, 10),
    }));
  }

  async getUserNonce(userId: number): Promise<number> {
    const result = await this.pool.query(
      "SELECT nonce FROM users WHERE user_id = $1",
      [userId]
    );
    return parseInt(result.rows[0].nonce, 10);
  }

  async getUserOrders(
    userId: number,
    page: number = 1,
    limit: number = 20,
    sortBy: string = "created_at",
    sortOrder: "asc" | "desc" = "desc"
  ): Promise<{ orders: Array<Order>; total: number }> {
    const offset = (page - 1) * limit;

    // Validate sort column to prevent SQL injection
    const allowedSortColumns = [
      "created_at",
      "updated_at",
      "price",
      "qty",
      "status",
    ];
    const safeSortBy = allowedSortColumns.includes(sortBy)
      ? sortBy
      : "created_at";
    const safeSortOrder = sortOrder === "asc" ? "ASC" : "DESC";

    // Get total count
    const countResult = await this.pool.query(
      "SELECT COUNT(*) FROM orders WHERE user_id = $1",
      [userId]
    );
    const total = parseInt(countResult.rows[0].count, 10);

    // Get paginated results
    const result = await this.pool.query(
      `SELECT * FROM orders WHERE user_id = $1 ORDER BY ${safeSortBy} ${safeSortOrder} LIMIT $2 OFFSET $3`,
      [userId, limit, offset]
    );

    const orders = result.rows.map((row) => ({
      order_id: row.order_id,
      instrument_id: parseInt(row.instrument_id, 10),
      user_id: parseInt(row.user_id, 10),
      side: row.side,
      type: row.type,
      price: parseInt(row.price, 10),
      qty: parseInt(row.qty, 10),
      qty_filled: parseInt(row.qty_filled, 10),
      qty_remaining: parseInt(row.qty_remaining, 10),
      status: row.status,
      created_at: row.created_at,
      updated_at: row.updated_at,
    }));

    return { orders, total };
  }

  async getUserOrdersByPair(
    userId: number,
    instrumentId: number,
    page: number = 1,
    limit: number = 20,
    sortBy: string = "created_at",
    sortOrder: "asc" | "desc" = "desc"
  ): Promise<{ orders: Array<Order>; total: number }> {
    const offset = (page - 1) * limit;

    // Validate sort column to prevent SQL injection
    const allowedSortColumns = [
      "created_at",
      "updated_at",
      "price",
      "qty",
      "status",
    ];
    const safeSortBy = allowedSortColumns.includes(sortBy)
      ? sortBy
      : "created_at";
    const safeSortOrder = sortOrder === "asc" ? "ASC" : "DESC";

    // Get total count
    const countResult = await this.pool.query(
      "SELECT COUNT(*) FROM orders WHERE user_id = $1 AND instrument_id = $2",
      [userId, instrumentId]
    );
    const total = parseInt(countResult.rows[0].count, 10);

    // Get paginated results
    const result = await this.pool.query(
      `SELECT * FROM orders WHERE user_id = $1 AND instrument_id = $2 ORDER BY ${safeSortBy} ${safeSortOrder} LIMIT $3 OFFSET $4`,
      [userId, instrumentId, limit, offset]
    );

    const orders = result.rows.map((row) => ({
      order_id: row.order_id,
      instrument_id: parseInt(row.instrument_id, 10),
      user_id: parseInt(row.user_id, 10),
      side: row.side,
      type: row.type,
      price: parseInt(row.price, 10),
      qty: parseInt(row.qty, 10),
      qty_filled: parseInt(row.qty_filled, 10),
      qty_remaining: parseInt(row.qty_remaining, 10),
      status: row.status,
      created_at: row.created_at,
      updated_at: row.updated_at,
    }));

    return { orders, total };
  }

  async getOrderbook(
    symbol: string,
    levels: number,
    groupTicks: number
  ): Promise<
    Array<{
      side: "bid" | "ask";
      price: number;
      qty: number;
    }>
  > {
    console.log("Getting orderbook for", symbol, levels, groupTicks);
    const result = await this.pool.query(
      "SELECT * FROM get_orderbook_grouped_by_ticks($1, $2, $3)",
      [symbol, levels, groupTicks]
    );

    // Convert string values to numbers since PostgreSQL returns bigint as strings
    return result.rows.map((row) => ({
      side: row.side,
      price: parseInt(row.price, 10),
      qty: parseInt(row.qty, 10),
    }));
  }

  async getLatestPrice(instrumentId: number): Promise<number> {
    const result = await this.pool.query(
      "SELECT price FROM trade_events WHERE instrument_id = $1 ORDER BY trade_id DESC LIMIT 1",
      [instrumentId]
    );
    return parseInt(result.rows?.[0]?.price, 10) || 0;
  }

  /**
   * Get price change over 24h for a pair
   */
  async getPriceChange(instrumentId: number): Promise<number> {
    const result = await this.pool.query(
      `WITH price_24h AS (
    SELECT price, trade_id FROM trade_events WHERE instrument_id = $1 AND trade_time < now() - interval '24 hours'
        UNION ALL
        SELECT 0 AS price, 0 as trade_id
        ORDER BY trade_id DESC LIMIT 1
        ),
    price_now AS (
        SELECT price FROM trade_events WHERE instrument_id = $1 ORDER BY trade_id DESC LIMIT 1
    )
    SELECT price_now.price - price_24h.price AS price_change FROM price_now, price_24h;
    `,
      [instrumentId]
    );
    return parseInt(result.rows?.[0]?.price_change, 10) || 0;
  }

  async getVolume(instrumentId: number): Promise<number> {
    const result = await this.pool.query(
      `SELECT SUM(qty) FROM trade_events WHERE instrument_id = $1 AND trade_time > now() - interval '24 hours'`,
      [instrumentId]
    );
    return parseInt(result.rows?.[0]?.sum, 10) || 0;
  }

  async getUserTrades(userId: number): Promise<Array<Trade>> {
    const result = await this.pool.query(
      `
      SELECT trade_id, instrument_id, price, qty, trade_time, side FROM trade_events WHERE 
      taker_user_id = $1
      OR maker_user_id = $1;
    `,
      [userId]
    );
    return result.rows.map((row) => ({
      trade_id: parseInt(row.trade_id, 10),
      instrument_id: parseInt(row.instrument_id, 10),
      price: parseInt(row.price, 10),
      qty: parseInt(row.qty, 10),
      trade_time: row.trade_time,
      side: row.side,
    }));
  }

  async getUserTradesByPair(
    userId: number,
    instrumentId: number
  ): Promise<Array<Trade>> {
    const result = await this.pool.query(
      `
      SELECT trade_id, instrument_id, price, qty, trade_time, side FROM trade_events WHERE 
      taker_user_id = $1
      OR maker_user_id = $1
      AND instrument_id = $2;
    `,
      [userId, instrumentId]
    );
    return result.rows.map((row) => ({
      trade_id: parseInt(row.trade_id, 10),
      instrument_id: parseInt(row.instrument_id, 10),
      price: parseInt(row.price, 10),
      qty: parseInt(row.qty, 10),
      trade_time: row.trade_time,
      side: row.side,
    }));
  }

  async getCandlestickData(
    instrumentId: number,
    tFrom: string,
    tTo: string,
    stepSec: number
  ): Promise<
    Array<{
      bucket: string;
      open: number;
      high: number;
      low: number;
      close: number;
      volume_trades: number;
      trade_count: number;
    }>
  > {
    const query = `
      WITH user_params AS (
        SELECT
          $1::bigint        AS instrument_id,
          $2::timestamptz   AS t_from,
          $3::timestamptz   AS t_to,
          $4::int           AS step_sec
      ),
      ft AS (  -- premier trade globalement (ou au moins avant t_to)
        SELECT MIN(te.trade_time) AS first_trade_time
        FROM trade_events te
        JOIN user_params p ON te.instrument_id = p.instrument_id
        WHERE te.trade_time < (SELECT t_to FROM user_params)
      ),
      params AS (
        SELECT
          p.instrument_id,
          GREATEST(
            p.t_from,
            -- si aucun trade avant t_to, on garde t_from (la série sera vide dans ce cas)
            COALESCE(
              to_timestamp(floor(extract(epoch from ft.first_trade_time) / p.step_sec) * p.step_sec),
              p.t_from
            )
          ) AS t_from,
          p.t_to,
          p.step_sec
        FROM user_params p LEFT JOIN ft ON TRUE
      ),
      aligned AS (
        SELECT
          -- aligne t_from AU DÉBUT DE SON BUCKET
          to_timestamp(floor(extract(epoch from t_from) / step_sec) * step_sec) AS from_aligned,
          -- aligne t_to AU DÉBUT DU DERNIER BUCKET DÉMARRANT AVANT t_to (fenêtre [from_aligned, t_to) )
          to_timestamp(floor(extract(epoch from (t_to - interval '1 microsecond')) / step_sec) * step_sec) AS to_aligned
        FROM params
      ),
      -- Série de buckets [t_from, t_to) espacés de step_sec
      series AS (
        SELECT generate_series(
                 (SELECT from_aligned FROM aligned),
                 (SELECT to_aligned FROM aligned),
                 make_interval(secs => (SELECT step_sec FROM params))
               ) AS bucket
      ),
      -- Trades restreints à la fenêtre, avec attribution au bucket
      base AS (
        SELECT
          to_timestamp(floor(extract(epoch from te.trade_time) / p.step_sec) * p.step_sec) AS bucket,
          te.price,
          te.qty,
          te.trade_time
        FROM trade_events te
        JOIN params p ON te.instrument_id = p.instrument_id
        WHERE te.trade_time >= p.t_from
          AND te.trade_time <  p.t_to
      ),
      -- Agrégats OHLC par bucket
      agg AS (
        SELECT
          b.bucket,
          (ARRAY_AGG(b.price ORDER BY b.trade_time ASC ))[1]  AS open_first_trade,
          MAX(b.price)                                        AS high_trade,
          MIN(b.price)                                        AS low_trade,
          (ARRAY_AGG(b.price ORDER BY b.trade_time DESC))[1]  AS close_trade,
          SUM(b.qty)                                          AS volume_trades,
          COUNT(*)                                            AS trade_count
        FROM base b
        GROUP BY b.bucket
      ),
      -- Dernier close AVANT la fenêtre (pour remplir les tout premiers buckets s'il n'y a pas encore eu de trade)
      prev_close AS (
        SELECT te.price AS prev_close
        FROM trade_events te
        JOIN params p ON te.instrument_id = p.instrument_id
        WHERE te.trade_time < p.t_from
        ORDER BY te.trade_time DESC
        LIMIT 1
      ),
      -- Joindre série complète et calculer "dernier bucket avec trade" en cumul
      series_with_last AS (
        SELECT
          s.bucket,
          a.high_trade, a.low_trade, a.close_trade,
          a.volume_trades, a.trade_count,
          -- bucket du dernier trade connu jusqu'à CE bucket (NULL s'il n'y en a pas encore)
          MAX(CASE WHEN a.close_trade IS NOT NULL THEN s.bucket END)
            OVER (ORDER BY s.bucket ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW)
            AS last_bucket_with_trade
        FROM series s
        LEFT JOIN agg a USING (bucket)
      ),
      filled AS (
        SELECT
          swl.bucket,
          -- forward-filled reference close
          COALESCE(swl.close_trade, last_a.close_trade, pc.prev_close) AS close_ff,
          -- for high/low: use trade highs/lows if present, else flat at reference close
          COALESCE(swl.high_trade, COALESCE(last_a.close_trade, pc.prev_close)) AS high_ff,
          COALESCE(swl.low_trade,  COALESCE(last_a.close_trade, pc.prev_close)) AS low_ff,
          COALESCE(swl.volume_trades, 0) AS volume_trades,
          COALESCE(swl.trade_count,  0) AS trade_count,
          pc.prev_close,
          last_a.open_first_trade
        FROM series_with_last swl
        LEFT JOIN agg last_a
          ON last_a.bucket = swl.last_bucket_with_trade
        LEFT JOIN prev_close pc ON TRUE
      )
      -- final: OPEN = previous bucket's CLOSE (lag of close_ff), first falls back to prev_close
      SELECT
        f.bucket,
        COALESCE(LAG(f.close_ff) OVER (ORDER BY f.bucket), f.open_first_trade) AS open,
        f.high_ff  AS high,
        f.low_ff   AS low,
        f.close_ff AS close,
        f.volume_trades,
        f.trade_count
      FROM filled f
      ORDER BY f.bucket;
    `;

    const result = await this.pool.query(query, [
      instrumentId,
      tFrom,
      tTo,
      stepSec,
    ]);

    return result.rows.map((row) => ({
      bucket: row.bucket,
      open: parseInt(row.open, 10),
      high: parseInt(row.high, 10),
      low: parseInt(row.low, 10),
      close: parseInt(row.close, 10),
      volume_trades: parseInt(row.volume_trades, 10),
      trade_count: parseInt(row.trade_count, 10),
    }));
  }

  async healthCheck(): Promise<boolean> {
    try {
      await this.pool.query("SELECT 1");
      return true;
    } catch {
      return false;
    }
  }
}
