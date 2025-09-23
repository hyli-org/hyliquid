<script setup lang="ts">
import { computed, reactive, ref } from "vue";
import MarketsPanel from "./components/MarketsPanel.vue";
import TopBar from "./components/TopBar.vue";
import ChartPlaceholder from "./components/ChartPlaceholder.vue";
import OrderbookComp from "./components/Orderbook.vue";
import OrderForm from "./components/OrderForm.vue";
import BottomTabs from "./components/BottomTabs.vue";
import PositionsTable from "./components/PositionsTable.vue";
import OrdersTable from "./components/OrdersTable.vue";
import FillsTable from "./components/FillsTable.vue";
import type { Market, Order, OrderType, Position, Side, Fill } from "./types";

const search = ref("");
const markets = ref<Market[]>([
    { symbol: "BTC-PERP", price: 61234, change: 2.4, vol: 123_000_000 },
    { symbol: "ETH-PERP", price: 2450.12, change: -1.2, vol: 54_000_000 },
    { symbol: "SOL-PERP", price: 178.45, change: 3.8, vol: 22_000_000 },
    { symbol: "LINK-PERP", price: 12.34, change: 0.9, vol: 8_200_000 },
    { symbol: "AVAX-PERP", price: 39.5, change: -0.7, vol: 5_900_000 },
]);

const filteredMarkets = computed(() => {
    const q = search.value.trim().toLowerCase();
    if (!q) return markets.value;
    return markets.value.filter((m) => m.symbol.toLowerCase().includes(q));
});

const selected = ref<Market>(markets.value[0]);

const side = ref<Side>("Long");
const orderType = ref<OrderType>("Limit");
const price = ref<number | null>(selected.value.price);
const size = ref<number | null>(0.1);
const leverage = ref(10);

const orderbook = reactive({
    bids: [
        { p: selected.value.price - 10, q: 12.4 },
        { p: selected.value.price - 8, q: 5.5 },
        { p: selected.value.price - 6, q: 1.7 },
        { p: selected.value.price - 4, q: 3.2 },
        { p: selected.value.price - 2, q: 9.9 },
    ],
    asks: [
        { p: selected.value.price + 2, q: 7.2 },
        { p: selected.value.price + 4, q: 4.3 },
        { p: selected.value.price + 6, q: 2.6 },
        { p: selected.value.price + 8, q: 6.1 },
        { p: selected.value.price + 10, q: 11.7 },
    ],
});

function selectMarket(m: Market) {
    selected.value = m;
    price.value = m.price;
}

const positions = ref<Position[]>([
    { symbol: "BTC-PERP", side: "Long", size: 0.25, entry: 60000, liq: 42000, pnl: 3123.45 },
]);
const orders = ref<Order[]>([
    { symbol: "ETH-PERP", side: "Short", size: 1.0, type: "Limit", price: 2500, status: "Open" },
]);
const fills = ref<Fill[]>([{ symbol: "SOL-PERP", side: "Long", size: 5, price: 176.9, time: "10:21:04" }]);

const bottomTab = ref<"Positions" | "Orders" | "Fills">("Positions");

function submitOrder() {
    if (!size.value || !price.value) return;
    orders.value.unshift({
        symbol: selected.value.symbol,
        side: side.value,
        size: size.value,
        type: orderType.value,
        price: price.value,
        status: "Open",
    });
}
</script>

<template>
    <div class="flex h-screen w-full overflow-hidden">
        <MarketsPanel
            :markets="filteredMarkets"
            :search="search"
            :selected-symbol="selected.symbol"
            @update:search="(v) => (search = v)"
            @select="selectMarket"
        />

        <main class="flex min-w-0 grow flex-col bg-neutral-950">
            <TopBar :selected="selected" :leverage="leverage" @update:leverage="(v) => (leverage = v)" />

            <div class="grid grow grid-cols-12 overflow-hidden">
                <section class="col-span-8 border-r border-neutral-800">
                    <ChartPlaceholder />

                    <div class="flex h-[calc(100%-20rem)] flex-col p-3">
                        <BottomTabs v-model="bottomTab" />

                        <component
                            :is="
                                bottomTab === 'Positions'
                                    ? PositionsTable
                                    : bottomTab === 'Orders'
                                      ? OrdersTable
                                      : FillsTable
                            "
                            v-bind="
                                bottomTab === 'Positions'
                                    ? { positions }
                                    : bottomTab === 'Orders'
                                      ? { orders }
                                      : { fills }
                            "
                        />
                    </div>
                </section>

                <OrderbookComp :asks="orderbook.asks" :bids="orderbook.bids" :mid="selected.price" />

                <OrderForm
                    :side="side"
                    :order-type="orderType"
                    :price="price"
                    :size="size"
                    :leverage="leverage"
                    :base-symbol="selected.symbol.split('-')[0]"
                    @update:side="(v) => (side = v)"
                    @update:orderType="(v) => (orderType = v)"
                    @update:price="(v) => (price = v)"
                    @update:size="(v) => (size = v)"
                    @submit="submitOrder"
                />
            </div>
        </main>
    </div>
</template>

<style scoped>
.tabular-nums {
    font-variant-numeric: tabular-nums;
}
</style>
