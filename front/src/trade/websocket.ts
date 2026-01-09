import { reactive } from "vue";
import type { Fill, Order, ApiTrade, ApiOrder } from "./trade";
import { transformOrder, transformTrade } from "./api";
import { WEBSOCKET_URL } from "../config";

export interface OrderbookEntry {
    price: number;
    quantity: number;
}

export interface WebSocketMessage {
    type: string;
    instrument?: string;
    data?: {
        bids?: OrderbookEntry[];
        asks?: OrderbookEntry[];
        trades?: ApiTrade[];
        orders?: ApiOrder[];
        candlesticks?: CandlestickApiData[];
    };
    message?: string;
    timestamp?: number;
}

export interface CandlestickApiData {
    bucket: string;
    open: number;
    high: number;
    low: number;
    close: number;
    volume_trades: number;
    trade_count: number;
}

export interface Subscription {
    type: string;
    instrument: string;
    groupTicks?: number;
    stepSec?: number;
    user?: string;
}

export interface WebSocketState {
    connected: boolean;
    error: string | null;
    bids: OrderbookEntry[];
    asks: OrderbookEntry[];
    mid: number;
    fills: Fill[];
    orders: Order[];
}

export interface TradeCallback {
    (trade: { price: number; qty: number; time: number }): void;
}

export interface CandlestickCallback {
    (candlesticks: CandlestickApiData[]): void;
}

class WebSocketManager {
    private ws: WebSocket | null = null;
    private currentBookSubscription: Subscription | null = null;
    private currentTradesSubscription: Subscription | null = null;
    private currentOrdersSubscription: Subscription | null = null;
    private currentCandlestickSubscription: Subscription | null = null;
    private reconnectTimeout: number | null = null;
    private url: string;
    private tradeCallbacks: Set<TradeCallback> = new Set();
    private candlestickCallbacks: Set<CandlestickCallback> = new Set();

    public state = reactive<WebSocketState>({
        connected: false,
        error: null,
        bids: [],
        asks: [],
        mid: 0,
        fills: [],
        orders: [],
    });

    constructor(url: string = WEBSOCKET_URL) {
        this.url = url;
    }

    connect(): void {
        if (this.ws?.readyState === WebSocket.OPEN) return;

        this.ws = new WebSocket(this.url);
        this.state.connected = false;

        this.ws.onopen = () => {
            this.state.connected = true;
            this.state.error = null;
            console.log("WebSocket connected");
        };

        this.ws.onmessage = (event: MessageEvent) => {
            this.handleMessage(event);
        };

        this.ws.onclose = () => {
            this.state.connected = false;
            this.scheduleReconnect();
        };

        this.ws.onerror = (error) => {
            this.state.error = "WebSocket connection error";
            console.error("WebSocket error:", error);
        };
    }

    disconnect(): void {
        if (this.reconnectTimeout) {
            clearTimeout(this.reconnectTimeout);
            this.reconnectTimeout = null;
        }

        if (this.ws) {
            this.ws.close();
            this.ws = null;
        }

        this.state.connected = false;
        this.currentBookSubscription = null;
        this.currentTradesSubscription = null;
        this.currentOrdersSubscription = null;
        this.currentCandlestickSubscription = null;
    }

    // Trade callback management
    onNewTrade(callback: TradeCallback): void {
        this.tradeCallbacks.add(callback);
    }

    offNewTrade(callback: TradeCallback): void {
        this.tradeCallbacks.delete(callback);
    }

    // Candlestick callback management
    onCandlestickUpdate(callback: CandlestickCallback): void {
        this.candlestickCallbacks.add(callback);
    }

    offCandlestickUpdate(callback: CandlestickCallback): void {
        this.candlestickCallbacks.delete(callback);
    }

    private notifyTradeCallbacks(trade: { price: number; qty: number; time: number }): void {
        this.tradeCallbacks.forEach((callback) => {
            try {
                callback(trade);
            } catch (error) {
                console.error("Error in trade callback:", error);
            }
        });
    }

    private notifyCandlestickCallbacks(candlesticks: CandlestickApiData[]): void {
        this.candlestickCallbacks.forEach((callback) => {
            try {
                callback(candlesticks);
            } catch (error) {
                console.error("Error in candlestick callback:", error);
            }
        });
    }

    subscribeTo(subscription: Subscription): void {
        if (!this.ws || this.ws.readyState !== WebSocket.OPEN) {
            console.warn("WebSocket not connected, cannot subscribe");
            return;
        }

        const message = {
            method: "subscribe",
            subscription,
        };

        this.ws.send(JSON.stringify(message));
    }

    subscribeToOrderbook(instrument: string, groupTicks: number): void {
        // Unsubscribe from previous subscription
        if (this.currentBookSubscription) {
            this.unsubscribe();
        }

        const subscription: Subscription = {
            type: "l2Book",
            instrument: instrument.toLowerCase(),
            groupTicks,
        };

        this.subscribeTo(subscription);

        this.currentBookSubscription = subscription;

        console.log("Subscribed to orderbook:", instrument);
    }

    subscribeToTrades(instrument: string, user: string): void {
        // Unsubscribe from previous trades subscription
        if (this.currentTradesSubscription) {
            this.unsubscribeTrades();
        }

        const subscription: Subscription = {
            type: "trades",
            instrument: instrument.toLowerCase(),
            user: user.toLowerCase(),
        };

        this.subscribeTo(subscription);
        this.currentTradesSubscription = subscription;
        console.log("Subscribed to trades:", instrument);
    }

    subscribeToOrders(instrument: string, user: string): void {
        // Unsubscribe from previous orders subscription
        if (this.currentOrdersSubscription) {
            this.unsubscribeOrders();
        }

        const subscription: Subscription = {
            type: "orders",
            instrument: instrument.toLowerCase(),
            user: user.toLowerCase(),
        };

        this.subscribeTo(subscription);
        this.currentOrdersSubscription = subscription;
        console.log("Subscribed to orders:", instrument);
    }

    subscribeToCandlestick(instrument: string, stepSec: number): void {
        // Unsubscribe from previous candlestick subscription
        if (this.currentCandlestickSubscription) {
            this.unsubscribeCandlestick();
        }

        const subscription: Subscription = {
            type: "candlestick",
            instrument: instrument.toLowerCase(),
            stepSec,
        };

        this.subscribeTo(subscription);
        this.currentCandlestickSubscription = subscription;
        console.log("Subscribed to candlestick:", instrument, stepSec);
    }

    unsubscribe(): void {
        if (!this.ws || this.ws.readyState !== WebSocket.OPEN || !this.currentBookSubscription) {
            return;
        }

        const message = {
            method: "unsubscribe",
            subscription: this.currentBookSubscription,
        };

        this.ws.send(JSON.stringify(message));
        this.currentBookSubscription = null;

        console.log("Unsubscribed from orderbook");
    }

    unsubscribeCandlestick(): void {
        if (!this.ws || this.ws.readyState !== WebSocket.OPEN || !this.currentCandlestickSubscription) {
            return;
        }

        const message = {
            method: "unsubscribe",
            subscription: this.currentCandlestickSubscription,
        };

        this.ws.send(JSON.stringify(message));
        this.currentCandlestickSubscription = null;

        console.log("Unsubscribed from candlestick");
    }

    unsubscribeTrades(): void {
        if (!this.ws || this.ws.readyState !== WebSocket.OPEN || !this.currentTradesSubscription) {
            return;
        }

        const message = {
            method: "unsubscribe",
            subscription: this.currentTradesSubscription,
        };

        this.ws.send(JSON.stringify(message));
        this.currentTradesSubscription = null;

        console.log("Unsubscribed from trades");
    }

    unsubscribeOrders(): void {
        if (!this.ws || this.ws.readyState !== WebSocket.OPEN || !this.currentOrdersSubscription) {
            return;
        }

        const message = {
            method: "unsubscribe",
            subscription: this.currentOrdersSubscription,
        };

        this.ws.send(JSON.stringify(message));
        this.currentOrdersSubscription = null;

        console.log("Unsubscribed from orders");
    }

    private handleMessage(event: MessageEvent): void {
        try {
            const data: WebSocketMessage = JSON.parse(event.data);

            if (data.type === "l2Book" && data.data) {
                this.state.bids = data.data.bids || [];
                this.state.asks = data.data.asks || [];

                // Calculate mid price
                if (data.data.bids && data.data.asks && data.data.bids.length > 0 && data.data.asks.length > 0) {
                    this.state.mid = (data.data.bids[0]!.price + data.data.asks[data.data.asks.length - 1]!.price) / 2;
                }
            } else if (data.type === "trades" && data.data) {
                console.log("Received trades:", data.data.trades);
                const fills = data.data.trades?.map((trade: any) => {
                    return transformTrade(trade);
                });
                this.state.fills = fills || [];

                // Notify trade callbacks with the latest trades
                if (data.data.trades && data.data.trades.length > 0) {
                    data.data.trades.forEach((trade: any) => {
                        // Convert API trade to callback format
                        const tradeData = {
                            price: trade.price,
                            qty: trade.qty,
                            time: Math.floor(new Date(trade.trade_time).getTime() / 1000), // Convert to Unix timestamp
                        };
                        this.notifyTradeCallbacks(tradeData);
                    });
                }
            } else if (data.type === "orders" && data.data) {
                console.log("Received orders:", data.data.orders);
                const orders = data.data.orders?.map((order: any) => {
                    return transformOrder(order);
                });
                this.state.orders = orders || [];
            } else if (data.type === "candlestick" && data.data) {
                this.notifyCandlestickCallbacks(data.data.candlesticks || []);
            } else if (data.type === "error") {
                this.state.error = data.message || "Unknown WebSocket error";
            }
        } catch (error) {
            console.error("WebSocket message parse error:", error);
            this.state.error = "Failed to parse WebSocket message";
        }
    }

    private scheduleReconnect(): void {
        if (this.reconnectTimeout) return;

        this.reconnectTimeout = window.setTimeout(() => {
            console.log("Attempting to reconnect WebSocket...");
            this.connect();
            this.reconnectTimeout = null;
        }, 3000);
    }
}

// Create singleton instance
export const websocketManager = new WebSocketManager();

// Initialize connection
websocketManager.connect();
