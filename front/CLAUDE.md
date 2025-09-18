# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Blackjack-Win95 is a React + TypeScript frontend application built with Vite that interacts with Hyli blockchain smart contracts. It features wallet integration, contract state monitoring, and transaction submission capabilities.

## Development Commands

```bash
# Install dependencies (using bun or npm)
bun install
npm install

# Run development server (port 5173 by default)
bun run dev
npm run dev

# Build for production
bun run build
npm run build

# Run linter
bun run lint
npm run lint

# Preview production build
bun run preview
npm run preview
```

## Architecture

### Application Structure
- **src/App.tsx** - Main component with three primary views:
  - `LandingPage` - Authentication UI with wallet providers (password, Google, GitHub)
  - `ScaffoldApp` - Main application UI after authentication
  - `AppContent` - Router that displays based on wallet connection status

### Core Features Implementation

1. **Wallet Integration (hyli-wallet v0.3.4)**
   - WalletProvider wraps the app with configuration for node URLs and session keys
   - Session keys configured with 24-hour duration and contract whitelist
   - Identity blobs created via `createIdentityBlobs()` for transactions

2. **Contract State Management**
   - States fetched from `/v1/indexer/contract/{contractName}/state` endpoint
   - Auto-refresh every 60 seconds via `setInterval`
   - Error handling for failed fetches with user-friendly messages

3. **Transaction Flow**
   - Submit transactions to `/api/increment` with wallet blobs
   - Poll transaction status at `/v1/indexer/transaction/hash/{txHash}`
   - 30-second polling timeout with 1-second intervals
   - Real-time UI updates showing initial submission and confirmation status

### API Communication Pattern
- Base URLs configured via environment variables
- Authentication headers: `x-user`, `x-session-key`, `x-request-signature`
- JSON request/response format
- Comprehensive error handling with status code and message display

## Environment Configuration

Required environment variables (`.env` file):
- `VITE_SERVER_BASE_URL` - Main API server (default: http://localhost:9003)
- `VITE_NODE_BASE_URL` - Blockchain node API (default: http://localhost:4321)
- `VITE_WALLET_SERVER_BASE_URL` - Wallet service (default: http://localhost:4000)
- `VITE_WALLET_WS_URL` - WebSocket for wallet updates (default: ws://localhost:8081/ws)

## Build Configuration

### Vite Configuration
- React plugin for JSX transformation
- Optimized for Noir-lang libraries (excluded from dependency optimization)
- Buffer polyfill configured
- Global defined as globalThis for browser compatibility

### TypeScript Configuration
- Strict mode enabled with all checks
- Target: ES2020 with DOM libraries
- Module: ESNext with bundler resolution
- JSX: react-jsx
- Separate configs for app and node environments