# Hyliquid Server API

A high-performance TypeScript backend API for the Hyliquid trading platform, built with Bun and Elysia.

## Features

- **High Performance**: Built with Bun runtime and Elysia framework for maximum speed
- **In-Memory Caching**: Assets, instruments, and users are cached in memory for fast access
- **PostgreSQL Integration**: Efficient database queries with connection pooling
- **RESTful API**: Clean REST endpoints matching the Rust implementation
- **Type Safety**: Full TypeScript support with comprehensive type definitions
- **Error Handling**: Robust error handling with proper HTTP status codes
- **CORS Support**: Cross-origin resource sharing enabled
- **Authentication**: Header-based authentication for protected endpoints

## API Endpoints

### Health & Configuration

- `GET /_health` - Health check endpoint
- `GET /api/config` - Get configuration information
- `GET /api/info` - Get order book information

### Trading

- `GET /api/book/{baseAssetSymbol}/{quoteAssetSymbol}` - Get order book for a trading pair
  - Query parameters:
    - `levels` (optional): Number of price levels (default: 20)
    - `group_ticks` (optional): Tick grouping (default: 10)

### User Data

- `GET /api/balances` - Get user balances (requires `x-identity` header)

## Project Structure

```
server-api/
├── src/
│   ├── api/           # API route handlers
│   ├── config/        # Configuration and database setup
│   ├── middleware/    # CORS, auth, error handling
│   ├── services/      # Business logic with in-memory caching
│   ├── types/         # TypeScript type definitions
│   └── index.ts       # Main server entry point
├── package.json
├── tsconfig.json
└── README.md
```

## Setup

1. **Install dependencies:**

   ```bash
   cd server-api
   bun install
   ```

2. **Configure environment:**

   ```bash
   cp .env.example .env
   # Edit .env with your database configuration
   ```

3. **Start the server:**

   ```bash
   # Development mode with hot reload
   bun run dev

   # Production build
   bun run build
   bun start
   ```

## Environment Variables

- `DATABASE_URL` - PostgreSQL connection string
- `PORT` - Server port (default: 3000)
- `HOST` - Server host (default: 0.0.0.0)
- `CONTRACT_NAME` - Contract name for configuration
- `NODE_ENV` - Environment (development/production)

## Database Schema

The API expects a PostgreSQL database with the following tables:

- `assets` - Trading assets (BTC, USDT, etc.)
- `instruments` - Trading pairs (BTC/USDT, etc.)
- `users` - User accounts
- `balances` - User asset balances
- `orders` - Order book orders
- `order_events` - Order event history

See the migration files in the main server directory for the complete schema.

## Performance Features

- **Connection Pooling**: PostgreSQL connection pool with configurable limits
- **In-Memory Caching**: Frequently accessed data cached in memory
- **Optimized Queries**: Efficient database queries with proper indexing
- **Async/Await**: Full async support for non-blocking operations

## Development

- **Type Checking**: `bun run type-check`
- **Linting**: `bun run lint`
- **Testing**: `bun test`

## Architecture

The API follows a clean architecture pattern:

1. **Routes** (`src/api/`) - Handle HTTP requests and responses
2. **Services** (`src/services/`) - Business logic with caching
3. **Config** (`src/config/`) - Database and application configuration
4. **Middleware** (`src/middleware/`) - Cross-cutting concerns
5. **Types** (`src/types/`) - TypeScript definitions

This structure ensures maintainability, testability, and scalability as the project grows.
