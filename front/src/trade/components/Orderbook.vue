<script setup lang="ts">
import { computed } from "vue";
import { orderbookState, instrumentsState, activityState } from "../trade";

const midPrice = computed(() => orderbookState.mid);

const lines = 10

// GroupTick options
const groupTickOptions = [
    { value: 0.001 * 10 ** 6, label: "0.001" },
    { value: 0.01 * 10 ** 6, label: "0.01" },
    { value: 0.1 * 10 ** 6, label: "0.1" },
    { value: 1 * 10 ** 6, label: "1" },
    { value: 10 * 10 ** 6, label: "10" },
    { value: 100 * 10 ** 4, label: "100" },
    { value: 1000 * 10 ** 4, label: "1000" }
];

// Calculate maximum quantity for percentage calculations
const maxQuantity = computed(() => {
    const bids = orderbookState.bids || [];
    const asks = orderbookState.asks || [];

    const allQuantities = [...bids, ...asks]
        .filter(entry => entry.quantity > 0)
        .map(entry => entry.quantity);

    return allQuantities.length > 0 ? Math.max(...allQuantities) : 1;
});

// Ensure we always have exactly 6 lines on each side
const displayBids = computed(() => {
    const bids = orderbookState.bids || [];
    const paddedBids = [...bids];

    // Pad with empty entries if we have fewer than 6
    while (paddedBids.length < lines) {
        paddedBids.push({ price: 0, quantity: 0 });
    }

    // Limit to 6 entries
    return paddedBids.slice(0, lines);
});

const displayAsks = computed(() => {
    const asks = orderbookState.asks || [];
    const paddedAsks = [...asks];

    // Pad with empty entries if we have fewer than 6
    while (paddedAsks.length < lines) {
        paddedAsks.unshift({ price: 0, quantity: 0 });
    }

    // Limit to 6 entries
    return paddedAsks.slice(0, lines);
});

// Calculate percentage width for background bars
const getQuantityPercentage = (quantity: number) => {
    if (quantity === 0) return 0;
    return Math.min((quantity / maxQuantity.value) * 100, 100);
};

</script>

<template>
    <section class="col-span-2 border-r border-neutral-800 p-3 flex flex-col">
        <div class="flex justify-between items-center mb-2">
            <div class="text-xs text-neutral-400">Orderbook</div>
            <div class="flex items-center gap-2">
                <label class="text-xs text-neutral-400">Group:</label>
                <select v-model="activityState.orderbookTicks"
                    class="text-xs bg-neutral-800 border border-neutral-700 rounded px-2 py-1 text-neutral-300 focus:outline-none focus:border-neutral-500">
                    <option v-for="option in groupTickOptions" :key="option.value" :value="option.value">
                        {{ option.label }}
                    </option>
                </select>
            </div>
        </div>
        <div v-if="orderbookState.fetching" class="text-xs text-neutral-400">Loading…</div>
        <div v-else-if="orderbookState.error" class="text-xs text-rose-400">{{ orderbookState.error }}</div>
        <template v-else>
            <div class="flex flex-col">
                <!-- Asks section - top half -->
                <div class="space-y-1 h-1/2">
                    <div v-for="(a, index) in displayAsks" :key="'a' + index + '-' + a.price" :class="[
                        'relative flex justify-between text-sm text-rose-300',
                        a.price === 0 ? 'opacity-30' : ''
                    ]">
                        <!-- Background bar for ask -->
                        <div v-if="a.quantity > 0" class="absolute inset-0 bg-rose-500/10"
                            :style="{ width: getQuantityPercentage(a.quantity) + '%' }"></div>
                        <span class="relative z-10 tabular-nums">{{
                            a.price === 0 ? '—' : instrumentsState.toRealPrice(instrumentsState.selected?.symbol,
                                a.price)
                        }}</span>
                        <span class="relative z-10 tabular-nums">{{
                            a.quantity === 0 ? '—' : instrumentsState.toRealQty(instrumentsState.selected?.symbol,
                                a.quantity)
                        }}</span>
                    </div>
                </div>

                <!-- Mid price - center -->
                <div class="border-t border-b border-neutral-800 py-1 text-center text-neutral-300 flex-shrink-0">
                    <span class="tabular-nums">{{
                        instrumentsState.toRealPrice(instrumentsState.selected?.symbol, midPrice)
                    }}</span>
                </div>

                <!-- Bids section - bottom half -->
                <div class="space-y-1 h-1/2">
                    <div v-for="(b, index) in displayBids" :key="'b' + index + '-' + b.price" :class="[
                        'relative flex justify-between text-sm text-emerald-300',
                        b.price === 0 ? 'opacity-30' : ''
                    ]">
                        <!-- Background bar for bid -->
                        <div v-if="b.quantity > 0" class="absolute inset-0 bg-emerald-500/10"
                            :style="{ width: getQuantityPercentage(b.quantity) + '%' }"></div>
                        <span class="relative z-10 tabular-nums">{{
                            b.price === 0 ? '—' : instrumentsState.toRealPrice(instrumentsState.selected?.symbol,
                                b.price)
                        }}</span>
                        <span class="relative z-10 tabular-nums">{{
                            b.quantity === 0 ? '—' : instrumentsState.toRealQty(instrumentsState.selected?.symbol,
                                b.quantity)
                        }}</span>
                    </div>
                </div>
            </div>
        </template>
    </section>
</template>

<style scoped>
.tabular-nums {
    font-variant-numeric: tabular-nums;
}
</style>
