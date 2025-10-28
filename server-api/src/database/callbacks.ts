import { Pool } from "pg";
import { L2BookData, L2BookSubscription, Order, Trade } from "@/types";
import { DatabaseConfig } from "@/config/database";
import { UserService } from "@/services/user-service";
import { DatabaseQueries } from "./queries";

export class DatabaseCallbacks {
  private static instance: DatabaseCallbacks;
  private pool: Pool;
  private notificationClient: any = null; // Dedicated connection for notifications
  private tradeNotifCallbacks: Map<string, (payload: Trade[]) => void> =
    new Map();
  private orderNotifCallbacks: Map<string, (payload: Order[]) => void> =
    new Map();
  private bookNotifCallbacks: Map<string, (instrument: string) => void> =
    new Map();
  private instrumentNotifCallbacks: Map<string, () => void> = new Map();
  private notificationChannels = ["book", "orders", "trades", "instruments"];
  // TODO: store this in db to be retrieved when restarting the server
  private last_seen_trade_id: number = 0;
  private last_seen_order_id: number = 0;
  private last_seen_instrument_id: number = 0;
  private userService: UserService;
  private queries: DatabaseQueries;
  private isShuttingDown: boolean = false;

  private constructor(pool: Pool) {
    this.pool = pool;
    this.userService = new UserService(new DatabaseQueries(this.pool));
    this.queries = new DatabaseQueries(this.pool);
    this.initializeNotificationConnection();
    this.initializeLastSeenIds();
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
        if (message.channel === "book") {
          this.handleL2BookUpdate(message.payload);
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

  private handleL2BookUpdate(instrument: string) {
    console.log(
      "Handling L2 book update for instrument",
      instrument,
      this.bookNotifCallbacks.size
    );
    for (const callback of this.bookNotifCallbacks.values()) {
      if (callback) {
        callback(instrument);
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
    callback: (instrument: string) => void
  ) {
    this.bookNotifCallbacks.set(client_id, callback);
  }
  addInstrumentsCallback(client_id: string, callback: () => void) {
    this.instrumentNotifCallbacks.set(client_id, callback);
  }
  removeBookNotificationCallback(client_id: string) {
    this.bookNotifCallbacks.delete(client_id);
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
