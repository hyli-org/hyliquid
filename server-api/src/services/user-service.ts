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
  private queries: DatabaseQueries;

  constructor(queries: DatabaseQueries) {
    this.queries = queries;
  }

  /**
   * Initialize the service by loading all users into memory
   */
  async initialize(): Promise<void> {}

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
    const userRecord = await this.queries.getUserByIdentity(user);
    if (!userRecord) {
      throw new CustomError(`User not found: ${user}`, 404);
    }

    return userRecord.user_id;
  }

  /**
   * Get user balances
   */
  async getBalances(user: string): Promise<UserBalances> {
    const userId = await this.getUserId(user);
    const balanceRows = await this.queries.getUserBalances(userId);

    const balances: BalanceResponse[] = balanceRows.map((row) => ({
      symbol: row.symbol,
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
    return this.queries.getUserByIdentity(user) !== null;
  }
}
