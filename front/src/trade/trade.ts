import { reactive } from "vue";
import { fetchMarkets, fetchOrderbook, fetchPositions, fetchOrders, fetchFills } from "./api";
import { useSWR } from "../api_call";
import type { SWRResponse } from "../api_call";
import { watchEffect } from "vue";

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

// Markets + selection
const markets = useSWR<Market[]>(fetchMarkets);

export const marketsState = reactive({
    search: "",
    selected: null as Market | null,
    list: [] as Market[],
    fetching: markets.fetching,
    error: markets.error,
});

watchEffect(() => {
    const v = markets.data.value;
    marketsState.list = v ?? [];
    if (v && !marketsState.selected) {
        marketsState.selected = v[0] ?? null;
    }
});

// Order form state
export const orderFormState = reactive({
    side: "Long" as Side,
    orderType: "Limit" as OrderType,
    price: null as number | null,
    size: 0.1 as number | null,
    leverage: 10,
    orderSubmit: null as SWRResponse<void> | null,
});

// Orderbook state via SWRV
const orderbook = useSWR(() => fetchOrderbook(marketsState.selected!.symbol));

export const orderbookState = reactive({
    bids: [] as { p: number; q: number }[],
    asks: [] as { p: number; q: number }[],
    fetching: orderbook.fetching,
    error: null as string | null,
});

watchEffect(() => {
    const v = orderbook.data.value;
    if (v) {
        orderbookState.bids = v.bids;
        orderbookState.asks = v.asks;
        if (marketsState.selected) {
            marketsState.selected = { ...marketsState.selected, price: v.mid };
            if (orderFormState.price == null) orderFormState.price = v.mid;
        }
    }
});

// Activity state via SWRV
const swPositions = useSWR<Position[]>(fetchPositions);
const swOrders = useSWR<Order[]>(fetchOrders);
const swFills = useSWR<Fill[]>(fetchFills);

export const activityState = reactive({
    positions: [] as Position[],
    orders: [] as Order[],
    fills: [] as Fill[],
    bottomTab: "Positions" as "Positions" | "Orders" | "Fills",
    positionsLoading: swPositions.fetching,
    ordersLoading: swOrders.fetching,
    fillsLoading: swFills.fetching,
    positionsError: swPositions.error,
    ordersError: swOrders.error,
    fillsError: swFills.error,
});

watchEffect(() => {
    const positions = swPositions.data.value;
    const orders = swOrders.data.value;
    const fills = swFills.data.value;

    if (positions) activityState.positions = positions;
    if (orders) activityState.orders = orders;
    if (fills) activityState.fills = fills;
});
