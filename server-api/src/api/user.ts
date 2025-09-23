/**
 * Balance endpoints
 */

import { Elysia } from 'elysia';
import { AssetService, UserService } from '../services';
import { authMiddleware, AuthHeaders } from '../middleware/auth';
import { CustomError } from '../middleware/error-handler';

export const userRoutes = (userService: UserService, assetService: AssetService) => {
  return new Elysia({ name: 'user' })
    .use(authMiddleware())
    .get('/api/user/balances', async ({ auth }: { auth: AuthHeaders }) => {
      try {
        const balances = await userService.getBalances(auth.user);
        return balances;
      } catch (error) {
        if (error instanceof Error) {
          throw new CustomError(error.message, 404);
        }
        throw error;
      }
    })
    .get('/api/user/orders', async ({ auth }: { auth: AuthHeaders }) => {
      const orders = await userService.getOrders(auth.user);
      return orders;
    })
    .get('/api/user/orders/:baseAssetSymbol/:quoteAssetSymbol', async ({ auth, params }: { auth: AuthHeaders, params: { baseAssetSymbol: string, quoteAssetSymbol: string } }) => {
      const symbol = assetService.getInstrumentSymbol(params.baseAssetSymbol, params.quoteAssetSymbol);
      const instrumentId = assetService.getInstrumentId(symbol);
      if (!instrumentId) {
        throw new CustomError(`Instrument not found: ${symbol}`, 404);
      }
      const orders = await userService.getOrdersByPair(auth.user, instrumentId);
      return orders;
    })
    ;
};
