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
        { symbol: "BTC-USD", price: 61234, change: +(2.4 + (Math.random() - 0.5)).toFixed(2), vol: 123_000_000 },
        { symbol: "ETH-BTC", price: 2450.12, change: +(-1.2 + (Math.random() - 0.5)).toFixed(2), vol: 54_000_000 },
        { symbol: "SOL-USD", price: 178.45, change: +(3.8 + (Math.random() - 0.5)).toFixed(2), vol: 22_000_000 },
        { symbol: "LINK-USD", price: 12.34, change: +(0.9 + (Math.random() - 0.5)).toFixed(2), vol: 8_200_000 },
        { symbol: "AVAX-ETH", price: 39.5, change: +(-0.7 + (Math.random() - 0.5)).toFixed(2), vol: 5_900_000 },
    ];
}

export async function fetchOrderbook(symbol: string) {
    await delay(250 + Math.random() * 250);
    maybeFail(0.1);
    const mid = 60000 + Math.random() * 5000;
    return {
        mid,
        bids: [2, 4, 6, 8, 10].map((d) => ({
            price: +(mid - d).toFixed(2),
            quantity: +(Math.random() * 12).toFixed(2),
        })),
        asks: [2, 4, 6, 8, 10].map((d) => ({
            price: +(mid + d).toFixed(2),
            quantity: +(Math.random() * 12).toFixed(2),
        })),
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
