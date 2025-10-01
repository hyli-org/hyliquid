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
    return result.rows;
  }

  async getAllInstruments(): Promise<Instrument[]> {
    const result = await this.pool.query(
      "SELECT * FROM instruments ORDER BY symbol"
    );
    return result.rows;
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
      instrument_id: row.instrument_id,
      user_id: row.user_id,
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
      instrument_id: row.instrument_id,
      user_id: row.user_id,
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
      trade_id: row.trade_id,
      instrument_id: row.instrument_id,
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
      trade_id: row.trade_id,
      instrument_id: row.instrument_id,
      price: parseInt(row.price, 10),
      qty: parseInt(row.qty, 10),
      trade_time: row.trade_time,
      side: row.side,
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
