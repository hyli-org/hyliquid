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
  async getUserById(identity: string): Promise<User | null> {
    return this.queries.getUserById(identity);
  }

  /**
   * Get user balances
   */
  async getBalances(user: string): Promise<UserBalances> {
    const balanceRows = await this.queries.getUserBalances(user);

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
    const nonce = await this.queries.getUserNonce(user);
    return nonce;
  }

  /**
   * Get user orders with pagination
   */
  async getOrdersPaginated(
    user: string,
    pagination: PaginationQuery = {}
  ): Promise<PaginatedUserOrders> {
    const page = pagination.page || 1;
    const limit = Math.min(pagination.limit || 20, 100); // Cap at 100 items per page
    const sortBy = pagination.sort_by || "created_at";
    const sortOrder = pagination.sort_order || "desc";

    const { orders, total } = await this.queries.getUserOrders(
      user,
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
    const page = pagination.page || 1;
    const limit = Math.min(pagination.limit || 20, 100); // Cap at 100 items per page
    const sortBy = pagination.sort_by || "created_at";
    const sortOrder = pagination.sort_order || "desc";

    const { orders, total } = await this.queries.getUserOrdersByPair(
      user,
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
    const trades = await this.queries.getUserTrades(user);
    return { trades };
  }

  /**
   * Get user trades by pair
   */
  async getTradesByPair(
    identity: string,
    instrumentId: number
  ): Promise<UserTrades> {
    const trades = await this.queries.getUserTradesByPair(
      identity,
      instrumentId
    );
    return { trades };
  }

  /**
   * Check if a user exists
   */
  hasUser(user: string): boolean {
    return this.queries.getUserById(user) !== null;
  }
}
