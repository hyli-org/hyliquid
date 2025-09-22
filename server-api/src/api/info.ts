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
};
