/**
 * API routes index
 */

import { Elysia } from 'elysia';
import { healthRoutes } from './health';
import { configRoutes } from './config';
import { infoRoutes } from './info';
import { bookRoutes } from './book';
import { userRoutes } from './user';
import { AssetService, BookService, UserService } from '../services';

export const createApiRoutes = (bookService: BookService, userService: UserService, assetService: AssetService) => {
  return new Elysia()
    .use(healthRoutes())
    .use(configRoutes())
    .use(infoRoutes(bookService, assetService))
    .use(bookRoutes(bookService))
    // Authenticated routes
    .use(userRoutes(userService, assetService))
};
