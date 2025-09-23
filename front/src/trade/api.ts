import type { Fill, Market, Order, OrderType, Position, Side } from "./trade";

function delay(ms: number) {
    return new Promise((res) => setTimeout(res, ms));
}

function maybeFail(prob = 0.12) {
    if (Math.random() < prob) {
        const errors = ["Network error", "Rate limited", "Internal server error", "Temporary outage"];
        const msg = errors[Math.floor(Math.random() * errors.length)];
        throw new Error(msg);
    }
}

export async function fetchMarkets(): Promise<Market[]> {
    await delay(350 + Math.random() * 300);
    maybeFail(0.08);
    return [
        { symbol: "BTC-PERP", price: 61234, change: +(2.4 + (Math.random() - 0.5)).toFixed(2), vol: 123_000_000 },
        { symbol: "ETH-PERP", price: 2450.12, change: +(-1.2 + (Math.random() - 0.5)).toFixed(2), vol: 54_000_000 },
        { symbol: "SOL-PERP", price: 178.45, change: +(3.8 + (Math.random() - 0.5)).toFixed(2), vol: 22_000_000 },
        { symbol: "LINK-PERP", price: 12.34, change: +(0.9 + (Math.random() - 0.5)).toFixed(2), vol: 8_200_000 },
        { symbol: "AVAX-PERP", price: 39.5, change: +(-0.7 + (Math.random() - 0.5)).toFixed(2), vol: 5_900_000 },
    ];
}

export async function fetchOrderbook(symbol: string) {
    await delay(250 + Math.random() * 250);
    maybeFail(0.1);
    const mid = 60000 + Math.random() * 5000;
    return {
        mid,
        bids: [2, 4, 6, 8, 10].map((d) => ({ p: +(mid - d).toFixed(2), q: +(Math.random() * 12).toFixed(2) })),
        asks: [2, 4, 6, 8, 10].map((d) => ({ p: +(mid + d).toFixed(2), q: +(Math.random() * 12).toFixed(2) })),
    };
}

export async function fetchPositions(): Promise<Position[]> {
    await delay(200 + Math.random() * 200);
    maybeFail(0.06);
    return [
        {
            symbol: "BTC-PERP",
            side: "Long",
            size: 0.25,
            entry: 60000,
            liq: 42000,
            pnl: +(3000 + Math.random() * 500).toFixed(2) as unknown as number,
        },
    ];
}

export async function fetchOrders(): Promise<Order[]> {
    await delay(200 + Math.random() * 200);
    maybeFail(0.06);
    return [{ symbol: "ETH-PERP", side: "Short", size: 1.0, type: "Limit", price: 2500, status: "Open" }];
}

export async function fetchFills(): Promise<Fill[]> {
    await delay(200 + Math.random() * 200);
    maybeFail(0.06);
    return [
        {
            symbol: "SOL-PERP",
            side: "Long",
            size: 5,
            price: +(176 + Math.random() * 5).toFixed(2) as unknown as number,
            time: new Date().toLocaleTimeString(),
        },
    ];
}

export async function placeOrder(input: {
    symbol: string;
    side: Side;
    size: number;
    type: OrderType;
    price: number | null;
}): Promise<Order> {
    await delay(300 + Math.random() * 300);
    maybeFail(0.15);
    if (input.type === "Limit" && !input.price) throw new Error("Price required for limit order");
    if (input.size <= 0) throw new Error("Size must be positive");
    return {
        symbol: input.symbol,
        side: input.side,
        size: input.size,
        type: input.type,
        price: input.type === "Market" ? (input.price ?? 0) : input.price!,
        status: "Open",
    };
}
