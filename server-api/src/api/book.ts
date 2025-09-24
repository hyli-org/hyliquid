/**
 * Order book endpoints
 */

import { Elysia } from 'elysia';
import { GetBookQuery } from '../types';
import { BookService } from '../services';
import { CustomError } from '../middleware/error-handler';

export const bookRoutes = (bookService: BookService) => {
  return new Elysia({ name: 'book' })
    .get(
      '/api/book/:baseAssetSymbol/:quoteAssetSymbol',
      async ({ params, query }) => {
        const { baseAssetSymbol, quoteAssetSymbol } = params as {
          baseAssetSymbol: string;
          quoteAssetSymbol: string;
        };

        const queryParams = query as GetBookQuery;
        const levels = queryParams.levels ? parseInt(queryParams.levels.toString()) : 10;
        const groupTicks = queryParams.group_ticks ? parseInt(queryParams.group_ticks.toString()) : 10;

        try {
          const book = await bookService.getOrderBook(
            baseAssetSymbol,
            quoteAssetSymbol,
            levels,
            groupTicks
          );
          return book;
        } catch (error) {
          if (error instanceof Error) {
            throw new CustomError(error.message, 404);
          }
          throw error;
        }
      }
    );
};
