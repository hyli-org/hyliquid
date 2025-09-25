import { reactive } from "vue";

export interface OrderbookEntry {
    price: number;
    quantity: number;
}

export interface WebSocketMessage {
    type: string;
    instrument?: string;
    data?: {
        bids: OrderbookEntry[];
        asks: OrderbookEntry[];
    };
    message?: string;
    timestamp?: number;
}

export interface Subscription {
    type: string;
    instrument: string;
    groupTicks: number;
}

export interface WebSocketState {
    connected: boolean;
    error: string | null;
    bids: OrderbookEntry[];
    asks: OrderbookEntry[];
    mid: number;
}

class WebSocketManager {
    private ws: WebSocket | null = null;
    private currentSubscription: Subscription | null = null;
    private reconnectTimeout: number | null = null;
    private url: string;

    public state = reactive<WebSocketState>({
        connected: false,
        error: null,
        bids: [],
        asks: [],
        mid: 0,
    });

    constructor(url: string = "ws://localhost:3000/ws") {
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
        this.currentSubscription = null;
    }

    subscribeToOrderbook(instrument: string, groupTicks: number = 1): void {
        if (!this.ws || this.ws.readyState !== WebSocket.OPEN) {
            console.warn("WebSocket not connected, cannot subscribe");
            return;
        }

        // Unsubscribe from previous subscription
        if (this.currentSubscription) {
            this.unsubscribe();
        }

        const subscription: Subscription = {
            type: "l2Book",
            instrument: instrument.toLowerCase(),
            groupTicks,
        };

        const message = {
            method: "subscribe",
            subscription,
        };

        this.ws.send(JSON.stringify(message));
        this.currentSubscription = subscription;

        console.log("Subscribed to orderbook:", instrument);
    }

    unsubscribe(): void {
        if (!this.ws || this.ws.readyState !== WebSocket.OPEN || !this.currentSubscription) {
            return;
        }

        const message = {
            method: "unsubscribe",
            subscription: this.currentSubscription,
        };

        this.ws.send(JSON.stringify(message));
        this.currentSubscription = null;

        console.log("Unsubscribed from orderbook");
    }

    private handleMessage(event: MessageEvent): void {
        try {
            const data: WebSocketMessage = JSON.parse(event.data);

            if (data.type === "l2Book" && data.data) {
                this.state.bids = data.data.bids;
                this.state.asks = data.data.asks;

                // Calculate mid price
                if (data.data.bids.length > 0 && data.data.asks.length > 0) {
                    this.state.mid = (data.data.bids[0]!.price + data.data.asks[0]!.price) / 2;
                }
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
