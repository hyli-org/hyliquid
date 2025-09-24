import { reactive } from "vue";
import { fetchInstruments, fetchOrderbook, fetchPositions, fetchOrders, fetchFills, fetchBalances, type PaginationInfo, type PaginationParams } from "./api";
import { useSWR } from "../api_call";
import type { SWRResponse } from "../api_call";
import { watchEffect } from "vue";
import { ref } from "vue";

// Re-export types for components
export type { PaginationInfo, PaginationParams } from "./api";

export type Side = "Bid" | "Ask";
export type OrderType = "Market" | "Limit";
export type OrderStatus = "Open" | "Filled" | "Cancelled" | "Rejected";

export interface Asset {
    symbol: string;
    scale: number;
    step: number;
}

export interface Instrument {
    symbol: string;
    price: number;
    price_scale: number;
    qty_step: number;
    base_asset: string;
    quote_asset: string;
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
    available: number;
    locked: number;
    total: number;
}

// Instruments
const instrumentsAndAssets = useSWR<{instruments: Instrument[], assets: Asset[]}>(fetchInstruments);

export const instrumentsState = reactive({
    search: "",
    selected: null as Instrument | null,
    list: [] as Instrument[],
    fetching: instrumentsAndAssets.fetching,
    error: instrumentsAndAssets.error,
    toRealPrice: (instrument_symbol: string, price: number) => {
        const quoteAsset = instrumentsState.list.find(i => i.symbol === instrument_symbol)?.quote_asset;
        const priceScale = instrumentsState.list.find(i => i.symbol === instrument_symbol)?.price_scale ?? 0;
        const quoteAssetScale = assetsState.list.find(a => a.symbol === quoteAsset)?.scale ?? 0;
        const real = price / 10 ** (quoteAssetScale + priceScale);
        return real.toLocaleString(undefined, { minimumFractionDigits: 0, maximumFractionDigits: quoteAssetScale });
    },
    toRealQty: (instrument_symbol: string, qty: number) => {
        const baseAsset = instrumentsState.list.find(i => i.symbol === instrument_symbol)?.base_asset;
        const baseAssetScale = assetsState.list.find(a => a.symbol === baseAsset)?.scale ?? 0;
        const real = qty / 10 ** baseAssetScale;
        return real.toLocaleString(undefined, { minimumFractionDigits: 0, maximumFractionDigits: baseAssetScale });
    },
});

// Assets
export const assetsState = reactive({
    list: [] as Asset[],
    fetching: instrumentsAndAssets.fetching,
    error: instrumentsAndAssets.error,
    toRealQty: (asset_symbol: string, value: number) => {
        const asset = assetsState.list.find(a => a.symbol === asset_symbol);
        const real = value / 10 ** (asset?.scale ?? 0);
        return real.toLocaleString(undefined, { minimumFractionDigits: 0, maximumFractionDigits: asset?.scale ?? 0 });
    },
});

watchEffect(() => {
    const v = instrumentsAndAssets.data.value?.instruments ?? [];
    instrumentsState.list = v ?? [];
    const a = instrumentsAndAssets.data.value?.assets ?? [];
    assetsState.list = a ?? [];
    if (v && !instrumentsState.selected) {
        instrumentsState.selected = v[0] ?? null;
    }
});

watchEffect(() => {
    const v = instrumentsAndAssets.data.value?.assets ?? [];
    assetsState.list = v ?? [];
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
const swOrders = useSWR<{ orders: Order[], pagination?: PaginationInfo }>(() => fetchOrders());
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
    // Pagination state for orders
    ordersPagination: null as PaginationInfo | null,
    ordersCurrentPage: 1,
    ordersPageSize: 20,
    ordersSortBy: 'created_at',
    ordersSortOrder: 'desc' as 'asc' | 'desc',
});

watchEffect(() => {
    const positions = swPositions.data.value;
    const ordersData = swOrders.data.value;
    const fills = swFills.data.value;
    const balances = swBalances.data.value;

    if (positions) activityState.positions = positions;
    if (ordersData) {
        activityState.orders = ordersData.orders;
        if (ordersData.pagination) {
            activityState.ordersPagination = ordersData.pagination;
        }
    }
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

// Functions to handle orders pagination
export async function loadOrdersPage(page: number, pageSize?: number, sortBy?: string, sortOrder?: 'asc' | 'desc') {
    const pagination: PaginationParams = {
        page,
        limit: pageSize || activityState.ordersPageSize,
        sort_by: sortBy || activityState.ordersSortBy,
        sort_order: sortOrder || activityState.ordersSortOrder
    };
    
    // Update state
    activityState.ordersCurrentPage = page;
    if (pageSize) activityState.ordersPageSize = pageSize;
    if (sortBy) activityState.ordersSortBy = sortBy;
    if (sortOrder) activityState.ordersSortOrder = sortOrder;
    
    // Fetch new data
    const result = await fetchOrders(pagination);
    activityState.orders = result.orders;
    if (result.pagination) {
        activityState.ordersPagination = result.pagination;
    }
}

export async function nextOrdersPage() {
    if (activityState.ordersPagination?.has_next) {
        await loadOrdersPage(activityState.ordersCurrentPage + 1);
    }
}

export async function prevOrdersPage() {
    if (activityState.ordersPagination?.has_prev) {
        await loadOrdersPage(activityState.ordersCurrentPage - 1);
    }
}

export async function changeOrdersSorting(sortBy: string, sortOrder: 'asc' | 'desc') {
    await loadOrdersPage(1, activityState.ordersPageSize, sortBy, sortOrder);
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
