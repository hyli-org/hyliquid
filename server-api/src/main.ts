/**
 * Main server entry point
 */

import { Elysia } from 'elysia';
import { DatabaseConfig, DatabaseQueries } from './config/database';
import { getAppConfig } from './config/app';
import { AssetService, UserService, BookService } from './services';
import { createApiRoutes } from './api';
import { corsMiddleware, errorHandler, loggerMiddleware } from './middleware/index';
import { extractRouteInfo, displayRoutes } from './utils/route-info';

async function main() {
  console.log('Starting Hyliquid Server API...');

  // Get application configuration
  const config = getAppConfig();
  console.log('Configuration loaded:', {
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
    console.error('Failed to connect to database. Exiting...');
    process.exit(1);
  }
  console.log('Database connection established');

  // Initialize services
  const assetService = new AssetService(dbQueries);
  const userService = new UserService(dbQueries);
  const bookService = new BookService(dbQueries, assetService);

  // Load data into memory
  try {
    await assetService.initialize();
    await userService.initialize();
    console.log('Services initialized successfully');
  } catch (error) {
    console.error('Failed to initialize services:', error);
    process.exit(1);
  }

  // Create the main application
  const app = new Elysia()
    .use(corsMiddleware())
    .use(errorHandler())
    .use(loggerMiddleware())
    .use(createApiRoutes(bookService, userService, assetService))
    .onError(({ error }: { error: Error }) => {
      console.error('Unhandled error:', error);
    });

  // Start the server
  app.listen({
    port: config.port,
    hostname: config.host,
  });

  console.log(`🚀 Server running at http://${config.host}:${config.port}`);
  
  // Display available endpoints dynamically
  try {
    const routes = extractRouteInfo(app);
    displayRoutes(routes);
  } catch (error) {
    console.warn('Could not extract route information:', error);
    // Fallback to basic message
    console.log('API endpoints are available - check the route definitions for details');
  }

  // Graceful shutdown
  process.on('SIGINT', async () => {
    console.log('\nReceived SIGINT, shutting down gracefully...');
    await dbConfig.close();
    process.exit(0);
  });

  process.on('SIGTERM', async () => {
    console.log('\nReceived SIGTERM, shutting down gracefully...');
    await dbConfig.close();
    process.exit(0);
  });
}

// Handle unhandled promise rejections
process.on('unhandledRejection', (reason: any, promise: Promise<any>) => {
  console.error('Unhandled Rejection at:', promise, 'reason:', reason);
});

// Handle uncaught exceptions
process.on('uncaughtException', (error: Error) => {
  console.error('Uncaught Exception:', error);
  process.exit(1);
});

main().catch((error: Error) => {
  console.error('Failed to start server:', error);
  process.exit(1);
});
