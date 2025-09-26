/**
 * Database entity types matching the PostgreSQL schema
 */

export interface Asset {
  asset_id: number;
  symbol: string;
  scale: number;
  step: number;
  status: string;
  created_at: Date;
}

export interface Instrument {
  instrument_id: number;
  symbol: string;
  tick_size: number;
  qty_step: number;
  base_asset_id: number;
  quote_asset_id: number;
  status: MarketStatus;
  created_at: Date;
}

export interface User {
  user_id: number;
  identity: string;
  status: UserStatus;
  created_at: Date;
}

export interface Balance {
  user_id: number;
  asset_id: number;
  total: number;
  reserved: number;
  available: number;
  updated_at: Date;
}

export interface Order {
  order_signed_id: string;
  instrument_id: number;
  user_id: number;
  side: OrderSide;
  type: OrderType;
  price: number | null;
  qty: number;
  qty_filled: number;
  qty_remaining: number;
  status: OrderStatus;
  created_at: Date;
  updated_at: Date;
}

export interface Trade {
  trade_id: number;
  instrument_id: number;
  price: number;
  qty: number;
  trade_time: Date;
  side: OrderSide;
}

// Enums
export enum MarketStatus {
  ACTIVE = "active",
  HALTED = "halted",
  CLOSED = "closed",
}

export enum UserStatus {
  ACTIVE = "active",
  SUSPENDED = "suspended",
  CLOSED = "closed",
}

export enum OrderSide {
  BID = "bid",
  ASK = "ask",
}

export enum OrderType {
  LIMIT = "limit",
  MARKET = "market",
  STOP_LIMIT = "stop_limit",
  STOP_MARKET = "stop_market",
}

export enum OrderStatus {
  OPEN = "open",
  PARTIALLY_FILLED = "partially_filled",
  FILLED = "filled",
  CANCELLED = "cancelled",
  REJECTED = "rejected",
}
