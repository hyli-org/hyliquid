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
  L2BookData,
} from "../types/websocket";
import { BookService } from "./book-service";
import { Order, Trade } from "@/types";
import { DatabaseCallbacks } from "@/database/callbacks";
import { CustomError } from "@/middleware";

export class WebSocketService {
  private channelManager: ChannelManager;
  private bookService: BookService;
  private databaseCallbacks: DatabaseCallbacks;

  constructor(bookService: BookService) {
    this.bookService = bookService;
    this.channelManager = {
      clients: new Map(),
      intervals: new Map(),
    };
    this.databaseCallbacks = DatabaseCallbacks.getInstance();
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
    const client = this.channelManager.clients.get(clientId);
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
    const client = this.channelManager.clients.get(clientId);
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
    const client = this.channelManager.clients.get(clientId);
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
    const client = this.channelManager.clients.get(clientId);
    if (!client) return;

    if (subscription.instrument) {
      subscription.instrument = subscription.instrument.toUpperCase();
    }

    const subscriptionKey = this.getSubscriptionKey(subscription);

    try {
      switch (subscription.type) {
        case "l2Book":
          await this.subscribeToL2Book(
            clientId,
            subscription as L2BookSubscription
          );
          break;
        case "trades":
          await this.subscribeToTrades(
            clientId,
            subscription as TradesSubscription
          );
          break;
        case "orders":
          await this.subscribeToOrders(
            clientId,
            subscription as OrdersSubscription
          );
          break;
        default:
          this.sendError(
            clientId,
            `Unknown subscription type: ${subscription.type}`
          );
          return;
      }

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
    const client = this.channelManager.clients.get(clientId);
    if (!client) return;

    const subscriptionKey = this.getSubscriptionKey(subscription);

    if (client.subscriptions.has(subscriptionKey)) {
      client.subscriptions.delete(subscriptionKey);
      console.log(
        `Client ${clientId} unsubscribed from ${subscription.type}: ${subscription.instrument}. Key: ${subscriptionKey}`
      );
    } else {
      console.log(
        `Client ${clientId} did not have subscription ${subscriptionKey}`
      );
    }
  }

  private async subscribeToOrders(
    clientId: string,
    subscription: OrdersSubscription
  ) {
    const client = this.channelManager.clients.get(clientId);
    if (!client) return;

    this.databaseCallbacks.addOrderNotificationCallback(
      subscription.user,
      (payload: Order[]) => {
        this.sendOrdersUpdate(clientId, subscription.instrument, payload);
      }
    );
  }

  private sendOrdersUpdate(
    clientId: string,
    instrument: string,
    payload: Order[]
  ) {
    const client = this.channelManager.clients.get(clientId);
    if (!client || client.ws.raw.readyState !== 1) return;

    const response: WebSocketResponse = {
      type: "orders",
      instrument: instrument,
      data: { orders: payload },
      timestamp: Date.now(),
    };

    try {
      client.ws.send(JSON.stringify(response));
    } catch (error) {
      console.error(
        `Error sending orders update to client ${clientId}:`,
        error
      );
    }
  }

  private async subscribeToTrades(
    clientId: string,
    subscription: TradesSubscription
  ) {
    const client = this.channelManager.clients.get(clientId);
    if (!client) return;

    this.databaseCallbacks.addTradeNotificationCallback(
      subscription.user,
      (payload: Trade[]) => {
        this.sendTradesUpdate(clientId, subscription.instrument, payload);
      }
    );
  }

  private sendTradesUpdate(
    clientId: string,
    instrument: string,
    payload: Trade[]
  ) {
    const client = this.channelManager.clients.get(clientId);
    if (!client || client.ws.raw.readyState !== 1) return;

    const response: WebSocketResponse = {
      type: "trades",
      instrument: instrument,
      data: { trades: payload },
      timestamp: Date.now(),
    };

    try {
      client.ws.send(JSON.stringify(response));
    } catch (error) {
      console.error(
        `Error sending trades update to client ${clientId}:`,
        error
      );
    }
  }

  /**
   * Subscribe to L2 book updates
   */
  private async subscribeToL2Book(
    clientId: string,
    subscription: L2BookSubscription
  ) {
    const { instrument, groupTicks = 10 } = subscription;

    // Validate instrument exists
    const [baseAsset, quoteAsset] = instrument.split("/");
    if (!this.bookService.hasPair(baseAsset, quoteAsset)) {
      throw new CustomError(`Instrument not found: ${instrument}`, 404);
    }

    this.databaseCallbacks.addBookNotificationCallback(
      clientId,
      (instrument: string) => {
        this.triggerL2BookUpdate(instrument);
      }
    );

    // Send initial data
    try {
      const bookData = await this.bookService.getOrderBook(
        baseAsset,
        quoteAsset,
        10, // default levels
        groupTicks
      );

      this.sendL2BookUpdate(clientId, instrument, bookData);
    } catch (error) {
      throw new CustomError(
        `Failed to get initial book data: ${
          error instanceof Error ? error.message : "Unknown error"
        }`,
        500
      );
    }
  }

  /**
   * Send L2 book update to a specific client
   */
  private sendL2BookUpdate(
    clientId: string,
    instrument: string,
    bookData: any
  ) {
    const client = this.channelManager.clients.get(clientId);
    if (!client || client.ws.raw.readyState !== 1) return; // 1 = OPEN

    const response: WebSocketResponse = {
      type: "l2Book",
      instrument,
      data: bookData,
      timestamp: Date.now(),
    };

    try {
      client.ws.send(JSON.stringify(response));
    } catch (error) {
      console.error(
        `Error sending L2 book update to client ${clientId}:`,
        error
      );
    }
  }

  /**
   * Handle client disconnect
   */
  private handleDisconnect(clientId: string) {
    const client = this.channelManager.clients.get(clientId);
    if (!client) return;

    console.log(`WebSocket client disconnected: ${clientId}`);

    // Clean up subscriptions and message queue
    client.subscriptions.clear();
    client.messageQueue = [];
    client.isProcessing = false;

    this.channelManager.clients.delete(clientId);
  }

  /**
   * Trigger L2 book update for all subscribed clients
   */
  public async triggerL2BookUpdate(instrument: string) {
    console.log("Triggering L2 book update for", instrument);
    const [baseAsset, quoteAsset] = instrument.split("/");

    // Group clients by their groupTicks value to minimize SQL calls
    const clientsByGroupTicks = new Map<
      number,
      Array<{ clientId: string; subscription: L2BookSubscription }>
    >();

    for (const [clientId, client] of this.channelManager.clients) {
      for (const subscription of client.subscriptions.values()) {
        if (
          subscription.type === "l2Book" &&
          subscription.instrument === instrument
        ) {
          const l2Subscription = subscription as L2BookSubscription;
          const groupTicks = l2Subscription.groupTicks || 10; // default to 10

          if (!clientsByGroupTicks.has(groupTicks)) {
            clientsByGroupTicks.set(groupTicks, []);
          }
          clientsByGroupTicks.get(groupTicks)!.push({
            clientId,
            subscription: l2Subscription,
          });
        }
      }
    }

    if (clientsByGroupTicks.size === 0) {
      console.log(`No clients subscribed to ${instrument}`);
      return;
    }

    try {
      // Make one call to getOrderBook per unique groupTicks value and send updates immediately
      const bookDataPromises = Array.from(clientsByGroupTicks.keys()).map(
        async (groupTicks) => {
          try {
            const bookData = await this.bookService.getOrderBook(
              baseAsset,
              quoteAsset,
              20, // default levels
              groupTicks
            );

            // Send updates immediately when data is available
            const clients = clientsByGroupTicks.get(groupTicks)!;
            for (const { clientId } of clients) {
              this.sendL2BookUpdate(clientId, instrument, bookData);
            }

            return { groupTicks, clientCount: clients.length, success: true };
          } catch (error) {
            console.error(
              `Error getting book data for groupTicks ${groupTicks}:`,
              error
            );
            return { groupTicks, clientCount: 0, success: false, error };
          }
        }
      );

      // Wait for all promises to complete and log results
      const results = await Promise.allSettled(bookDataPromises);
      let totalClientsUpdated = 0;
      let successfulGroupTicks = 0;

      for (const result of results) {
        if (result.status === "fulfilled" && result.value.success) {
          totalClientsUpdated += result.value.clientCount;
          successfulGroupTicks++;
        }
      }

      console.log(
        `L2 book update sent to ${totalClientsUpdated} clients for ${instrument} (${successfulGroupTicks}/${clientsByGroupTicks.size} unique groupTicks successful)`
      );
    } catch (error) {
      console.error(
        `Error triggering L2 book update for ${instrument}:`,
        error
      );
      throw error;
    }
  }

  /**
   * Send error message to client
   */
  private sendError(clientId: string, message: string) {
    const client = this.channelManager.clients.get(clientId);
    if (!client || client.ws.raw.readyState !== 1) return;

    const errorResponse = {
      type: "error",
      message,
      timestamp: Date.now(),
    };

    try {
      client.ws.send(JSON.stringify(errorResponse));
    } catch (error) {
      console.error(
        `Error sending error message to client ${clientId}:`,
        error
      );
    }
  }

  /**
   * Generate unique client ID
   */
  private generateClientId(): string {
    return `client_${Date.now()}_${Math.random().toString(36).substr(2, 9)}`;
  }

  /**
   * Generate subscription key for tracking
   */
  private getSubscriptionKey(subscription: any): string {
    return `${subscription.type}_${subscription.instrument.toLowerCase()}_${
      subscription.groupTicks || "default"
    }`;
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
