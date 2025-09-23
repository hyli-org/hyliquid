/**
 * WebSocket types and interfaces
 */

export interface WebSocketSubscription {
  type: string;
  instrument: string;
  groupTicks?: number;
  levels?: number;
}

export interface WebSocketMessage {
  method: 'subscribe' | 'unsubscribe';
  subscription: WebSocketSubscription;
}

export interface WebSocketResponse {
  type: string;
  instrument: string;
  data: any;
  timestamp: number;
}

export interface L2BookSubscription extends WebSocketSubscription {
  type: 'l2Book';
  groupTicks: number;
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
}

export interface ChannelManager {
  clients: Map<string, ClientConnection>;
  intervals: Map<string, NodeJS.Timeout>;
}
