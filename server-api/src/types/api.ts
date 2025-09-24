/**
 * API request and response types
 */

import { Order, Trade } from "./orderbook";

export interface ConfigResponse {
  contract_name: string;
}

export interface BalanceResponse {
  token: string;
  total: number;
  reserved: number;
  available: number;
}

export interface UserBalances {
  balances: BalanceResponse[];
}

export interface UserOrders {
  orders: Order[];
}

export interface PaginatedUserOrders extends PaginatedResponse<Order> {}

export interface UserTrades {
  trades: Trade[];
}

export interface GetBookQuery {
  levels?: number;
  group_ticks?: number;
}

export interface PaginationQuery {
  page?: number;
  limit?: number;
  sort_by?: string;
  sort_order?: 'asc' | 'desc';
}

export interface PaginatedResponse<T> {
  data: T[];
  pagination: {
    page: number;
    limit: number;
    total: number;
    total_pages: number;
    has_next: boolean;
    has_prev: boolean;
  };
}

export interface AuthHeaders {
  user: string;
}

export interface AppError {
  status: number;
  message: string;
}
