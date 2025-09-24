/**
 * Info endpoint
 */

import { Elysia } from 'elysia';
import { AssetService, BookService } from '../services';

export const infoRoutes = (bookService: BookService, assetService: AssetService) => {
  return new Elysia({ name: 'info' })
    .get('/api/info', async () => {
      const info = await bookService.getInfo();
      const instruments = await assetService.getActiveInstruments();
      const assets = await assetService.getAllAssets();
      return {
        info, 
        assets,
        instruments
      };
    })
    .get('/api/market/price/:baseAssetSymbol/:quoteAssetSymbol', async ({ params }: { params: { baseAssetSymbol: string, quoteAssetSymbol: string } }) => {
      const price = await bookService.getLatestPrice(params.baseAssetSymbol, params.quoteAssetSymbol);
      const change = await bookService.getPriceChange(params.baseAssetSymbol, params.quoteAssetSymbol);
      const vol = await bookService.getVolume(params.baseAssetSymbol, params.quoteAssetSymbol);
      return {price, timestamp: Date.now(), change, vol};
    })
};
