/**
 * Asset service with in-memory caching
 */

import { Asset, Instrument, MarketStatus } from "../types";
import { DatabaseQueries } from "../database/queries";
import { DatabaseCallbacks } from "@/database/callbacks";

export class AssetService {
  private assetMap: Map<string, Asset> = new Map();
  private instrumentMap: Map<string, Instrument> = new Map();
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

    // Load instruments
    const instruments = await this.queries.getAllInstruments();
    this.instrumentMap.clear();
    for (const instrument of instruments) {
      this.instrumentMap.set(instrument.symbol, instrument);
    }

    // Register a callback to reload the asset and instrument maps
    DatabaseCallbacks.getInstance().addInstrumentsCallback(
      "asset-service",
      () => {
        console.log("Got instruments update");
        this.initialize();
      }
    );

    console.log(
      `Loaded ${this.assetMap.size} assets and ${this.instrumentMap.size} instruments into memory`
    );
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
  getInstrument(symbol: string): Instrument | null {
    return this.instrumentMap.get(symbol) || null;
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
  getInstrumentId(symbol: string): number | null {
    return this.getInstrument(symbol)?.instrument_id || null;
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
  getAllInstruments(): Instrument[] {
    return Array.from(this.instrumentMap.values());
  }

  /**
   * Get active instruments only
   */
  getActiveInstruments(): Instrument[] {
    return this.getAllInstruments().filter(
      (instrument) => instrument.status === MarketStatus.ACTIVE
    );
  }

  /**
   * Check if an asset exists
   */
  hasAsset(symbol: string): boolean {
    return this.assetMap.has(symbol);
  }

  /**
   * Check if an instrument exists
   */
  hasInstrument(symbol: string): boolean {
    return this.instrumentMap.has(symbol);
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
  getInstrumentCount(): number {
    return this.instrumentMap.size;
  }
}
