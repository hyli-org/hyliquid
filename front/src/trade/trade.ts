import { reactive } from "vue";
import { fetchMarkets, fetchOrderbook, fetchPositions, fetchOrders, fetchFills } from "./mock_api";
import { useSWR } from "../api_call";
import type { SWRResponse } from "../api_call";
import { watchEffect } from "vue";
import { ref } from "vue";

export type Side = "Long" | "Short";
export type OrderType = "Market" | "Limit";

export interface Market {
    symbol: string;
    price: number;
    change: number;
    vol: number;
}

export interface OrderbookEntry {
    price: number;
    quantity: number;
}

export interface PerpPosition {
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

// Orderbook state via SWRV
const orderbook = useSWR(() => {
    if (!marketsState.selected) throw new Error("No market selected");
    return fetchOrderbook(marketsState.selected!.symbol);
});

export const orderbookState = reactive({
    bids: [] as OrderbookEntry[],
    asks: [] as OrderbookEntry[],
    fetching: orderbook.fetching,
    error: orderbook.error,
});

watchEffect(() => {
    // Clear the orders when changing market
    marketsState.selected;
    orderbookState.bids = [];
    orderbookState.asks = [];
});

watchEffect(() => {
    const v = orderbook.data.value;
    if (v) {
        orderbookState.bids = v.bids;
        orderbookState.asks = v.asks;
    }
});

// Activity state via SWRV
const swPositions = useSWR<PerpPosition[]>(fetchPositions);
const swOrders = useSWR<Order[]>(fetchOrders);
const swFills = useSWR<Fill[]>(fetchFills);

export const activityState = reactive({
    positions: [] as PerpPosition[],
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

// Order form state
const orderFormState = {
    orderType: ref<OrderType>("Limit"),
    price: ref<number | null>(null),
    size: ref<number | null>(0.1),
    side: ref<Side>("Long"),
    leverage: ref(10),
    orderSubmit: ref<SWRResponse<void> | null>(null),
};

// Composable for order form state
export function useOrderFormState() {
    return {
        orderType: orderFormState.orderType,
        price: orderFormState.price,
        size: orderFormState.size,
        side: orderFormState.side,
        leverage: orderFormState.leverage,
        orderSubmit: orderFormState.orderSubmit,
    };
}

export async function submitOrder() {
    const created = placeOrder({
        symbol: marketsState.selected!.symbol,
        side: orderFormState.side.value,
        size: orderFormState.size.value ?? 0,
        type: orderFormState.orderType.value,
        price: orderFormState.price.value,
    });
    orderFormState.orderSubmit.value = created;
    // TODO: should we do this?
    // activityState.orders.unshift(created);
}

export function placeOrder(input: {
    symbol: string;
    side: Side;
    size: number;
    type: OrderType;
    price: number | null;
}): SWRResponse<void> {
    return useSWR(async () => {
        // Mock implementation
        if (input.type === "Limit" && !input.price) throw new Error("Price required for limit order");
        if (input.size <= 0) throw new Error("Size must be positive");

        await fetch("/api/place_order", {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify(input),
        });
    });
}
