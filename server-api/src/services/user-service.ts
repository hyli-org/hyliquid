/**
 * User service with in-memory caching
 */

import {
  UserBalances,
  BalanceResponse,
  UserTrades,
  PaginatedUserOrders,
  PaginationQuery,
  User,
} from "../types";
import { DatabaseQueries } from "../database/queries";
import { CustomError } from "@/middleware";

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
    console.log("Loading users into memory...");

    const users = await this.queries.getAllUsers();
    this.userIdMap.clear();

    for (const user of users) {
      this.userIdMap.set(user.identity, user.user_id);
    }

    console.log(`Loaded ${this.userIdMap.size} users into memory`);
  }

  /**
   * Get user by identity
   */
  async getUserById(userId: number): Promise<User | null> {
    return this.queries.getUserById(userId);
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
      throw new CustomError(`User not found: ${user}`, 404);
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

    const balances: BalanceResponse[] = balanceRows.map((row) => ({
      token: row.symbol,
      total: row.total,
      reserved: row.reserved,
      available: row.available,
    }));

    return { balances };
  }

  /**
   * Get user nonce
   */
  async getNonce(user: string): Promise<number> {
    const userId = await this.getUserId(user);
    const nonce = await this.queries.getUserNonce(userId);
    return nonce;
  }

  /**
   * Get user orders with pagination
   */
  async getOrdersPaginated(
    user: string,
    pagination: PaginationQuery = {}
  ): Promise<PaginatedUserOrders> {
    const userId = await this.getUserId(user);
    const page = pagination.page || 1;
    const limit = Math.min(pagination.limit || 20, 100); // Cap at 100 items per page
    const sortBy = pagination.sort_by || "created_at";
    const sortOrder = pagination.sort_order || "desc";

    const { orders, total } = await this.queries.getUserOrders(
      userId,
      page,
      limit,
      sortBy,
      sortOrder
    );

    const totalPages = Math.ceil(total / limit);

    return {
      data: orders,
      pagination: {
        page,
        limit,
        total,
        total_pages: totalPages,
        has_next: page < totalPages,
        has_prev: page > 1,
      },
    };
  }

  /**
   * Get user orders by pair with pagination
   */
  async getOrdersByPairPaginated(
    user: string,
    instrumentId: number,
    pagination: PaginationQuery = {}
  ): Promise<PaginatedUserOrders> {
    const userId = await this.getUserId(user);
    const page = pagination.page || 1;
    const limit = Math.min(pagination.limit || 20, 100); // Cap at 100 items per page
    const sortBy = pagination.sort_by || "created_at";
    const sortOrder = pagination.sort_order || "desc";

    const { orders, total } = await this.queries.getUserOrdersByPair(
      userId,
      instrumentId,
      page,
      limit,
      sortBy,
      sortOrder
    );

    const totalPages = Math.ceil(total / limit);

    return {
      data: orders,
      pagination: {
        page,
        limit,
        total,
        total_pages: totalPages,
        has_next: page < totalPages,
        has_prev: page > 1,
      },
    };
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
  async getTradesByPair(
    user: string,
    instrumentId: number
  ): Promise<UserTrades> {
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
