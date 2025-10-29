import { Pool } from "pg";
import {
  CandlestickSubscription,
  InstrumentsSubscription,
  L2BookData,
  L2BookSubscription,
  Order,
  OrdersSubscription,
  Trade,
  TradesSubscription,
} from "@/types";
import { DatabaseConfig } from "@/config/database";
import { UserService } from "@/services/user-service";
import { DatabaseQueries } from "./queries";
import { getAppConfig } from "@/config/app";
import { CandlestickData } from "@/types/api";
import {
  SubscriptionManager,
  PolledSubscriptionHandler,
  BookSubscriptionHandler,
  CandlestickSubscriptionHandler,
} from "./subscription-manager";

export class DatabaseCallbacks {
  private static instance: DatabaseCallbacks;
  private pool: Pool;
  private notificationClient: any = null;
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

  // Subscription managers
  private tradeManager: SubscriptionManager<Trade[], TradesSubscription>;
  private orderManager: SubscriptionManager<Order[], OrdersSubscription>;
  private instrumentManager: SubscriptionManager<void, InstrumentsSubscription>;
  private bookHandler: PolledSubscriptionHandler<
    L2BookData,
    L2BookSubscription
  >;
  public candlestickHandler: PolledSubscriptionHandler<
    CandlestickData[],
    CandlestickSubscription
  >;

  private constructor(pool: Pool) {
    this.pool = pool;
    this.userService = new UserService(new DatabaseQueries(this.pool));
    this.queries = new DatabaseQueries(this.pool);
    this.pollingIntervalMs = getAppConfig().wsPollingIntervalMs;

    // Initialize subscription managers
    this.tradeManager = new SubscriptionManager<Trade[], TradesSubscription>();
    this.orderManager = new SubscriptionManager<Order[], OrdersSubscription>();
    this.instrumentManager = new SubscriptionManager<
      void,
      InstrumentsSubscription
    >();

    // Initialize polled subscription handlers
    this.bookHandler = new BookSubscriptionHandler(this.queries);
    this.candlestickHandler = new CandlestickSubscriptionHandler(this.queries);

    this.initializeNotificationConnection();
    this.initializeLastSeenIds();
    this.startPolling();
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
    });
    this.pool.query("SELECT MAX(event_id) FROM order_events").then((result) => {
      this.last_seen_order_id = result.rows[0].max || 0;
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
            payloads.set(taker_user.identity, []);
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
            payloads.get(maker_user.identity)!.push(payload);
          }
          if (taker_user && taker_user.user_id !== maker_user?.user_id) {
            payloads.get(taker_user.identity)!.push(payload);
          }
        }

        for (const [user_id, payload] of payloads) {
          const callbacks = Array.from(
            this.tradeManager.getAllCallbacks().entries()
          )
            .filter(([key, _]) => {
              return key.split(":")[1] === user_id;
            })
            .map(([_, callback]) => callback);
          for (const callback of callbacks) {
            callback?.(payload);
          }
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
            payloads.get(user.identity)!.push(payload);
          }
        }
        for (const [user_id, payload] of payloads) {
          const callbacks = Array.from(
            this.orderManager.getAllCallbacks().entries()
          )
            .filter(([key, _]) => {
              return key.split(":")[1] === user_id;
            })
            .map(([_, callback]) => callback);
          for (const callback of callbacks) {
            callback?.(payload);
          }
        }
      });
  }

  private startPolling() {
    this.pollingInterval = setInterval(() => {
      this.pollUpdates();
    }, this.pollingIntervalMs);
  }

  private async pollUpdates() {
    await Promise.all([
      this.bookHandler.pollUpdates(),
      this.candlestickHandler.pollUpdates(),
    ]);
  }

  private handleInstrumentsUpdate() {
    for (const callback of this.instrumentManager.getAllCallbacks().values()) {
      callback();
    }
  }

  addTradeNotificationCallback(
    client_id: string,
    subscription: TradesSubscription,
    callback: (payload: Trade[]) => void
  ) {
    this.tradeManager.addCallback(client_id, subscription, callback);
  }

  addOrderNotificationCallback(
    client_id: string,
    subscription: OrdersSubscription,
    callback: (payload: Order[]) => void
  ) {
    this.orderManager.addCallback(client_id, subscription, callback);
  }

  addBookNotificationCallback(
    client_id: string,
    subscription: L2BookSubscription,
    callback: (data: L2BookData) => void
  ) {
    this.bookHandler.addCallback(client_id, subscription, callback);
  }

  addInstrumentsCallback(client_id: string, callback: () => void) {
    this.instrumentManager.addCallback(
      client_id,
      { type: "instruments", instrument: "ALL" },
      callback
    );
  }

  addCandlestickNotificationCallback(
    client_id: string,
    subscription: CandlestickSubscription,
    callback: (data: CandlestickData[]) => void
  ) {
    this.candlestickHandler.addCallback(client_id, subscription, callback);
  }

  removeCandlestickNotificationCallback(
    client_id: string,
    subscription: CandlestickSubscription
  ) {
    this.candlestickHandler.removeCallback(client_id, subscription);
  }

  removeBookNotificationCallback(
    client_id: string,
    subscription: L2BookSubscription
  ) {
    this.bookHandler.removeCallback(client_id, subscription);
  }

  removeTradeNotificationCallback(
    client_id: string,
    subscription: TradesSubscription
  ) {
    this.tradeManager.removeCallback(client_id, subscription);
  }

  removeOrderNotificationCallback(
    client_id: string,
    subscription: OrdersSubscription
  ) {
    this.orderManager.removeCallback(client_id, subscription);
  }

  async close() {
    this.isShuttingDown = true;

    // Clear polling interval
    if (this.pollingInterval) {
      clearInterval(this.pollingInterval);
      this.pollingInterval = null;
    }

    if (this.notificationClient) {
      try {
        this.notificationClient.removeAllListeners();
        await this.notificationClient.release();
      } catch (error) {
        console.error("Error releasing notification connection:", error);
      }
    }
  }
}
