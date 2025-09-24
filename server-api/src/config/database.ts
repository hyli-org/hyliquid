/**
 * Database configuration and connection setup
 */

import { Pool, PoolConfig } from 'pg';
import { Asset, Instrument, Order, Trade } from '../types';

export class DatabaseConfig {
  private static instance: DatabaseConfig;
  private pool: Pool;

  private constructor() {
    const config: PoolConfig = {
      connectionString: process.env.HYLI_DATABASE_URL || 'postgresql://postgres:postgres@localhost:5432/orderbook',
      max: 20, // Maximum number of clients in the pool
      idleTimeoutMillis: 30000, // Close idle clients after 30 seconds
      connectionTimeoutMillis: 2000, // Return an error after 2 seconds if connection could not be established
    };

    this.pool = new Pool(config);

    // Handle pool errors
    this.pool.on('error', (err: Error) => {
      console.error('Unexpected error on idle client', err);
    });
  }

  public static getInstance(): DatabaseConfig {
    if (!DatabaseConfig.instance) {
      DatabaseConfig.instance = new DatabaseConfig();
    }
    return DatabaseConfig.instance;
  }

  public getPool(): Pool {
    return this.pool;
  }

  public async testConnection(): Promise<boolean> {
    try {
      const client = await this.pool.connect();
      await client.query('SELECT 1');
      client.release();
      return true;
    } catch (error) {
      console.error('Database connection test failed:', error);
      return false;
    }
  }

  public async close(): Promise<void> {
    await this.pool.end();
  }
}

/**
 * Database query helpers
 */
export class DatabaseQueries {
  private pool: Pool;

  constructor(pool: Pool) {
    this.pool = pool;
  }

  async getAllAssets(): Promise<Asset[]> {
    const result = await this.pool.query('SELECT * FROM assets ORDER BY symbol');
    return result.rows;
  }

  async getAllInstruments(): Promise<Instrument[]> {
    const result = await this.pool.query('SELECT * FROM instruments ORDER BY symbol');
    return result.rows;
  }

  async getAllUsers(): Promise<Array<{ identity: string; user_id: number }>> {
    const result = await this.pool.query('SELECT identity, user_id FROM users');
    return result.rows;
  }

  async getUserByIdentity(identity: string): Promise<{ user_id: number } | null> {
    const result = await this.pool.query(
      'SELECT user_id FROM users WHERE identity = $1',
      [identity]
    );
    return result.rows[0] || null;
  }

  async getUserBalances(userId: number): Promise<Array<{
    symbol: string;
    total: number;
    reserved: number;
    available: number;
  }>> {
    const result = await this.pool.query(`
      SELECT 
        assets.symbol, balances.total, balances.reserved, balances.available 
      FROM 
        balances
      JOIN 
        assets ON balances.asset_id = assets.asset_id
      WHERE 
        balances.user_id = $1
    `, [userId]);
    return result.rows.map(row => ({
      symbol: row.symbol,
      total: parseInt(row.total, 10),
      reserved: parseInt(row.reserved, 10),
      available: parseInt(row.available, 10)
    }));
  }

  async getUserOrders(userId: number): Promise<Array<Order>> {
    const result = await this.pool.query('SELECT * FROM orders WHERE user_id = $1', [userId]);
    return result.rows.map(row => ({
      order_id: row.order_id,
      order_signed_id: row.order_signed_id,
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
      updated_at: row.updated_at
    }));
  }

  async getUserOrdersByPair(userId: number, instrumentId: number): Promise<Array<Order>> {
    const result = await this.pool.query('SELECT * FROM orders WHERE user_id = $1 AND instrument_id = $2', [userId, instrumentId]);
    return result.rows.map(row => ({
      order_id: row.order_id,
      order_signed_id: row.order_signed_id,
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
      updated_at: row.updated_at
    }));
  }

  async getOrderbook(symbol: string, levels: number, groupTicks: number): Promise<Array<{
    side: 'bid' | 'ask';
    price: number;
    qty: number;
  }>> {
    const result = await this.pool.query(
      'SELECT * FROM get_orderbook_grouped_by_ticks($1, $2, $3)',
      [symbol, levels, groupTicks]
    );
    
    // Convert string values to numbers since PostgreSQL returns bigint as strings
    return result.rows.map(row => ({
      side: row.side,
      price: parseInt(row.price, 10),
      qty: parseInt(row.qty, 10)
    }));
  }

  async getLatestPrice(instrumentId: number): Promise<number> {
    const result = await this.pool.query('SELECT price FROM trades WHERE instrument_id = $1 ORDER BY trade_id DESC LIMIT 1', [instrumentId]);
    return parseInt(result.rows?.[0]?.price, 10) || 0;
  }

  /** 
   * Get price change over 24h for a pair
   */
  async getPriceChange(instrumentId: number): Promise<number> {
    const result = await this.pool.query(`WITH price_24h AS (
    SELECT price, trade_id FROM trades WHERE instrument_id = $1 AND trade_time < now() - interval '24 hours'
        UNION ALL
        SELECT 0 AS price, 0 as trade_id
        ORDER BY trade_id DESC LIMIT 1
        ),
    price_now AS (
        SELECT price FROM trades WHERE instrument_id = $1 ORDER BY trade_id DESC LIMIT 1
    )
    SELECT price_now.price - price_24h.price AS price_change FROM price_now, price_24h;
    `, [instrumentId]);
    return parseInt(result.rows?.[0]?.price_change, 10) || 0;
  }

  async getVolume(instrumentId: number): Promise<number> {
    const result = await this.pool.query(`SELECT SUM(qty) FROM trades WHERE instrument_id = $1 AND trade_time > now() - interval '24 hours'`, [instrumentId]);
    return parseInt(result.rows?.[0]?.sum, 10) || 0;
  }

  async getUserTrades(userId: number): Promise<Array<Trade>> {
    const result = await this.pool.query(`
      WITH ids AS(
          SELECT order_signed_id FROM order_signed_ids WHERE user_id = $1
      )
      SELECT trade_id, instrument_id, price, qty, trade_time, side FROM trades WHERE 
      taker_order_signed_id IN(select order_signed_id from ids)
      OR maker_order_signed_id IN(select order_signed_id from ids);
    `, [userId]);
    return result.rows.map(row => ({
      trade_id: row.trade_id,
      instrument_id: row.instrument_id,
      price: parseInt(row.price, 10),
      qty: parseInt(row.qty, 10),
      trade_time: row.trade_time,
      side: row.side
    }));
  }

  async getUserTradesByPair(userId: number, instrumentId: number): Promise<Array<Trade>> {
    const result = await this.pool.query(`
      WITH ids AS(
          SELECT order_signed_id FROM order_signed_ids WHERE user_id = $1
      )
      SELECT trade_id, instrument_id, price, qty, trade_time, side FROM trades WHERE 
      taker_order_signed_id IN(select order_signed_id from ids)
      OR maker_order_signed_id IN(select order_signed_id from ids)
      AND instrument_id = $2;
    `, [userId, instrumentId]);
    return result.rows.map(row => ({
      trade_id: row.trade_id,
      instrument_id: row.instrument_id,
      price: parseInt(row.price, 10),
      qty: parseInt(row.qty, 10),
      trade_time: row.trade_time,
      side: row.side
    }));
  }

  async healthCheck(): Promise<boolean> {
    try {
      await this.pool.query('SELECT 1');
      return true;
    } catch {
      return false;
    }
  }
}
