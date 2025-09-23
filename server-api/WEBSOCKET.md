# WebSocket API Documentation

This document describes the WebSocket functionality added to the Hyliquid Server API.

## Overview

The WebSocket server provides real-time data streaming capabilities with support for channels. Currently, it supports the `l2Book` channel for live order book updates.

## Connection

Connect to the WebSocket server at:
```
ws://localhost:3000
```

## Message Format

### Subscription Messages

All messages must be JSON objects with the following structure:

```json
{
  "method": "subscribe" | "unsubscribe",
  "subscription": {
    "type": "l2Book",
    "instrument": "btc/usdc",
    "groupTicks": 10 
  }
}
```

### Response Messages

The server sends JSON responses with the following structure:

```json
{
  "type": "l2Book",
  "instrument": "btc/usdc",
  "data": {
    "bids": [
      { "price": 45000.5, "quantity": 1.25 },
      { "price": 45000.0, "quantity": 2.5 }
    ],
    "asks": [
      { "price": 45001.0, "quantity": 1.0 },
      { "price": 45001.5, "quantity": 3.75 }
    ]
  },
  "timestamp": 1640995200000
}
```

## Supported Channels

### l2Book Channel

The `l2Book` channel provides real-time level 2 order book data.

#### Parameters:
- `type`: Must be `"l2Book"`
- `instrument`: Trading pair in format `"BASE/QUOTE"` (e.g., `"btc/usdc"`)
- `groupTicks`: Price grouping level (optional, defaults to 10)

#### Features:
- Sends order book updates when an order is made by any user
- Supports multiple clients subscribing to different instruments and groupTicks
- Sends initial data immediately upon subscription

#### Example Usage:

1. **Subscribe to BTC/USDC order book with groupTicks=10:**
```json
{
  "method": "subscribe",
  "subscription": {
    "type": "l2Book",
    "instrument": "btc/usdc",
    "groupTicks": 10
  }
}
```

2. **Unsubscribe from current subscription:**
```json
{
  "method": "unsubscribe",
  "subscription": {
    "type": "l2Book",
    "instrument": "btc/usdc",
    "groupTicks": 10
  }
}
```

3. **Change groupTicks by unsubscribing and resubscribing:**
```json
{"method":"unsubscribe","subscription":{"type":"l2Book","instrument":"btc/usdc","groupTicks":10}}
{"method":"subscribe","subscription":{"type":"l2Book","instrument":"btc/usdc","groupTicks":3}}
```

## Error Handling

The server sends error messages in the following format:

```json
{
  "type": "error",
  "message": "Error description",
  "timestamp": 1640995200000
}
```

Common error scenarios:
- Invalid message format
- Unknown subscription type
- Instrument not found
- Internal server errors

## Status Endpoints

The server provides HTTP endpoints to monitor WebSocket status:

### Get WebSocket Statistics
```
GET /api/websocket/stats
```

Response:
```json
{
  "success": true,
  "data": {
    "connectedClients": 5,
    "activeIntervals": 3,
    "subscriptions": 8
  },
  "timestamp": 1640995200000
}
```

### WebSocket Health Check
```
GET /api/websocket/health
```

Response:
```json
{
  "success": true,
  "status": "healthy",
  "connectedClients": 5,
  "activeIntervals": 3,
  "totalSubscriptions": 8,
  "timestamp": 1640995200000
}
```

## Testing

A test client is provided at `websocket-test-client.js`. To test the WebSocket functionality:

1. Start the server:
```bash
bun run dev
```

2. In another terminal, run the test client:
```bash
node websocket-test-client.js
```
