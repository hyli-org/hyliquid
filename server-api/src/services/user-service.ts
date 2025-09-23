/**
 * User service with in-memory caching
 */

import { UserBalances, BalanceResponse, UserOrders, UserTrades } from '../types';
import { DatabaseQueries } from '../config/database';

export class UserService {
  private userIdMap: Map<string, number> = new Map();
  private queries: DatabaseQueries;

  constructor(queries: DatabaseQueries) {
    this.queries = queries;
  }

  /**
   * Initialize the service by loading all users into memory
   */
  async initialize(): Promise<void> {
    console.log('Loading users into memory...');
    
    const users = await this.queries.getAllUsers();
    this.userIdMap.clear();
    
    for (const user of users) {
      this.userIdMap.set(user.identity, user.user_id);
    }

    console.log(`Loaded ${this.userIdMap.size} users into memory`);
  }

  /**
   * Get user ID by identity, with fallback to database lookup
   */
  async getUserId(user: string): Promise<number> {
    // Check in-memory cache first
    const cachedUserId = this.userIdMap.get(user);
    if (cachedUserId !== undefined) {
      return cachedUserId;
    }

    // Fallback to database lookup
    const userRecord = await this.queries.getUserByIdentity(user);
    if (!userRecord) {
      throw new Error(`User not found: ${user}`);
    }

    // Cache the result for future lookups
    this.userIdMap.set(user, userRecord.user_id);
    return userRecord.user_id;
  }

  /**
   * Get user balances
   */
  async getBalances(user: string): Promise<UserBalances> {
    const userId = await this.getUserId(user);
    const balanceRows = await this.queries.getUserBalances(userId);

    const balances: BalanceResponse[] = balanceRows.map(row => ({
      token: row.symbol,
      total: row.total,
      reserved: row.reserved,
      available: row.available,
    }));

    return { balances };
  }

  /**
   * Get user orders
   */
  async getOrders(user: string): Promise<UserOrders> {
    const userId = await this.getUserId(user);
    const orders = await this.queries.getUserOrders(userId);
    return { orders };
  }

  /**
   * Get user orders by pair
   */
  async getOrdersByPair(user: string, instrumentId: number): Promise<UserOrders> {
    const userId = await this.getUserId(user);
    const orders = await this.queries.getUserOrdersByPair(userId, instrumentId);
    return { orders };
  }

  /**
   * Get user trades
   */
  async getTrades(user: string): Promise<UserTrades> {
    const userId = await this.getUserId(user);
    const trades = await this.queries.getUserTrades(userId);
    return { trades };
  }

  /**
   * Get user trades by pair
   */
  async getTradesByPair(user: string, instrumentId: number): Promise<UserTrades> {
    const userId = await this.getUserId(user);
    const trades = await this.queries.getUserTradesByPair(userId, instrumentId);
    return { trades };
  }

  /**
   * Check if a user exists
   */
  hasUser(user: string): boolean {
    return this.userIdMap.has(user);
  }

  /**
   * Get user count
   */
  getUserCount(): number {
    return this.userIdMap.size;
  }

  /**
   * Manually add a user to the cache (useful for testing or real-time updates)
   */
  cacheUser(identity: string, userId: number): void {
    this.userIdMap.set(identity, userId);
  }

  /**
   * Remove a user from the cache
   */
  removeUserFromCache(identity: string): void {
    this.userIdMap.delete(identity);
  }
}
