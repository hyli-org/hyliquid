import { reactive } from "vue";
import { fetchInstruments, fetchOrderbook, fetchPositions, fetchOrders, fetchFills, fetchBalances } from "./api";
import { useSWR } from "../api_call";
import type { SWRResponse } from "../api_call";
import { watchEffect } from "vue";
import { ref } from "vue";

export type Side = "Bid" | "Ask";
export type OrderType = "Market" | "Limit";
export type OrderStatus = "Open" | "Filled" | "Cancelled" | "Rejected";

export interface Instrument {
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
    type: OrderType;
    price: number | null;
    qty: number;
    qty_filled: number;
    qty_remaining: number;
    status: OrderStatus;
    created_at: Date;
    updated_at: Date;
  }

export interface Fill {
    symbol: string;
    side: Side;
    size: number;
    price: number;
    time: string;
}

export interface Balance {
    asset: string;
    free: number;
    locked: number;
    total: number;
}

// Instruments
const instruments = useSWR<Instrument[]>(fetchInstruments);

export const instrumentsState = reactive({
    search: "",
    selected: null as Instrument | null,
    list: [] as Instrument[],
    fetching: instruments.fetching,
    error: instruments.error,
});

watchEffect(() => {
    const v = instruments.data.value;
    instrumentsState.list = v ?? [];
    if (v && !instrumentsState.selected) {
        instrumentsState.selected = v[0] ?? null;
    }
});

// Function to select instrument by symbol (for URL-based selection)
export function selectInstrumentBySymbol(symbol: string): boolean {
    const instrument = instrumentsState.list.find(instrument => instrument.symbol === symbol);
    if (instrument) {
        instrumentsState.selected = instrument;
        return true;
    }
    return false;
}

// Orderbook state via SWRV
const orderbook = useSWR(() => {
    if (!instrumentsState.selected) throw new Error("No instrument selected");
    return fetchOrderbook(instrumentsState.selected!.symbol);
});

export const orderbookState = reactive({
    bids: [] as OrderbookEntry[],
    asks: [] as OrderbookEntry[],
    mid: 0,
    fetching: orderbook.fetching,
    error: orderbook.error,
});

watchEffect(() => {
    // Clear the orders when changing instrument
    instrumentsState.selected;
    orderbookState.bids = [];
    orderbookState.asks = [];
});

watchEffect(() => {
    const v = orderbook.data.value;
    if (v) {
        orderbookState.bids = v.bids;
        orderbookState.asks = v.asks;
        orderbookState.mid = v.mid;
    }
});

// Activity state via SWRV
const swPositions = useSWR<PerpPosition[]>(fetchPositions);
const swOrders = useSWR<Order[]>(fetchOrders);
const swFills = useSWR<Fill[]>(fetchFills);
const swBalances = useSWR<Balance[]>(() => fetchBalances());

export const activityState = reactive({
    positions: [] as PerpPosition[],
    orders: [] as Order[],
    fills: [] as Fill[],
    balances: [] as Balance[],
    bottomTab: "Positions" as "Positions" | "Orders" | "Fills" | "Balances",
    positionsLoading: swPositions.fetching,
    ordersLoading: swOrders.fetching,
    fillsLoading: swFills.fetching,
    balancesLoading: swBalances.fetching,
    positionsError: swPositions.error,
    ordersError: swOrders.error,
    fillsError: swFills.error,
    balancesError: swBalances.error,
});

watchEffect(() => {
    const positions = swPositions.data.value;
    const orders = swOrders.data.value;
    const fills = swFills.data.value;
    const balances = swBalances.data.value;

    if (positions) activityState.positions = positions;
    if (orders) activityState.orders = orders;
    if (fills) activityState.fills = fills;
    if (balances) activityState.balances = balances;
});

// Order form state
const orderFormState = {
    orderType: ref<OrderType>("Limit"),
    price: ref<number | null>(null),
    size: ref<number | null>(0.1),
    side: ref<Side>("Bid"),
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
        symbol: instrumentsState.selected!.symbol,
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
        if (input.type === "Limit" && !input.price) throw new Error("Price required for limit order");
        if (input.size <= 0) throw new Error("Size must be positive");

        await fetch("http://localhost:9002/create_order", {
            method: "POST",
            headers: {
                "Content-Type": "application/json",
                "x-identity": "test_user",
                "x-public-key": "1234",
                "x-signature": "4564",
            },
            body: JSON.stringify({
                order_id: "", // You may want to generate a unique ID here
                order_side: input.side,
                order_type: input.type,
                pair: input.symbol.split("/"),
                // TODO bertrand: pick decimals from API
                price: input.price,
                // TODO bertrand: pick decimals from API
                quantity: Math.round(input.size * 10000000),
            }),
        });
    });
}
