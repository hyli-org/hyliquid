/**
 * Database configuration and connection setup
 */

import { Pool, PoolConfig } from "pg";
import { Asset, Instrument, Order, Trade } from "../types";

export class DatabaseConfig {
  private static instance: DatabaseConfig;
  private pool: Pool;

  private constructor() {
    const config: PoolConfig = {
      connectionString:
        process.env.HYLI_DATABASE_URL ||
        "postgresql://postgres:postgres@localhost:5432/orderbook",
      max: 20, // Maximum number of clients in the pool
      idleTimeoutMillis: 30000, // Close idle clients after 30 seconds
      connectionTimeoutMillis: 2000, // Return an error after 2 seconds if connection could not be established
    };

    this.pool = new Pool(config);

    // Handle pool errors
    this.pool.on("error", (err: Error) => {
      console.error("Unexpected error on idle client", err);
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
      await client.query("SELECT 1");
      client.release();
      return true;
    } catch (error) {
      console.error("Database connection test failed:", error);
      return false;
    }
  }

  public async close(): Promise<void> {
    await this.pool.end();
  }
}
