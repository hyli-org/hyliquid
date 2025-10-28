/**
 * WebSocket types and interfaces
 */

export interface WebSocketSubscription {
  type: string;
  instrument: string;
}

export interface WebSocketMessage {
  method: "subscribe" | "unsubscribe";
  subscription: WebSocketSubscription;
}

export interface WebSocketResponse {
  type: string;
  instrument: string;
  data: any;
  timestamp: number;
}

export interface L2BookSubscription extends WebSocketSubscription {
  type: "l2Book";
  groupTicks: number;
}

export interface TradesSubscription extends WebSocketSubscription {
  type: "trades";
  user: string;
}

export interface OrdersSubscription extends WebSocketSubscription {
  type: "orders";
  user: string;
}

export interface CandlestickSubscription extends WebSocketSubscription {
  type: "candlestick";
  stepSec: number;
}

export interface L2BookEntry {
  price: number;
  quantity: number;
}

export interface L2BookData {
  bids: Array<L2BookEntry>;
  asks: Array<L2BookEntry>;
}

export interface ClientConnection {
  id: string;
  ws: any; // WebSocket connection
  subscriptions: Map<string, WebSocketSubscription>;
  messageQueue: any[];
  isProcessing: boolean;
}

export interface ChannelManager {
  clients: Map<string, ClientConnection>;
  intervals: Map<string, NodeJS.Timeout>;
}
