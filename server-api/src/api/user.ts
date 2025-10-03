/**
 * Balance endpoints
 */

import { Elysia } from "elysia";
import { AssetService, UserService } from "../services";
import { authMiddleware, AuthHeaders } from "../middleware/auth";
import { CustomError } from "../middleware/error-handler";
import { PaginationQuery } from "../types";

export const userRoutes = (
  userService: UserService,
  assetService: AssetService
) => {
  return new Elysia({ name: "user" })
    .use(authMiddleware())
    .get("/api/user/balances", async ({ auth }: { auth: AuthHeaders }) => {
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
    .get("/api/user/nonce", async ({ auth }: { auth: AuthHeaders }) => {
      const nonce = await userService.getNonce(auth.user);
      return nonce;
    })
    .get(
      "/api/user/orders",
      async ({ auth, query }: { auth: AuthHeaders; query: any }) => {
        const pagination: PaginationQuery = {
          page: query.page ? parseInt(query.page.toString()) : undefined,
          limit: query.limit ? parseInt(query.limit.toString()) : undefined,
          sort_by: query.sort_by?.toString(),
          sort_order: query.sort_order?.toString() as "asc" | "desc",
        };
        const orders = await userService.getOrdersPaginated(
          auth.user,
          pagination
        );
        return orders;
      }
    )
    .get(
      "/api/user/orders/:baseAssetSymbol/:quoteAssetSymbol",
      async ({
        auth,
        params,
        query,
      }: {
        auth: AuthHeaders;
        params: { baseAssetSymbol: string; quoteAssetSymbol: string };
        query: any;
      }) => {
        const symbol = assetService.getInstrumentSymbol(
          params.baseAssetSymbol,
          params.quoteAssetSymbol
        );
        const instrumentId = assetService.getInstrumentId(symbol);
        if (!instrumentId) {
          throw new CustomError(`Instrument not found: ${symbol}`, 404);
        }

        const pagination: PaginationQuery = {
          page: query.page ? parseInt(query.page.toString()) : undefined,
          limit: query.limit ? parseInt(query.limit.toString()) : undefined,
          sort_by: query.sort_by?.toString(),
          sort_order: query.sort_order?.toString() as "asc" | "desc",
        };
        const orders = await userService.getOrdersByPairPaginated(
          auth.user,
          instrumentId,
          pagination
        );
        return orders;
      }
    )
    .get("/api/user/trades", async ({ auth }: { auth: AuthHeaders }) => {
      const trades = await userService.getTrades(auth.user);
      return trades;
    })
    .get(
      "/api/user/trades/:baseAssetSymbol/:quoteAssetSymbol",
      async ({
        auth,
        params,
      }: {
        auth: AuthHeaders;
        params: { baseAssetSymbol: string; quoteAssetSymbol: string };
      }) => {
        const symbol = assetService.getInstrumentSymbol(
          params.baseAssetSymbol,
          params.quoteAssetSymbol
        );
        const instrumentId = assetService.getInstrumentId(symbol);
        if (!instrumentId) {
          throw new CustomError(`Instrument not found: ${symbol}`, 404);
        }
        const trades = await userService.getTradesByPair(
          auth.user,
          instrumentId
        );
        return trades;
      }
    );
};
