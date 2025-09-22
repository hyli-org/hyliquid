/**
 * Database configuration and connection setup
 */

import { Pool, PoolConfig } from 'pg';
import { Asset, Instrument } from '../types';

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
    return result.rows;
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
    return result.rows;
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
