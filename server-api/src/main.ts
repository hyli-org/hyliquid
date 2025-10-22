/**
 * Main server entry point
 */

import { Elysia } from "elysia";
import { DatabaseConfig } from "./config/database";
import { DatabaseQueries } from "./database/queries";
import { getAppConfig } from "./config/app";
import {
  AssetService,
  UserService,
  BookService,
  WebSocketService,
} from "./services";
import { createApiRoutes } from "./api";
import {
  corsMiddleware,
  errorHandler,
  loggerMiddleware,
} from "./middleware/index";
import { extractRouteInfo, displayRoutes } from "./utils/route-info";
import { DatabaseCallbacks } from "./database/callbacks";

async function main() {
  console.log("Starting Hyliquid Server API...");

  // Get application configuration
  const config = getAppConfig();
  console.log("Configuration loaded:", {
    port: config.port,
    host: config.host,
    contractName: config.contractName,
    nodeEnv: config.nodeEnv,
  });

  // Initialize database
  const dbConfig = DatabaseConfig.getInstance();
  const dbQueries = new DatabaseQueries(dbConfig.getPool());

  // Test database connection
  const isDbConnected = await dbConfig.testConnection();
  if (!isDbConnected) {
    console.error("Failed to connect to database. Exiting...");
    process.exit(1);
  }
  console.log("Database connection established");

  // Initialize services
  const assetService = new AssetService(dbQueries);
  const userService = new UserService(dbQueries);
  const bookService = new BookService(dbQueries, assetService);
  const webSocketService = new WebSocketService(bookService);

  // Initialize database callbacks (this will create the dedicated notification connection)
  const databaseCallbacks = DatabaseCallbacks.getInstance();

  // Load data into memory
  try {
    await assetService.initialize();
    await userService.initialize();
    console.log("Services initialized successfully");
  } catch (error) {
    console.error("Failed to initialize services:", error);
    process.exit(1);
  }

  // Create WebSocket route
  const wsRoute = webSocketService.createWebSocketRoute();

  // Create the main application
  const app = new Elysia()
    .use(corsMiddleware())
    .use(errorHandler())
    .use(loggerMiddleware())
    .use(wsRoute)
    //@ts-ignore
    .use(createApiRoutes(bookService, userService, assetService, dbQueries))
    // Proxy all unknown requests to the Rust server
    .all(
      "/*",
      async ({ request, path }: { request: Request; path: string }) => {
        const targetUrl = `${config.serverBaseUrl}${path}`;

        try {
          // Forward the request to the Rust server
          const response = await fetch(targetUrl, {
            method: request.method,
            headers: request.headers,
            body:
              request.method !== "GET" && request.method !== "HEAD"
                ? await request.blob()
                : undefined,
          });
          console.log(
            `🔄 Forwarded request ${request.method} ${path} to ${targetUrl}`
          );

          // Forward the response back to the client
          return new Response(response.body, {
            status: response.status,
            statusText: response.statusText,
            headers: response.headers,
          });
        } catch (error) {
          console.error(`Proxy error for ${targetUrl}:`, error);
          return new Response("Proxy Error: Unable to forward request", {
            status: 502,
          });
        }
      }
    )
    .onError(({ error }: { error: Error }) => {
      console.error("Unhandled error:", error);
    });

  // Start the server
  app.listen({
    port: config.port,
    hostname: config.host,
  });

  console.log(`🚀 Server running at http://${config.host}:${config.port}`);
  console.log(
    `🔌 WebSocket server running at ws://${config.host}:${config.port}`
  );
  console.log(`🔄 Proxying unknown requests to ${config.serverBaseUrl}`);

  // Display available endpoints dynamically
  try {
    const routes = extractRouteInfo(app);
    displayRoutes(routes);
  } catch (error) {
    console.warn("Could not extract route information:", error);
    // Fallback to basic message
    console.log(
      "API endpoints are available - check the route definitions for details"
    );
  }

  // Graceful shutdown
  process.on("SIGINT", async () => {
    console.log("\nReceived SIGINT, shutting down gracefully...");
    webSocketService.close();
    await databaseCallbacks.close();
    await dbConfig.close();
    process.exit(0);
  });

  process.on("SIGTERM", async () => {
    console.log("\nReceived SIGTERM, shutting down gracefully...");
    webSocketService.close();
    await databaseCallbacks.close();
    await dbConfig.close();
    process.exit(0);
  });
}

// Handle unhandled promise rejections
process.on("unhandledRejection", (reason: any, promise: Promise<any>) => {
  console.error("Unhandled Rejection at:", promise, "reason:", reason);
});

// Handle uncaught exceptions
process.on("uncaughtException", (error: Error) => {
  console.error("Uncaught Exception:", error);
  process.exit(1);
});

main().catch((error: Error) => {
  console.error("Failed to start server:", error);
  process.exit(1);
});
