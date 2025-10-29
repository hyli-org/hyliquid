/**
 * Elysia native WebSocket service for real-time data streaming
 */

import {
  WebSocketResponse,
  ClientConnection,
  ChannelManager,
  L2BookSubscription,
  TradesSubscription,
  OrdersSubscription,
  CandlestickSubscription,
  L2BookData,
  WebSocketSubscription,
  getSubscriptionKey,
} from "../types/websocket";
import { BookService } from "./book-service";
import { Order, Trade, CandlestickData } from "@/types";
import { DatabaseCallbacks } from "@/database/callbacks";
import { CustomError } from "@/middleware";

// Subscription configuration type
type SubscriptionConfig<T> = {
  subscribe: (
    clientId: string,
    subscription: WebSocketSubscription,
    callback: (data: T) => void
  ) => void;
  getInitialData?: (subscription: WebSocketSubscription) => Promise<T>;
};

// Subscription handlers map
type SubscriptionHandlers = {
  [K in WebSocketSubscription["type"]]: SubscriptionConfig<any>;
};

export class WebSocketService {
  private channelManager: ChannelManager;
  private bookService: BookService;
  private databaseCallbacks: DatabaseCallbacks;

  // Subscription configuration
  private readonly subscriptionConfigs: SubscriptionHandlers;

  // Unsubscribe handlers map
  private readonly unsubscribeHandlers = {
    l2Book: (id: string, sub: WebSocketSubscription) =>
      this.databaseCallbacks.removeBookNotificationCallback(
        id,
        sub as L2BookSubscription
      ),
    candlestick: (id: string, sub: WebSocketSubscription) =>
      this.databaseCallbacks.removeCandlestickNotificationCallback(
        id,
        sub as CandlestickSubscription
      ),
    trades: (id: string, sub: WebSocketSubscription) =>
      this.databaseCallbacks.removeTradeNotificationCallback(
        id,
        sub as TradesSubscription
      ),
    orders: (id: string, sub: WebSocketSubscription) =>
      this.databaseCallbacks.removeOrderNotificationCallback(
        id,
        sub as OrdersSubscription
      ),
  };

  constructor(bookService: BookService) {
    this.bookService = bookService;
    this.channelManager = {
      clients: new Map(),
      intervals: new Map(),
    };
    this.databaseCallbacks = DatabaseCallbacks.getInstance();

    // Initialize subscription configurations
    this.subscriptionConfigs = {
      l2Book: {
        subscribe: (clientId, subscription, callback) => {
          this.databaseCallbacks.addBookNotificationCallback(
            clientId,
            subscription as L2BookSubscription,
            callback
          );
        },
        getInitialData: async (subscription) => {
          const { instrument, groupTicks = 20 } =
            subscription as L2BookSubscription;
          const [baseAsset, quoteAsset] = instrument.split("/");
          if (!this.bookService.hasPair(baseAsset, quoteAsset)) {
            throw new CustomError(`Instrument not found: ${instrument}`, 404);
          }
          return await this.bookService.getOrderBook(
            baseAsset,
            quoteAsset,
            20,
            groupTicks
          );
        },
      },
      trades: {
        subscribe: (clientId, subscription, callback) => {
          this.databaseCallbacks.addTradeNotificationCallback(
            clientId,
            subscription as TradesSubscription,
            callback
          );
        },
      },
      orders: {
        subscribe: (clientId, subscription, callback) => {
          this.databaseCallbacks.addOrderNotificationCallback(
            clientId,
            subscription as OrdersSubscription,
            callback
          );
        },
      },
      candlestick: {
        subscribe: (clientId, subscription, callback) => {
          this.databaseCallbacks.addCandlestickNotificationCallback(
            clientId,
            subscription as CandlestickSubscription,
            callback
          );
        },
        getInitialData: async (subscription) => {
          const { instrument, stepSec } =
            subscription as CandlestickSubscription;
          return await this.databaseCallbacks.candlestickHandler.fetchData(
            subscription as CandlestickSubscription
          );
        },
      },
    };
  }

  /**
   * Create WebSocket route using Elysia's native WebSocket support
   */
  createWebSocketRoute() {
    const { Elysia, t } = require("elysia");

    return (
      new Elysia({ name: "websocket" })
        .ws("/ws", {
          // Validate incoming messages
          body: t.Object({
            method: t.Union([t.Literal("subscribe"), t.Literal("unsubscribe")]),
            subscription: t.Object({
              type: t.String(),
              instrument: t.String(),
              groupTicks: t.Optional(t.Number()),
              stepSec: t.Optional(t.Number()),
            }),
          }),

          open: (ws: any) => {
            const clientId = this.generateClientId();
            const client: ClientConnection = {
              id: clientId,
              ws,
              subscriptions: new Map(),
              messageQueue: [],
              isProcessing: false,
            };

            this.channelManager.clients.set(clientId, client);
            console.log(`WebSocket client connected: ${clientId}`);

            // Store client ID in WebSocket data for later use
            ws.data.clientId = clientId;
          },

          message: (ws: any, message: any) => {
            try {
              this.enqueueMessage(ws.data.clientId, message);
            } catch (error) {
              console.error("Invalid WebSocket message:", error);
              this.sendError(ws.data.clientId, "Invalid message format");
            }
          },

          close: (ws: any) => {
            this.handleDisconnect(ws.data.clientId);
          },
        })
        // WebSocket status endpoints
        .get("/api/websocket/stats", async () => {
          try {
            const stats = this.getStats();
            return {
              success: true,
              data: stats,
              timestamp: Date.now(),
            };
          } catch (error) {
            throw new CustomError(
              `Failed to get WebSocket stats: ${
                error instanceof Error ? error.message : "Unknown error"
              }`,
              500
            );
          }
        })
        .get("/api/websocket/health", async () => {
          try {
            const stats = this.getStats();
            return {
              success: true,
              status: "healthy",
              connectedClients: stats.connectedClients,
              totalSubscriptions: stats.subscriptions,
              timestamp: Date.now(),
            };
          } catch (error) {
            return {
              success: false,
              status: "unhealthy",
              error: error instanceof Error ? error.message : "Unknown error",
              timestamp: Date.now(),
            };
          }
        })
    );
  }

  /**
   * Enqueue a message for processing
   */
  private enqueueMessage(clientId: string, message: any) {
    const client = this.getClient(clientId);
    if (!client) {
      console.error(`Client not found: ${clientId}`);
      return;
    }

    // Add message to queue
    client.messageQueue.push(message);

    // Start processing if not already processing
    this.processQueue(clientId);
  }

  /**
   * Process the message queue for a client
   */
  private async processQueue(clientId: string) {
    const client = this.getClient(clientId);
    if (!client) {
      return;
    }

    // Acquire lock - if already processing, return immediately
    if (client.isProcessing) {
      return;
    }

    // Set processing flag atomically
    client.isProcessing = true;

    try {
      // Process all messages in the queue
      while (client.messageQueue.length > 0) {
        const message = client.messageQueue.shift();
        await this.handleMessage(clientId, message);
      }
    } catch (error) {
      console.error(`Error processing queue for client ${clientId}:`, error);
    } finally {
      // Always release the lock
      client.isProcessing = false;
    }
  }

  /**
   * Handle incoming WebSocket messages
   */
  private async handleMessage(clientId: string, message: any) {
    const client = this.getClient(clientId);
    if (!client) {
      console.error(`Client not found: ${clientId}`);
      return;
    }

    try {
      switch (message.method) {
        case "subscribe":
          await this.handleSubscribe(clientId, message.subscription);
          break;
        case "unsubscribe":
          this.handleUnsubscribe(clientId, message.subscription);
          break;
        default:
          this.sendError(clientId, `Unknown method: ${message.method}`);
      }
    } catch (error) {
      console.error(`Error handling message for client ${clientId}:`, error);
      this.sendError(clientId, "Internal server error");
    }
  }

  /**
   * Handle subscription requests
   */
  private async handleSubscribe(clientId: string, subscription: any) {
    const client = this.getClient(clientId);
    if (!client) return;

    if (subscription.instrument) {
      subscription.instrument = subscription.instrument.toUpperCase();
    }

    const subscriptionKey = getSubscriptionKey(subscription);
    const config =
      this.subscriptionConfigs[subscription.type as keyof SubscriptionHandlers];

    if (!config) {
      this.sendError(
        clientId,
        `Unknown subscription type: ${subscription.type}`
      );
      return;
    }

    try {
      await this.subscribeToChannel(clientId, subscription, config);
      client.subscriptions.set(subscriptionKey, subscription);
      console.log(
        `Client ${clientId} subscribed to ${subscription.type}: ${subscription.instrument}. Key: ${subscriptionKey}`
      );
    } catch (error) {
      console.error(`Subscription error for client ${clientId}:`, error);
      this.sendError(
        clientId,
        `Failed to subscribe: ${
          error instanceof Error ? error.message : "Unknown error"
        }`
      );
    }
  }

  /**
   * Handle unsubscription requests
   */
  private handleUnsubscribe(clientId: string, subscription: any) {
    const client = this.getClient(clientId);
    if (!client) return;

    const subscriptionKey = getSubscriptionKey(subscription);
    const existingSubscription = client.subscriptions.get(subscriptionKey);

    if (existingSubscription) {
      client.subscriptions.delete(subscriptionKey);

      // Use the unsubscribe handlers map
      const handler =
        this.unsubscribeHandlers[
          existingSubscription.type as keyof typeof this.unsubscribeHandlers
        ];
      if (handler) {
        handler(clientId, existingSubscription);
      }

      console.log(
        `Client ${clientId} unsubscribed from ${subscription.type}: ${subscription.instrument}. Key: ${subscriptionKey}`
      );
    } else {
      console.log(
        `Client ${clientId} did not have subscription ${subscriptionKey}`
      );
    }
  }

  /**
   * Generic subscription handler
   */
  private async subscribeToChannel<T>(
    clientId: string,
    subscription: WebSocketSubscription,
    config: SubscriptionConfig<T>
  ) {
    const client = this.getClient(clientId);
    if (!client) return;

    // Create callback that sends data to client
    const callback = (data: T) => {
      this.sendUpdate(
        clientId,
        subscription.type,
        subscription.instrument,
        data
      );
    };

    // Register the subscription
    config.subscribe(clientId, subscription, callback);

    // Send initial data if available
    if (config.getInitialData) {
      try {
        const initialData = await config.getInitialData(subscription);
        this.sendUpdate(
          clientId,
          subscription.type,
          subscription.instrument,
          initialData
        );
      } catch (error) {
        throw new CustomError(
          `Failed to get initial data: ${
            error instanceof Error ? error.message : "Unknown error"
          }`,
          500
        );
      }
    }
  }

  /**
   * Generic method to send updates to clients
   */
  private sendUpdate<T>(
    clientId: string,
    type: string,
    instrument: string,
    data: T
  ) {
    const client = this.getClient(clientId);
    if (!client || !this.isWebSocketOpen(client)) return;

    const response: WebSocketResponse = {
      type,
      instrument,
      data: this.formatDataForType(type, data),
      timestamp: Date.now(),
    };

    this.safeJsonSend(client, response);
  }

  /**
   * Format data based on subscription type
   */
  private formatDataForType<T>(type: string, data: T): any {
    switch (type) {
      case "orders":
        return { orders: data };
      case "trades":
        return { trades: data };
      case "candlestick":
        return { candlesticks: data };
      case "l2Book":
      default:
        return data;
    }
  }

  /**
   * Handle client disconnect
   */
  private handleDisconnect(clientId: string) {
    const client = this.getClient(clientId);
    if (!client) return;

    console.log(`WebSocket client disconnected: ${clientId}`);

    for (const subscription of client.subscriptions.values()) {
      this.handleUnsubscribe(clientId, subscription);
    }

    // Clean up subscriptions and message queue
    client.subscriptions.clear();
    client.messageQueue = [];
    client.isProcessing = false;

    this.channelManager.clients.delete(clientId);
  }

  /**
   * Send error message to client
   */
  private sendError(clientId: string, message: string) {
    const client = this.getClient(clientId);
    if (!client || !this.isWebSocketOpen(client)) return;

    const errorResponse = {
      type: "error",
      message,
      timestamp: Date.now(),
    };

    this.safeJsonSend(client, errorResponse);
  }

  /**
   * Helper method to get client by ID
   */
  private getClient(clientId: string): ClientConnection | undefined {
    return this.channelManager.clients.get(clientId);
  }

  /**
   * Helper method to check if WebSocket is open
   */
  private isWebSocketOpen(client: ClientConnection): boolean {
    return client.ws.raw.readyState === 1; // 1 = OPEN
  }

  /**
   * Helper method to safely send JSON data
   */
  private safeJsonSend(client: ClientConnection, data: any) {
    try {
      client.ws.send(JSON.stringify(data));
    } catch (error) {
      console.error(`Error sending data to client ${client.id}:`, error);
    }
  }

  /**
   * Generate unique client ID
   */
  private generateClientId(): string {
    return `client_${Date.now()}_${Math.random().toString(36).substr(2, 9)}`;
  }

  /**
   * Get connection statistics
   */
  getStats() {
    return {
      connectedClients: this.channelManager.clients.size,
      subscriptions: Array.from(this.channelManager.clients.values()).reduce(
        (total, client) => total + client.subscriptions.size,
        0
      ),
    };
  }

  /**
   * Close all connections and clean up
   */
  close() {
    // Close all client connections
    for (const client of this.channelManager.clients.values()) {
      if (client.ws.raw.readyState === 1) {
        client.ws.close();
      }
    }
    this.channelManager.clients.clear();

    console.log("WebSocket service closed");
  }
}
