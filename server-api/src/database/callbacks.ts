import { Pool } from "pg";
import {
  CandlestickSubscription,
  L2BookData,
  L2BookSubscription,
  Order,
  Trade,
} from "@/types";
import { DatabaseConfig } from "@/config/database";
import { UserService } from "@/services/user-service";
import { DatabaseQueries } from "./queries";
import { getAppConfig } from "@/config/app";
import { CandlestickData } from "@/types/api";

export class DatabaseCallbacks {
  private static instance: DatabaseCallbacks;
  private pool: Pool;
  private notificationClient: any = null; // Dedicated connection for notifications
  private tradeNotifCallbacks: Map<string, (payload: Trade[]) => void> =
    new Map();
  private orderNotifCallbacks: Map<string, (payload: Order[]) => void> =
    new Map();
  private bookNotifCallbacks: Map<string, (data: L2BookData) => void> =
    new Map();
  private instrumentNotifCallbacks: Map<string, () => void> = new Map();
  private candlestickNotifCallbacks: Map<
    string,
    (data: CandlestickData[]) => void
  > = new Map();
  private notificationChannels = ["orders", "trades", "instruments"];
  // TODO: store this in db to be retrieved when restarting the server
  private last_seen_trade_id: number = 0;
  private last_seen_order_id: number = 0;
  private last_seen_instrument_id: number = 0;
  private userService: UserService;
  private queries: DatabaseQueries;
  private isShuttingDown: boolean = false;
  private pollingInterval: NodeJS.Timeout | null = null;
  private pollingIntervalMs: number;
  private activeBookSubscriptions: Set<L2BookSubscription> = new Set();
  private activeCandlestickSubscriptions: Set<CandlestickSubscription> =
    new Set();

  private constructor(pool: Pool) {
    this.pool = pool;
    this.userService = new UserService(new DatabaseQueries(this.pool));
    this.queries = new DatabaseQueries(this.pool);
    this.pollingIntervalMs = getAppConfig().wsPollingIntervalMs;
    this.initializeNotificationConnection();
    this.initializeLastSeenIds();
    this.startPolling();
  }

  private getBookCallbackKey(
    client_id: string,
    subscription: L2BookSubscription
  ): string {
    return `${client_id}:${subscription.type}:${subscription.instrument}:${subscription.groupTicks}`;
  }

  private getCandlestickCallbackKey(
    client_id: string,
    subscription: CandlestickSubscription
  ): string {
    return `${client_id}:${subscription.type}:${subscription.instrument}:${subscription.stepSec}`;
  }

  public static getInstance(): DatabaseCallbacks {
    if (!DatabaseCallbacks.instance) {
      DatabaseCallbacks.instance = new DatabaseCallbacks(
        DatabaseConfig.getInstance().getPool()
      );
    }
    return DatabaseCallbacks.instance;
  }

  private async initializeNotificationConnection() {
    // Don't attempt to reconnect if we're shutting down
    if (this.isShuttingDown) {
      console.log(
        "Skipping notification connection initialization - shutting down"
      );
      return;
    }

    try {
      // Create a dedicated connection for notifications
      this.notificationClient = await this.pool.connect();
      console.log("Dedicated notification connection established");

      // Set up notification listener on the dedicated connection
      this.notificationClient.on("notification", (message: any) => {
        console.log(
          "Database notification received",
          message.channel,
          message.payload
        );
        if (message.channel === "trades") {
          this.handleNewTrades();
        }
        if (message.channel === "orders") {
          this.handleNewOrders();
        }
        if (message.channel === "instruments") {
          this.handleInstrumentsUpdate();
        }
      });

      // Start listening on all channels with the dedicated connection
      for (const channel of this.notificationChannels) {
        console.log(
          `LISTENING on channel ${channel} with dedicated connection`
        );
        await this.notificationClient.query(`LISTEN ${channel}`);
      }

      // Handle connection errors
      this.notificationClient.on("error", (err: Error) => {
        console.error("Notification connection error:", err);
        // Only attempt to reconnect if not shutting down
        if (!this.isShuttingDown) {
          setTimeout(() => this.initializeNotificationConnection(), 1000);
        }
      });

      this.notificationClient.on("end", () => {
        console.log(
          "Notification connection ended, attempting to reconnect..."
        );
        // Only attempt to reconnect if not shutting down
        if (!this.isShuttingDown) {
          setTimeout(() => this.initializeNotificationConnection(), 1000);
        }
      });
    } catch (error) {
      console.error("Failed to initialize notification connection:", error);

      // Check if the error is due to pool being ended
      if (
        error instanceof Error &&
        error.message.includes("Cannot use a pool after calling end")
      ) {
        console.log("Pool has been ended, stopping reconnection attempts");
        return;
      }

      // Only retry if not shutting down and it's not a pool end error
      if (!this.isShuttingDown) {
        setTimeout(() => this.initializeNotificationConnection(), 1000);
      }
    }
  }

  private initializeLastSeenIds() {
    this.pool.query("SELECT MAX(trade_id) FROM trade_events").then((result) => {
      this.last_seen_trade_id = result.rows[0].max || 0;
      console.log("Last seen trade id", this.last_seen_trade_id);
    });
    this.pool.query("SELECT MAX(event_id) FROM order_events").then((result) => {
      this.last_seen_order_id = result.rows[0].max || 0;
      console.log("Last seen order id", this.last_seen_order_id);
    });
  }

  private handleNewTrades() {
    // Get new events since last_seen_trade_id
    this.pool
      .query(
        "SELECT trade_id, instrument_id, price, qty, trade_time, side, maker_user_id, taker_user_id FROM trade_events WHERE trade_id > $1",
        [this.last_seen_trade_id]
      )
      .then(async (result) => {
        if (result.rows.length === 0) {
          console.log("No new trades");
          return;
        }
        this.last_seen_trade_id = result.rows[result.rows.length - 1].trade_id;

        // trades sorted by user_id
        const payloads: Map<string, Trade[]> = new Map();
        for (const row of result.rows) {
          const maker_user = await this.userService.getUserById(
            row.maker_user_id
          );
          const taker_user = await this.userService.getUserById(
            row.taker_user_id
          );

          if (maker_user && !payloads.has(maker_user.identity)) {
            payloads.set(maker_user.identity, []);
          }
          if (taker_user && !payloads.has(taker_user.identity)) {
            payloads.set(row.taker_user_id, []);
          }
          const payload = {
            trade_id: row.trade_id,
            instrument_id: parseInt(row.instrument_id, 10),
            price: parseInt(row.price, 10),
            qty: parseInt(row.qty, 10),
            trade_time: row.trade_time,
            side: row.side,
          };
          if (maker_user) {
            payloads.get(maker_user?.identity)?.push(payload);
          }
          if (taker_user && taker_user.user_id !== maker_user?.user_id) {
            payloads.get(taker_user.identity)?.push(payload);
          }
        }

        for (const [user_id, payload] of payloads) {
          // console.log("Notifying trade callback for user", user_id, payload);
          this.tradeNotifCallbacks.get(user_id)?.(payload);
        }
      })
      .catch((error: Error) => {
        console.error(`Failed to get new trades`, error);
      });
  }

  private handleNewOrders() {
    this.pool
      .query(
        "SELECT event_id, order_id, instrument_id, user_id, side, type, price, qty, qty_filled, status, event_time FROM order_events WHERE event_id > $1",
        [this.last_seen_order_id]
      )
      .then(async (result) => {
        // console.log(`New orders after ${this.last_seen_order_id}`, result.rows);
        if (result.rows.length === 0) {
          console.log("No new orders");
          return;
        }
        this.last_seen_order_id = result.rows[result.rows.length - 1].event_id;
        const payloads: Map<string, Order[]> = new Map();
        for (const row of result.rows) {
          const user = await this.userService.getUserById(row.user_id);
          if (user) {
            if (!payloads.has(user.identity)) {
              payloads.set(user.identity, []);
            }
            const payload: Order = {
              order_id: row.order_id,
              instrument_id: parseInt(row.instrument_id, 10),
              user_id: parseInt(row.user_id, 10),
              side: row.side,
              type: row.type,
              price: parseInt(row.price, 10),
              qty: parseInt(row.qty, 10),
              qty_filled: parseInt(row.qty_filled, 10),
              qty_remaining:
                parseInt(row.qty, 10) - parseInt(row.qty_filled, 10),
              status: row.status,
              created_at: new Date(row.event_time), // TODO: fix this
              updated_at: new Date(row.event_time),
            };
            payloads.get(user.identity)?.push(payload);
          }
        }
        for (const [user_id, payload] of payloads) {
          // console.log("Notifying order callback for user", user_id, payload);
          this.orderNotifCallbacks.get(user_id)?.(payload);
        }
      });
  }

  private startPolling() {
    this.pollingInterval = setInterval(() => {
      this.pollUpdates();
    }, this.pollingIntervalMs);
    console.log(`Started polling with interval ${this.pollingIntervalMs}ms`);
  }

  private async pollUpdates() {
    // Poll L2 book updates for active instruments
    for (const subscription of this.activeBookSubscriptions) {
      const { instrument, groupTicks } = subscription;
      try {
        console.log(
          `Polling L2 book for ${instrument} with groupTicks ${groupTicks}`
        );
        const rawBookData = await this.queries.getOrderbook(
          instrument,
          20,
          groupTicks
        );

        // Transform raw data to L2BookData format
        const l2BookData: L2BookData = {
          bids: rawBookData
            .filter((entry) => entry.side === "bid")
            .map((entry) => ({
              price: entry.price,
              quantity: entry.qty,
            })),
          asks: rawBookData
            .filter((entry) => entry.side === "ask")
            .map((entry) => ({
              price: entry.price,
              quantity: entry.qty,
            })),
        };

        console.log("book notif subscriptions", this.bookNotifCallbacks.keys());
        const callbacks = Array.from(this.bookNotifCallbacks.entries())
          .filter(([key, _]) => {
            // Parse the key to check if it matches the subscription
            const parts = key.split(":");
            return (
              parts[1] === subscription.type &&
              parts[2] === subscription.instrument &&
              parseInt(parts[3]) === subscription.groupTicks
            );
          })
          .map(([_, callback]) => callback);

        for (const callback of callbacks) {
          callback(l2BookData);
        }
      } catch (error) {
        console.error(`Error polling L2Book for ${instrument}:`, error);
      }
    }

    // Poll candlestick updates for active subscriptions
    for (const subscription of this.activeCandlestickSubscriptions) {
      const { instrument, stepSec } = subscription;
      const candlestickData = await this.getCandlestickData(
        instrument,
        stepSec
      );
      const callbacks = Array.from(this.candlestickNotifCallbacks.entries())
        .filter(([key, _]) => {
          // Parse the key to check if it matches the subscription
          const parts = key.split(":");
          return (
            parts[1] === subscription.type &&
            parts[2] === subscription.instrument &&
            parseInt(parts[3]) === subscription.stepSec
          );
        })
        .map(([_, callback]) => callback);
      for (const callback of callbacks) {
        callback(candlestickData);
      }
    }
  }

  private handleInstrumentsUpdate() {
    for (const callback of this.instrumentNotifCallbacks.values()) {
      if (callback) {
        callback();
      }
    }
  }

  addTradeNotificationCallback(
    user: string,
    callback: (payload: Trade[]) => void
  ) {
    this.tradeNotifCallbacks.set(user, callback);
  }
  addOrderNotificationCallback(
    user: string,
    callback: (payload: Order[]) => void
  ) {
    this.orderNotifCallbacks.set(user, callback);
  }
  addBookNotificationCallback(
    client_id: string,
    subscription: L2BookSubscription,
    callback: (data: L2BookData) => void
  ) {
    const key = this.getBookCallbackKey(client_id, subscription);
    this.bookNotifCallbacks.set(key, callback);
    this.activeBookSubscriptions.add(subscription);
  }
  addInstrumentsCallback(client_id: string, callback: () => void) {
    this.instrumentNotifCallbacks.set(client_id, callback);
  }

  // Candlestick callback methods
  addCandlestickNotificationCallback(
    client_id: string,
    subscription: CandlestickSubscription,
    callback: (data: CandlestickData[]) => void
  ) {
    const key = this.getCandlestickCallbackKey(client_id, subscription);
    this.candlestickNotifCallbacks.set(key, callback);
    this.activeCandlestickSubscriptions.add(subscription);
  }

  removeCandlestickNotificationCallback(
    client_id: string,
    subscription: CandlestickSubscription
  ) {
    const key = this.getCandlestickCallbackKey(client_id, subscription);
    this.candlestickNotifCallbacks.delete(key);

    // Check if any other client is still subscribed to this subscription
    const hasOtherSubscriptions = Array.from(
      this.candlestickNotifCallbacks.keys()
    ).some((key) => {
      const parts = key.split(":");
      return (
        parts[1] === subscription.type &&
        parts[2] === subscription.instrument &&
        parseInt(parts[3]) === subscription.stepSec
      );
    });
    if (!hasOtherSubscriptions) {
      this.activeCandlestickSubscriptions.delete(subscription);
      console.log(
        `Stopped polling candlestick: ${subscription.instrument}_${subscription.stepSec}`
      );
    }
  }

  // Candlestick data methods
  async getInitialCandlestickData(
    instrument: string,
    stepSec: number
  ): Promise<CandlestickData[]> {
    const now = new Date();
    const fromDate = new Date(now.getTime() - stepSec * 1000 * 1000); // 1000 candles back
    const toDate = now;

    const rawData = await this.queries.getCandlestickData(
      parseInt(instrument.split("/")[0]), // Assuming instrument_id is first part
      fromDate.toISOString(),
      toDate.toISOString(),
      stepSec
    );

    return rawData.map((row) => ({
      time: Math.floor(new Date(row.bucket).getTime() / 1000),
      open: row.open,
      high: row.high,
      low: row.low,
      close: row.close,
      volume_trades: row.volume_trades,
      trade_count: row.trade_count,
    }));
  }

  async getCandlestickData(
    instrument: string,
    stepSec: number
  ): Promise<CandlestickData[]> {
    const now = new Date();
    const fromDate = new Date(now.getTime() - stepSec * 1000 * 10); // Last 10 candles
    const toDate = now;

    const rawData = await this.queries.getCandlestickData(
      parseInt(instrument.split("/")[0]), // Assuming instrument_id is first part
      fromDate.toISOString(),
      toDate.toISOString(),
      stepSec
    );

    return rawData.map((row) => ({
      time: Math.floor(new Date(row.bucket).getTime() / 1000),
      open: row.open,
      high: row.high,
      low: row.low,
      close: row.close,
      volume_trades: row.volume_trades,
      trade_count: row.trade_count,
    }));
  }

  removeBookNotificationCallback(
    client_id: string,
    subscription: L2BookSubscription
  ) {
    console.log("RemoveBookNotificationCallback", client_id, subscription);
    const key = this.getBookCallbackKey(client_id, subscription);
    const callback = this.bookNotifCallbacks.get(key);

    console.log("callback", callback);
    console.log("key", key);
    console.log("book notif callbacks", this.bookNotifCallbacks.keys());
    if (!this.bookNotifCallbacks.delete(key)) {
      console.error("Failed to remove book notification callback");
      console.log("book notif callbacks", this.bookNotifCallbacks.keys());
      return;
    }
    console.log("book notif callbacks", this.bookNotifCallbacks.keys());

    // Check if any other client is still subscribed to this instrument
    const hasOtherSubscriptions = Array.from(
      this.bookNotifCallbacks.keys()
    ).some((key) => {
      const parts = key.split(":");
      return (
        parts[1] === subscription.type &&
        parts[2] === subscription.instrument &&
        parseInt(parts[3]) === subscription.groupTicks
      );
    });

    if (!hasOtherSubscriptions) {
      this.activeBookSubscriptions.delete(subscription);
      console.log(
        `Stopped polling L2 book: ${subscription.instrument}_${subscription.groupTicks}`
      );
    }
  }

  removeTradeNotificationCallback(user: string) {
    this.tradeNotifCallbacks.delete(user);
  }
  removeOrderNotificationCallback(user: string) {
    this.orderNotifCallbacks.delete(user);
  }

  async close() {
    console.log("Closing database callbacks...");
    this.isShuttingDown = true;

    // Clear polling interval
    if (this.pollingInterval) {
      clearInterval(this.pollingInterval);
      this.pollingInterval = null;
      console.log("Polling interval cleared");
    }

    if (this.notificationClient) {
      try {
        // Remove all event listeners to prevent reconnection attempts
        this.notificationClient.removeAllListeners();
        await this.notificationClient.release();
        console.log("Notification connection released");
      } catch (error) {
        console.error("Error releasing notification connection:", error);
      }
    }
  }
}
