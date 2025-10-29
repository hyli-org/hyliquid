import {
  CandlestickSubscription,
  L2BookData,
  L2BookSubscription,
  CandlestickData,
} from "@/types";
import { DatabaseQueries } from "./queries";

// Generic subscription manager for type-safe callback handling
export class SubscriptionManager<T, S> {
  private callbacks: Map<string, (payload: T) => void> = new Map();
  private activeSubscriptions: Set<S> = new Set();
  private keyGenerator: (clientId: string, subscription: S) => string;

  constructor(keyGenerator: (clientId: string, subscription: S) => string) {
    this.keyGenerator = keyGenerator;
  }

  addCallback(
    clientId: string,
    subscription: S,
    callback: (payload: T) => void
  ): void {
    const key = this.keyGenerator(clientId, subscription);
    this.callbacks.set(key, callback);
    this.activeSubscriptions.add(subscription);
  }

  removeCallback(clientId: string, subscription: S): boolean {
    const key = this.keyGenerator(clientId, subscription);
    const removed = this.callbacks.delete(key);

    // Check if any other client is still subscribed to this subscription
    const hasOtherSubscriptions = Array.from(this.callbacks.keys()).some(
      (key) => {
        const parts = key.split(":");
        return this.matchesSubscription(parts, subscription);
      }
    );

    if (!hasOtherSubscriptions) {
      this.activeSubscriptions.delete(subscription);
    }

    return removed;
  }

  getCallbacksForSubscription(subscription: S): Array<(payload: T) => void> {
    return Array.from(this.callbacks.entries())
      .filter(([key, _]) => {
        const parts = key.split(":");
        return this.matchesSubscription(parts, subscription);
      })
      .map(([_, callback]) => callback);
  }

  getActiveSubscriptions(): Set<S> {
    return this.activeSubscriptions;
  }

  getAllCallbacks(): Map<string, (payload: T) => void> {
    return this.callbacks;
  }

  protected matchesSubscription(parts: string[], subscription: S): boolean {
    // Generate the expected key for this subscription and compare with the parts
    // This is a generic implementation that works for most cases
    const expectedKey = this.keyGenerator(parts[0], subscription);
    const expectedParts = expectedKey.split(":");
    return (
      expectedParts.length === parts.length &&
      expectedParts.every((part, index) => part === parts[index])
    );
  }
}

// Base class for polled subscriptions
export abstract class PolledSubscriptionHandler<T, S> {
  protected subscriptionManager: SubscriptionManager<T, S>;
  protected queries: DatabaseQueries;

  constructor(
    keyGenerator: (clientId: string, subscription: S) => string,
    queries: DatabaseQueries
  ) {
    this.subscriptionManager = new SubscriptionManager(keyGenerator);
    this.queries = queries;
  }

  abstract fetchData(subscription: S): Promise<T>;
  abstract transformData(rawData: any): T;

  async pollUpdates(): Promise<void> {
    for (const subscription of this.subscriptionManager.getActiveSubscriptions()) {
      try {
        const data = await this.fetchData(subscription);
        const callbacks =
          this.subscriptionManager.getCallbacksForSubscription(subscription);

        for (const callback of callbacks) {
          callback(data);
        }
      } catch (error) {
        console.error(`Error polling subscription:`, error);
      }
    }
  }

  addCallback(
    clientId: string,
    subscription: S,
    callback: (payload: T) => void
  ): void {
    this.subscriptionManager.addCallback(clientId, subscription, callback);
  }

  removeCallback(clientId: string, subscription: S): boolean {
    return this.subscriptionManager.removeCallback(clientId, subscription);
  }
}

// Specific handler implementations
export class BookSubscriptionHandler extends PolledSubscriptionHandler<
  L2BookData,
  L2BookSubscription
> {
  constructor(queries: DatabaseQueries) {
    super(
      (clientId, subscription) =>
        `${clientId}:${subscription.type}:${subscription.instrument}:${subscription.groupTicks}`,
      queries
    );
  }

  async fetchData(subscription: L2BookSubscription): Promise<L2BookData> {
    const rawBookData = await this.queries.getOrderbook(
      subscription.instrument,
      20,
      subscription.groupTicks
    );
    return this.transformData(rawBookData);
  }

  transformData(rawData: any): L2BookData {
    return {
      bids: rawData
        .filter((entry: any) => entry.side === "bid")
        .map((entry: any) => ({
          price: entry.price,
          quantity: entry.qty,
        })),
      asks: rawData
        .filter((entry: any) => entry.side === "ask")
        .map((entry: any) => ({
          price: entry.price,
          quantity: entry.qty,
        })),
    };
  }
}

export class CandlestickSubscriptionHandler extends PolledSubscriptionHandler<
  CandlestickData[],
  CandlestickSubscription
> {
  constructor(queries: DatabaseQueries) {
    super(
      (clientId, subscription) =>
        `${clientId}:${subscription.type}:${subscription.instrument}:${subscription.stepSec}`,
      queries
    );
  }

  async fetchData(
    subscription: CandlestickSubscription
  ): Promise<CandlestickData[]> {
    const now = new Date();
    const fromDate = new Date(now.getTime() - subscription.stepSec * 1000 * 10); // Last 10 candles
    const toDate = now;

    const instrument = await this.queries.getInstrument(
      subscription.instrument
    );

    if (!instrument) {
      throw new Error(`Instrument not found: ${subscription.instrument}`);
    }

    const rawData = await this.queries.getCandlestickData(
      instrument?.instrument_id,
      fromDate.toISOString(),
      toDate.toISOString(),
      subscription.stepSec
    );

    return this.transformData(rawData);
  }

  transformData(rawData: any): CandlestickData[] {
    return rawData.map((row: any) => ({
      time: Math.floor(new Date(row.bucket).getTime() / 1000),
      open: row.open,
      high: row.high,
      low: row.low,
      close: row.close,
      volume_trades: row.volume_trades,
      trade_count: row.trade_count,
    }));
  }
}
