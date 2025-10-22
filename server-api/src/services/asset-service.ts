/**
 * Asset service with in-memory caching
 */

import { Asset, Instrument, MarketStatus } from "../types";
import { DatabaseQueries } from "../database/queries";
import { DatabaseCallbacks } from "@/database/callbacks";

export class AssetService {
  private assetMap: Map<string, Asset> = new Map();
  private queries: DatabaseQueries;

  constructor(queries: DatabaseQueries) {
    this.queries = queries;
  }

  /**
   * Initialize the service by loading all assets and instruments into memory
   */
  async initialize(): Promise<void> {
    console.log("Loading assets and instruments into memory...");

    // Load assets
    const assets = await this.queries.getAllAssets();
    this.assetMap.clear();
    for (const asset of assets) {
      this.assetMap.set(asset.symbol, asset);
    }

    // Register a callback to reload the asset and instrument maps
    DatabaseCallbacks.getInstance().addInstrumentsCallback(
      "asset-service",
      () => {
        console.log("Got instruments update");
        this.initialize();
      }
    );

    console.log(`Loaded ${this.assetMap.size} assets into memory`);
  }

  /**
   * Get an asset by symbol
   */
  getAsset(symbol: string): Asset | null {
    return this.assetMap.get(symbol) || null;
  }

  /**
   * Get an instrument by symbol
   */
  async getInstrument(symbol: string): Promise<Instrument | null> {
    return await this.queries.getInstrument(symbol);
  }

  /**
   * Get instrument symbol by base and quote asset symbols
   */
  getInstrumentSymbol(
    baseAssetSymbol: string,
    quoteAssetSymbol: string
  ): string {
    return `${baseAssetSymbol.toUpperCase()}/${quoteAssetSymbol.toUpperCase()}`;
  }

  /**
   * Get an instrument by id
   */
  async getInstrumentId(symbol: string): Promise<number | null> {
    return (await this.getInstrument(symbol))?.instrument_id || null;
  }

  /**
   * Get all assets
   */
  getAllAssets(): Asset[] {
    return Array.from(this.assetMap.values());
  }

  /**
   * Get all instruments
   */
  async getAllInstruments(): Promise<Instrument[]> {
    return await this.queries.getAllInstruments();
  }

  /**
   * Get active instruments only
   */
  async getActiveInstruments(): Promise<Instrument[]> {
    return (await this.getAllInstruments()).filter(
      (instrument) => instrument.status === MarketStatus.ACTIVE
    );
  }

  /**
   * Check if an asset exists
   */
  async hasAsset(symbol: string): Promise<boolean> {
    return this.assetMap.has(symbol);
  }

  /**
   * Check if an instrument exists
   */
  async hasInstrument(symbol: string): Promise<boolean> {
    return (await this.getInstrument(symbol)) !== null;
  }

  /**
   * Get asset count
   */
  getAssetCount(): number {
    return this.assetMap.size;
  }

  /**
   * Get instrument count
   */
  async getInstrumentCount(): Promise<number> {
    return (await this.getAllInstruments()).length;
  }
}
