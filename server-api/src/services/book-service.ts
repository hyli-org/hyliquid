/**
 * Book service for orderbook operations
 */

import { OrderbookAPI, OrderbookEntry } from '../types';
import { DatabaseQueries } from '../config/database';
import { AssetService } from './asset-service';

export class BookService {
  private queries: DatabaseQueries;
  private assetService: AssetService;

  constructor(queries: DatabaseQueries, assetService: AssetService) {
    this.queries = queries;
    this.assetService = assetService;
  }

  /**
   * Get orderbook information (health check)
   */
  async getInfo(): Promise<string> {
    const isHealthy = await this.queries.healthCheck();
    return isHealthy ? 'Order book info' : 'Order book unavailable';
  }

  /**
   * Get order book for a trading pair
   */
  async getOrderBook(
    baseAssetSymbol: string,
    quoteAssetSymbol: string,
    levels: number = 20,
    groupTicks: number = 10
  ): Promise<OrderbookAPI> {
    const symbol = `${baseAssetSymbol.toUpperCase()}/${quoteAssetSymbol.toUpperCase()}`;
    
    // Validate that the instrument exists
    const instrument = this.assetService.getInstrument(symbol);
    if (!instrument) {
      throw new Error(`Instrument not found: ${symbol}`);
    }

    const rows = await this.queries.getOrderbook(symbol, levels, groupTicks);

    const bids: OrderbookEntry[] = [];
    const asks: OrderbookEntry[] = [];

    for (const row of rows) {
      const entry: OrderbookEntry = {
        price: row.price,
        quantity: row.qty,
      };

      if (row.side === 'bid') {
        bids.push(entry);
      } else if (row.side === 'ask') {
        asks.push(entry);
      }
    }

    return { bids, asks };
  }

  /**
   * Get all available trading pairs
   */
  getAvailablePairs(): string[] {
    return this.assetService.getActiveInstruments().map(instrument => instrument.symbol);
  }

  /**
   * Check if a trading pair exists
   */
  hasPair(baseAssetSymbol: string, quoteAssetSymbol: string): boolean {
    const symbol = `${baseAssetSymbol.toUpperCase()}/${quoteAssetSymbol.toUpperCase()}`;
    return this.assetService.hasInstrument(symbol);
  }

  /**
   * Get instrument details for a trading pair
   */
  getInstrumentDetails(baseAssetSymbol: string, quoteAssetSymbol: string) {
    const symbol = `${baseAssetSymbol.toUpperCase()}/${quoteAssetSymbol.toUpperCase()}`;
    return this.assetService.getInstrument(symbol);
  }
}
