import type { Balance, Fill, Instrument, Order, OrderStatus, PerpPosition } from "./trade";

// Base API URL - you may want to make this configurable
const API_BASE_URL = "http://localhost:3000";

// Authentication headers - you may want to get these from a store or context
const getAuthHeaders = () => ({
  "x-user": "tx_sender",     // TODO: Get from auth context
});

// Types for the real API responses
interface ApiInstrument {
  instrument_id: number;
  symbol: string;
  tick_size: number;
  qty_step: number;
  price_scale: number;
  base_asset_id: number;
  quote_asset_id: number;
  status: string;
  created_at: string;
}

interface ApiAsset {
  asset_id: number;
  symbol: string;
  scale: number;
  step: number;
  status: string;
  created_at: string;
}

interface ApiInfoResponse {
  info: any;
  assets: ApiAsset[];
  instruments: ApiInstrument[];
}

interface ApiOrderbookEntry {
  price: number;
  quantity: number;
}

interface ApiOrderbookResponse {
  bids: ApiOrderbookEntry[];
  asks: ApiOrderbookEntry[];
}

interface ApiOrder {
  order_id: number;
  order_signed_id: string;
  instrument_id: number;
  user_id: number;
  side: "bid" | "ask";
  type: "limit" | "market" | "stop_limit" | "stop_market";
  price: number | null;
  qty: number;
  qty_filled: number;
  qty_remaining: number;
  status: "open" | "partially_filled" | "filled" | "cancelled" | "rejected";
  created_at: string;
  updated_at: string;
}

interface ApiTrade {
  trade_id: number;
  instrument_id: number;
  price: number;
  qty: number;
  trade_time: string;
  side: "bid" | "ask";
}

interface ApiBalance {
  token: string;
  total: number;
  reserved: number;
  available: number;
}

// Helper function to transform API instrument to frontend instrument
function transformInstrument(apiInstrument: ApiInstrument, marketPrice?: number, priceChange?: number, vol?: number): Instrument {
  return {
    symbol: apiInstrument.symbol,
    price: marketPrice || 0,
    change: priceChange || 0,
    vol: vol || 0,
  };
}

// Helper function to transform API order to frontend order
function transformOrder(apiOrder: ApiOrder, instruments: ApiInstrument[]): Order {
  const instrument = instruments.find(i => i.instrument_id === apiOrder.instrument_id);
  
  return {
    symbol: instrument?.symbol || "UNKNOWN",
    side: apiOrder.side === "bid" ? "Bid" : "Ask",
    qty: apiOrder.qty,
    qty_filled: apiOrder.qty_filled,
    qty_remaining: apiOrder.qty_remaining,
    type: apiOrder.type === "limit" ? "Limit" : "Market",
    price: apiOrder.price || 0,
    status: apiOrder.status.charAt(0).toUpperCase() + apiOrder.status.slice(1).replace("_", " ") as OrderStatus,
    created_at: new Date(apiOrder.created_at),
    updated_at: new Date(apiOrder.updated_at),
  };
}

// Helper function to transform API trade to frontend fill
function transformTrade(apiTrade: ApiTrade, instruments: ApiInstrument[]): Fill {
  const instrument = instruments.find(i => i.instrument_id === apiTrade.instrument_id);
  
  return {
    symbol: instrument?.symbol || "UNKNOWN",
    side: apiTrade.side === "bid" ? "Bid" : "Ask",
    size: apiTrade.qty,
    price: apiTrade.price,
    time: new Date(apiTrade.trade_time).toLocaleTimeString(),
  };
}


export async function fetchMarketPrice(symbol: string): Promise<{ price: number; change: number; vol: number }> {
  try {
    // Parse symbol to get base and quote assets
    const [baseAsset, quoteAsset] = symbol.split("/");
    
    const response = await fetch(
      `${API_BASE_URL}/api/market/price/${baseAsset}/${quoteAsset}`
    );
    
    if (!response.ok) {
      throw new Error(`Failed to fetch market price: ${response.status} ${response.statusText}`);
    }
    
    const data = await response.json();
    
    return {
      price: data.price || 0,
      change: data.change || 0,
      vol: data.vol || 0,
    };
  } catch (error) {
    console.warn(`Failed to fetch market price for ${symbol}:`, error);
    return { price: 0, change: 0, vol: 0 };
  }
}

export async function fetchInstruments(): Promise<Instrument[]> {
  const response = await fetch(`${API_BASE_URL}/api/info`);
  if (!response.ok) {
    throw new Error(`Failed to fetch instruments: ${response.status} ${response.statusText}`);
  }
  
  const data: ApiInfoResponse = await response.json();

  // Transform API instruments to frontend format (initially with price 0)
  const instruments = await Promise.all(data.instruments
    .filter(inst => inst.status === "active")
    .map(inst => fetchMarketPrice(inst.symbol).then(price => transformInstrument(inst, price.price, price.change, price.vol))));

  return instruments;
}

export async function fetchOrderbook(symbol: string) {
  // Parse symbol to get base and quote assets
  const [baseAsset, quoteAsset] = symbol.split("/");
  
  const response = await fetch(
    `${API_BASE_URL}/api/book/${baseAsset}/${quoteAsset}?levels=20&group_ticks=1`
  );
  
  if (!response.ok) {
    throw new Error(`Failed to fetch orderbook: ${response.status} ${response.statusText}`);
  }
  
  const data: ApiOrderbookResponse = await response.json();
  
  // Calculate mid price
  const mid = data.bids.length > 0 && data.asks.length > 0 
    ? (data.bids[0]!.price + data.asks[0]!.price) / 2 
    : 0;

  console.log("mid", mid);
  console.log("bids", data.bids);
  console.log("asks", data.asks);
  
  return {
    mid,
    bids: data.bids,
    asks: data.asks,
  };
}

export async function fetchPositions(): Promise<PerpPosition[]> {
  return [];
}

export async function fetchOrders(): Promise<Order[]> {
  const response = await fetch(`${API_BASE_URL}/api/user/orders`, {
    headers: {
      "Content-Type": "application/json",
      ...getAuthHeaders(),
    },
  });
  
  if (!response.ok) {
    throw new Error(`Failed to fetch orders: ${response.status} ${response.statusText}`);
  }
  
  const data: { orders: ApiOrder[] } = await response.json();
  
  // Get instruments for symbol mapping
  const instrumentsResponse = await fetch(`${API_BASE_URL}/api/info`);
  const instrumentsData: ApiInfoResponse = await instrumentsResponse.json();
  
  return data.orders.map(order => transformOrder(order, instrumentsData.instruments));
}

export async function fetchFills(): Promise<Fill[]> {
  const response = await fetch(`${API_BASE_URL}/api/user/trades`, {
    headers: {
      "Content-Type": "application/json",
      ...getAuthHeaders(),
    },
  });
  
  if (!response.ok) {
    throw new Error(`Failed to fetch fills: ${response.status} ${response.statusText}`);
  }
  
  const data: { trades: ApiTrade[] } = await response.json();
  
  // Get instruments for symbol mapping
  const instrumentsResponse = await fetch(`${API_BASE_URL}/api/info`);
  const instrumentsData: ApiInfoResponse = await instrumentsResponse.json();
  
  return data.trades.map(trade => transformTrade(trade, instrumentsData.instruments));
}

export async function fetchBalances(): Promise<Balance[]> {
  const response = await fetch(`${API_BASE_URL}/api/user/balances`, {
    headers: {
      "Content-Type": "application/json",
      ...getAuthHeaders(),
    },
  });
  
  if (!response.ok) {
    throw new Error(`Failed to fetch balances: ${response.status} ${response.statusText}`);
  }
  
  const data: { balances: ApiBalance[] } = await response.json();
  
  return data.balances.map(balance => ({
    asset: balance.token,
    free: balance.available,
    locked: balance.reserved,
    total: balance.total,
  }));
}
