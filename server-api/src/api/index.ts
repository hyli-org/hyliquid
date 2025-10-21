/**
 * API routes index
 */

import { Elysia } from "elysia";
import { healthRoutes } from "./health";
import { configRoutes } from "./config";
import { infoRoutes } from "./info";
import { bookRoutes } from "./book";
import { userRoutes } from "./user";
import { chartRoutes } from "./chart";
import { AssetService, BookService, UserService } from "../services";
import { DatabaseQueries } from "../database/queries";

export const createApiRoutes = (
  bookService: BookService,
  userService: UserService,
  assetService: AssetService,
  dbQueries: DatabaseQueries
) => {
  return (
    new Elysia()
      .use(healthRoutes())
      .use(configRoutes())
      .use(infoRoutes(bookService, assetService))
      .use(bookRoutes(bookService))
      .use(chartRoutes(dbQueries))
      // Authenticated routes
      .use(userRoutes(userService, assetService))
  );
};
