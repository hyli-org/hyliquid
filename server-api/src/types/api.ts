/**
 * API request and response types
 */

import { Order } from "./database";

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

export interface GetBookQuery {
  levels?: number;
  group_ticks?: number;
}

export interface AuthHeaders {
  user: string;
}

export interface AppError {
  status: number;
  message: string;
}
