<script setup lang="ts">
import MarketsPanel from "./components/MarketsPanel.vue";
import TopBar from "./components/TopBar.vue";
import ChartPlaceholder from "./components/ChartPlaceholder.vue";
import OrderbookComp from "./components/Orderbook.vue";
import OrderForm from "./components/OrderForm.vue";
import BottomTabs from "./components/BottomTabs.vue";
import PositionsTable from "./components/PositionsTable.vue";
import OrdersTable from "./components/OrdersTable.vue";
import FillsTable from "./components/FillsTable.vue";
import { activityState, marketsState, orderFormState, orderbookState, type Market } from "./trade";
import { placeOrder } from "./api";
import { computed } from "vue";

const filteredMarkets = computed(() => {
    const q = marketsState.search.trim().toLowerCase();
    const list = marketsState.list ?? [];
    if (!q) return list;
    return list.filter((m) => m.symbol.toLowerCase().includes(q));
});

const baseSymbol = computed(() => (marketsState.selected ? marketsState.selected.symbol.split("-")[0] : "")!);

// Actions
function selectMarket(m: Market) {
    marketsState.selected = m;
    orderFormState.price = m.price;
}

async function submitOrder() {
    orderFormState.submitError = null;
    orderFormState.submitting = true;
    try {
        const created = await placeOrder({
            symbol: marketsState.selected!.symbol,
            side: orderFormState.side,
            size: orderFormState.size ?? 0,
            type: orderFormState.orderType,
            price: orderFormState.price,
        });
        activityState.orders.unshift(created);
    } catch (e: any) {
        orderFormState.submitError = String(e?.message ?? e);
    } finally {
        orderFormState.submitting = false;
    }
}
</script>

<template>
    <div class="flex h-screen w-full overflow-hidden">
        <MarketsPanel
            :markets="filteredMarkets"
            :search="marketsState.search"
            :selected-symbol="marketsState.selected?.symbol"
            :loading="marketsState.fetching"
            :error="marketsState.error"
            @update:search="(v) => (marketsState.search = v)"
            @select="selectMarket"
        />

        <main class="flex min-w-0 grow flex-col bg-neutral-950">
            <TopBar
                v-if="marketsState.selected"
                :selected="marketsState.selected"
                :leverage="orderFormState.leverage"
                @update:leverage="(v) => (orderFormState.leverage = v)"
            />

            <div class="grid grow grid-cols-12 overflow-hidden">
                <section class="col-span-8 border-r border-neutral-800">
                    <ChartPlaceholder />

                    <div class="flex h-[calc(100%-20rem)] flex-col p-3">
                        <BottomTabs v-model="activityState.bottomTab" />

                        <component
                            :is="
                                activityState.bottomTab === 'Positions'
                                    ? PositionsTable
                                    : activityState.bottomTab === 'Orders'
                                      ? OrdersTable
                                      : FillsTable
                            "
                            v-bind="
                                activityState.bottomTab === 'Positions'
                                    ? {
                                          positions: activityState.positions,
                                          loading: activityState.positionsLoading,
                                          error: activityState.positionsError,
                                      }
                                    : activityState.bottomTab === 'Orders'
                                      ? {
                                            orders: activityState.orders,
                                            loading: activityState.ordersLoading,
                                            error: activityState.ordersError,
                                        }
                                      : {
                                            fills: activityState.fills,
                                            loading: activityState.fillsLoading,
                                            error: activityState.fillsError,
                                        }
                            "
                        />
                    </div>
                </section>

                <OrderbookComp
                    :asks="orderbookState.asks"
                    :bids="orderbookState.bids"
                    :mid="marketsState.selected?.price ?? 0"
                    :loading="orderbookState.fetching"
                    :error="orderbookState.error"
                />

                <OrderForm
                    :side="orderFormState.side"
                    :order-type="orderFormState.orderType"
                    :price="orderFormState.price"
                    :size="orderFormState.size"
                    :leverage="orderFormState.leverage"
                    :base-symbol="baseSymbol"
                    :submitting="orderFormState.submitting"
                    :submit-error="orderFormState.submitError"
                    @update:side="(v) => (orderFormState.side = v)"
                    @update:orderType="(v) => (orderFormState.orderType = v)"
                    @update:price="(v) => (orderFormState.price = v)"
                    @update:size="(v) => (orderFormState.size = v)"
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
