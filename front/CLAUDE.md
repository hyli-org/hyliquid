# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

This is a React + TypeScript frontend application built with Vite that interacts with smart contracts. The project uses the hyli-wallet library for blockchain integration.

## Development Commands

```bash
# Install dependencies
bun install

# Run development server (hot reload enabled)
bun run dev

# Build for production
bun run build

# Preview production build
bun run preview

# Run linter
bun run lint
```

## Architecture

### Core Application Flow
1. **App.tsx** - Main component that orchestrates:
   - Contract state fetching from indexer API endpoints
   - Transaction submission via `/api/increment`
   - Transaction status polling with 30-second timeout
   - Username management with localStorage persistence

### API Integration Pattern
- All API calls use environment variables for base URLs
- Session management via headers: `x-user`, `x-session-key`, `x-request-signature`
- Contract states fetched from: `/indexer/data/{contractName}/raw_state`
- Transactions sent to: `/api/increment` with blob data

### Key Dependencies
- **hyli-wallet** (v0.3.4) - Wallet integration library
- **crypto-js** & **elliptic** - Cryptographic operations
- **React Router DOM** - Routing (though currently single-page)

## Environment Configuration

The app expects these environment variables (defined in `.env`):
- `VITE_SERVER_BASE_URL` - Main API server
- `VITE_NODE_BASE_URL` - Node API server  
- `VITE_WALLET_SERVER_BASE_URL` - Wallet server
- `VITE_WALLET_WS_URL` - WebSocket connection for wallet

## TypeScript Configuration

- Strict mode enabled
- Target: ES2020
- Module resolution: bundler
- Separate configs for app (`tsconfig.app.json`) and node (`tsconfig.node.json`)

## State Management

- Contract states stored in component state with 1-minute auto-refresh
- Username persisted in localStorage
- Transaction status tracked during polling with real-time UI updates