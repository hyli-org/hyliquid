<script setup lang="ts">
import { watchEffect, onMounted } from "vue";
import { useRoute } from "vue-router";
import InstrumentsPanel from "./components/InstrumentsPanel.vue";
import TopBar from "./components/TopBar.vue";
import ChartPlaceholder from "./components/ChartPlaceholder.vue";
import OrderbookComp from "./components/Orderbook.vue";
import OrderForm from "./components/OrderForm.vue";
import BottomTabs from "./components/BottomTabs.vue";
import PositionsTable from "./components/PositionsTable.vue";
import OrdersTable from "./components/OrdersTable.vue";
import FillsTable from "./components/FillsTable.vue";
import BalancesTable from "./components/BalancesTable.vue";
import { activityState, instrumentsState, selectInstrumentBySymbol } from "./trade";

const route = useRoute();

// Map hash values to tab names
const hashToTab: Record<string, "Positions" | "Orders" | "Fills" | "Balances"> = {
    '#positions': 'Positions',
    '#orders': 'Orders',
    '#fills': 'Fills', 
    '#balances': 'Balances'
};

// Initialize tab from URL hash on page load
onMounted(() => {
    const currentHash = window.location.hash;
    const tab = hashToTab[currentHash];
    if (tab) {
        activityState.bottomTab = tab;
    }
});

// Handle URL-based instrument selection
watchEffect(() => {
    const urlInstrument = route.params.instrument as string;
    if (urlInstrument && instrumentsState.list.length > 0) {
        // Decode the instrument symbol from the URL
        const decodedSymbol = decodeURIComponent(urlInstrument);
        selectInstrumentBySymbol(decodedSymbol);
    }
});
</script>

<template>
    <div class="flex h-screen w-full overflow-hidden">
        <InstrumentsPanel />

        <main class="flex min-w-0 grow flex-col bg-neutral-950">
            <TopBar v-if="instrumentsState.selected" />

            <div class="grid grow grid-cols-12 overflow-hidden">
                <section class="col-span-8 border-r border-neutral-800">
                    <ChartPlaceholder />

                    <div class="flex h-150 flex-col p-3">
                        <BottomTabs v-model="activityState.bottomTab" />

                        <component
                            :is="
                                activityState.bottomTab === 'Positions'
                                    ? PositionsTable
                                    : activityState.bottomTab === 'Orders'
                                      ? OrdersTable
                                      : activityState.bottomTab === 'Fills'
                                        ? FillsTable
                                        : BalancesTable
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
                                            pagination: activityState.ordersPagination,
                                        }
                                      : activityState.bottomTab === 'Fills'
                                        ? {
                                              fills: activityState.fills,
                                              loading: activityState.fillsLoading,
                                              error: activityState.fillsError,
                                          }
                                        : {
                                              balances: activityState.balances,
                                              loading: activityState.balancesLoading,
                                              error: activityState.balancesError,
                                          }
                            "
                        />
                    </div>
                </section>

                <OrderbookComp />

                <OrderForm />
            </div>
        </main>
    </div>
</template>

<style scoped>
.tabular-nums {
    font-variant-numeric: tabular-nums;
}
</style>
