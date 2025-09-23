export type Side = "Long" | "Short";
export type OrderType = "Market" | "Limit";

export interface Market {
    symbol: string;
    price: number;
    change: number;
    vol: number;
}

export interface OrderbookLevel {
    p: number; // price
    q: number; // quantity
}

export interface Position {
    symbol: string;
    side: Side;
    size: number;
    entry: number;
    liq: number;
    pnl: number;
}

export interface Order {
    symbol: string;
    side: Side;
    size: number;
    type: OrderType;
    price: number;
    status: string;
}

export interface Fill {
    symbol: string;
    side: Side;
    size: number;
    price: number;
    time: string;
}
