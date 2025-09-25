import { reactive } from "vue";
import {
    fetchInstruments,
    fetchPositions,
    fetchOrdersForInstrument,
    fetchFills,
    fetchBalances,
    type PaginationInfo,
    type PaginationParams,
} from "./trade_api";
import { useSWR } from "../api_call";
import type { SWRResponse } from "../api_call";
import { watchEffect } from "vue";
import { ref } from "vue";
import { v7 as uuidv7 } from "uuid";
import { websocketManager } from "./websocket";
import { BACKEND_API_URL } from "../config";
import { useWallet } from "hyli-wallet-vue";
import { encodeToHex } from "../utils";

// Re-export types for components
export type { PaginationInfo, PaginationParams } from "./trade_api";

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
const instrumentsAndAssets = useSWR<{ instruments: Instrument[]; assets: Asset[] }>(fetchInstruments);

export const instrumentsState = reactive({
    search: "",
    selected: null as Instrument | null,
    list: [] as Instrument[],
    fetching: instrumentsAndAssets.fetching,
    error: instrumentsAndAssets.error,
    toRealPrice: (instrument_symbol: string | undefined, price: number) => {
        if (!instrument_symbol)
            return price.toLocaleString(undefined, { minimumFractionDigits: 0, maximumFractionDigits: 0 });
        const quoteAsset = instrument_symbol.split("/")[1];
        const quoteAssetScale = assetsState.list.find((a) => a.symbol === quoteAsset)?.scale ?? 0;
        const real = price / 10 ** quoteAssetScale;
        return real.toLocaleString(undefined, { minimumFractionDigits: 0, maximumFractionDigits: quoteAssetScale });
    },
    toRealQty: (instrument_symbol: string | undefined, qty: number) => {
        if (!instrument_symbol)
            return qty.toLocaleString(undefined, { minimumFractionDigits: 0, maximumFractionDigits: 0 });
        const baseAsset = instrument_symbol.split("/")[0];
        const baseAssetScale = assetsState.list.find((a) => a.symbol === baseAsset)?.scale ?? 0;
        const qty_real = qty / 10 ** baseAssetScale;
        return qty_real.toLocaleString(undefined, { minimumFractionDigits: 0, maximumFractionDigits: baseAssetScale });
    },
    toIntPrice: (instrument_symbol: string | undefined, price: number) => {
        if (!instrument_symbol) return price;
        const quoteAsset = instrument_symbol.split("/")[1];
        const quoteAssetScale = assetsState.list.find((a) => a.symbol === quoteAsset)?.scale ?? 0;
        const int_price = price * 10 ** quoteAssetScale;
        return int_price;
    },
    toIntQty: (instrument_symbol: string | undefined, qty: number) => {
        if (!instrument_symbol) return qty;
        const baseAsset = instrument_symbol.split("/")[0];
        const baseAssetScale = assetsState.list.find((a) => a.symbol === baseAsset)?.scale ?? 0;
        const int_qty = qty * 10 ** baseAssetScale;
        return int_qty;
    },
});

// Assets
export const assetsState = reactive({
    list: [] as Asset[],
    fetching: instrumentsAndAssets.fetching,
    error: instrumentsAndAssets.error,
    toRealQty: (asset_symbol: string, value: number) => {
        const asset = assetsState.list.find((a) => a.symbol === asset_symbol);
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
    const instrument = instrumentsState.list.find((instrument) => instrument.symbol === symbol);
    if (instrument) {
        instrumentsState.selected = instrument;
        return true;
    }
    return false;
}

// Orderbook state using WebSocket manager
export const orderbookState = reactive({
    get bids() {
        return websocketManager.state.bids;
    },
    get asks() {
        return websocketManager.state.asks;
    },
    get mid() {
        return websocketManager.state.mid;
    },
    get connected() {
        return websocketManager.state.connected;
    },
    get error() {
        return websocketManager.state.error;
    },
    fetching: false, // Keep for compatibility
});

watchEffect(() => {
    // Subscribe to new instrument when selection changes
    if (instrumentsState.selected && websocketManager.state.connected) {
        websocketManager.subscribeToOrderbook(instrumentsState.selected.symbol);
    }
});

// Activity state via SWRV
const swPositions = useSWR<PerpPosition[]>(fetchPositions);
const swOrders = useSWR<{ orders: Order[]; pagination?: PaginationInfo }>(() => {
    if (!instrumentsState.selected) throw new Error("No instrument selected");
    const parts = instrumentsState.selected.symbol.split("/");
    if (parts.length !== 2) throw new Error("Invalid instrument symbol format");
    const baseAsset = parts[0]!;
    const quoteAsset = parts[1]!;
    return fetchOrdersForInstrument(baseAsset, quoteAsset);
});
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
    ordersSortBy: "created_at",
    ordersSortOrder: "desc" as "asc" | "desc",
});

watchEffect(() => {
    // Clear the orders when changing instrument
    instrumentsState.selected;
    activityState.orders = [];
    activityState.ordersPagination = null;
    activityState.ordersCurrentPage = 1;
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
    size: ref<number | null>(null),
    side: ref<Side>("Bid"),
    leverage: ref(10),
    orderSubmit: ref<SWRResponse<{ tx_hash: string }> | null>(null),
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
export async function loadOrdersPage(page: number, pageSize?: number, sortBy?: string, sortOrder?: "asc" | "desc") {
    if (!instrumentsState.selected) {
        throw new Error("No instrument selected");
    }

    const pagination: PaginationParams = {
        page,
        limit: pageSize || activityState.ordersPageSize,
        sort_by: sortBy || activityState.ordersSortBy,
        sort_order: sortOrder || activityState.ordersSortOrder,
    };

    // Update state
    activityState.ordersCurrentPage = page;
    if (pageSize) activityState.ordersPageSize = pageSize;
    if (sortBy) activityState.ordersSortBy = sortBy;
    if (sortOrder) activityState.ordersSortOrder = sortOrder;

    // Fetch new data for the selected instrument
    const parts = instrumentsState.selected.symbol.split("/");
    if (parts.length !== 2) throw new Error("Invalid instrument symbol format");
    const baseAsset = parts[0]!;
    const quoteAsset = parts[1]!;
    const result = await fetchOrdersForInstrument(baseAsset, quoteAsset, pagination);
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

export async function changeOrdersSorting(sortBy: string, sortOrder: "asc" | "desc") {
    await loadOrdersPage(1, activityState.ordersPageSize, sortBy, sortOrder);
}

export async function submitOrder() {
    const created = placeOrder({
        symbol: instrumentsState.selected!.symbol,
        side: orderFormState.side.value,
        size: instrumentsState.toIntQty(instrumentsState.selected!.symbol, orderFormState.size.value ?? 0),
        type: orderFormState.orderType.value,
        price: instrumentsState.toIntPrice(instrumentsState.selected!.symbol, orderFormState.price.value ?? 0),
    });
    orderFormState.orderSubmit.value = created;
    // TODO: should we do this?
    // activityState.orders.unshift(created);
}

const { wallet, getOrReuseSessionKey, signMessageWithSessionKey } = useWallet();

export function placeOrder(input: {
    symbol: string;
    side: Side;
    size: number; // integer amount
    type: OrderType;
    price: number | null; // integer price
}): SWRResponse<{ tx_hash: string }> {
    return useSWR(async () => {
        if (input.type === "Limit" && !input.price) throw new Error("Price required for limit order");
        if (input.size <= 0) throw new Error("Size must be positive");

        let nonce = await fetch(`${BACKEND_API_URL.value}/nonce`, {
            method: "GET",
            headers: {
                "x-identity": wallet.value?.address || "tx_sender",
            },
        });

        const res = await fetch(`${BACKEND_API_URL.value}/create_order`, {
            method: "POST",
            headers: {
                "Content-Type": "application/json",
                "x-identity": wallet.value?.address || "tx_sender",
                "x-public-key": (await getOrReuseSessionKey())?.publicKey || "",
                "x-signature": encodeToHex(
                    signMessageWithSessionKey(`${wallet.value?.address || "tx_sender"}:${nonce}:create_order:${1234}`)
                        .signature,
                ),
            },
            body: JSON.stringify({
                order_id: uuidv7(),
                order_side: input.side,
                order_type: input.type,
                pair: input.symbol.split("/"),
                price: input.price,
                quantity: input.size,
            }),
        });

        if (res.ok) {
            return {
                tx_hash: (await res.json()) as string,
            };
        } else {
            throw new Error(`Failed to create order: ${res.status} ${res.statusText}`);
        }
    });
}
