<script setup lang="ts">
import { computed } from "vue";
import { orderbookState, instrumentsState, activityState, assetsState } from "../trade";

const midPrice = computed(() => orderbookState.mid);

const lines = 10;

// GroupTick options
const groupTickOptions = computed(() => {
    const quoteAsset = instrumentsState.selected?.quote_asset;
    const scale = assetsState.list.find((a) => a.symbol === quoteAsset)?.scale ?? 0;

    console.log("quoteAsset", quoteAsset);
    console.log(
        "assetsState.list",
        assetsState.list.find((a) => a.symbol === quoteAsset),
    );
    console.log("scale", scale);

    return [
        { value: 0.001 * 10 ** scale, label: "0.001" },
        { value: 0.01 * 10 ** scale, label: "0.01" },
        { value: 0.1 * 10 ** scale, label: "0.1" },
        { value: 1 * 10 ** scale, label: "1" },
        { value: 10 * 10 ** scale, label: "10" },
        { value: 100 * 10 ** scale, label: "100" },
        { value: 1000 * 10 ** scale, label: "1000" },
    ].filter((option) => option.value >= 1);
});

// Calculate maximum quantity for percentage calculations
const maxQuantity = computed(() => {
    const bids = orderbookState.bids || [];
    const asks = orderbookState.asks || [];

    const allQuantities = [...bids, ...asks].filter((entry) => entry.quantity > 0).map((entry) => entry.quantity);

    return allQuantities.length > 0 ? Math.max(...allQuantities) : 1;
});

// Ensure we always have exactly 6 lines on each side with cumulative totals
const displayBids = computed(() => {
    const bids = orderbookState.bids || [];
    const paddedBids = [...bids];

    // Pad with empty entries if we have fewer than 6
    while (paddedBids.length < lines) {
        paddedBids.push({ price: 0, quantity: 0 });
    }

    // Limit to 6 entries and calculate cumulative totals
    const limitedBids = paddedBids.slice(0, lines);
    let cumulativeTotal = 0;

    return limitedBids.map((bid) => {
        if (bid.quantity > 0) {
            cumulativeTotal += bid.quantity;
        }
        return {
            ...bid,
            total: bid.quantity > 0 ? cumulativeTotal : 0,
        };
    });
});

const displayAsks = computed(() => {
    const asks = orderbookState.asks || [];
    const paddedAsks = [...asks];

    // Pad with empty entries if we have fewer than 6
    while (paddedAsks.length < lines) {
        paddedAsks.unshift({ price: 0, quantity: 0 });
    }

    // Limit to 6 entries and calculate cumulative totals (from bottom to top for asks)
    const limitedAsks = paddedAsks.slice(0, lines);
    let cumulativeTotal = 0;

    // Calculate totals from bottom to top (reverse order for asks)
    const reversedAsks = [...limitedAsks].reverse();
    const asksWithTotals = reversedAsks.map((ask) => {
        if (ask.quantity > 0) {
            cumulativeTotal += ask.quantity;
        }
        return {
            ...ask,
            total: ask.quantity > 0 ? cumulativeTotal : 0,
        };
    });

    // Return in original order (top to bottom)
    return asksWithTotals.reverse();
});

// Calculate percentage width for background bars
const getQuantityPercentage = (quantity: number) => {
    if (quantity === 0) return 0;
    return Math.min((quantity / maxQuantity.value) * 100, 100);
};
</script>

<template>
    <section class="col-span-2 flex flex-col border-r border-[var(--border-default)] p-3">
        <div class="flex justify-between items-center mb-2">
            <div class="text-xs font-semibold uppercase tracking-wide text-[var(--text-accent)]">Orderbook</div>
            <div class="flex items-center gap-2">
                <label class="text-xs text-[var(--text-muted)]"></label>
                <select
                    v-model="activityState.orderbookTicks"
                    class="rounded border border-[var(--border-default)] bg-[var(--surface-input)] px-2 py-1 text-xs text-[var(--text-secondary)] focus:border-[var(--accent)] focus:outline-none"
                >
                    <option v-for="option in groupTickOptions" :key="option.value" :value="option.value">
                        {{ option.label }}
                    </option>
                </select>
            </div>
        </div>
        <div v-if="orderbookState.fetching" class="text-xs text-[var(--text-muted)]">Loading…</div>
        <div v-else-if="orderbookState.error" class="text-xs text-[var(--sell-color)]">{{ orderbookState.error }}</div>
        <template v-else>
            <!-- Headers -->
            <div
                class="mb-2 grid grid-cols-3 border-b border-[var(--border-default)] pb-1 text-xs text-[var(--text-muted)]"
            >
                <span class="text-left">Price</span>
                <span class="text-center">Size ({{ instrumentsState.selected?.base_asset }})</span>
                <span class="text-right">Total</span>
            </div>
            <div class="flex flex-col">
                <!-- Asks section - top half -->
                <div class="space-y-1 h-1/2">
                    <div
                        v-for="(a, index) in displayAsks"
                        :key="'a' + index + '-' + a.price"
                        :class="[
                            'relative grid grid-cols-3 text-sm text-[var(--sell-color)]',
                            a.price === 0 ? 'opacity-30' : 'drop-shadow-sm',
                        ]"
                    >
                        <!-- Background bar for ask -->
                        <div
                            v-if="a.quantity > 0"
                            class="absolute inset-0 bg-[var(--sell-soft)]"
                            :style="{ width: getQuantityPercentage(a.quantity) + '%' }"
                        ></div>
                        <span class="relative z-10 tabular-nums text-left">{{
                            a.price === 0
                                ? "—"
                                : instrumentsState.toRealPrice(instrumentsState.selected?.symbol, a.price)
                        }}</span>
                        <span class="relative z-10 tabular-nums text-center">{{
                            a.quantity === 0
                                ? "—"
                                : instrumentsState.toRealQty(instrumentsState.selected?.symbol, a.quantity)
                        }}</span>
                        <span class="relative z-10 tabular-nums text-right">{{
                            a.total === 0 ? "—" : instrumentsState.toRealQty(instrumentsState.selected?.symbol, a.total)
                        }}</span>
                    </div>
                </div>

                <!-- Mid price - center -->
                <div
                    class="flex-shrink-0 border-y border-[var(--border-default)] py-1 my-3 text-center text-[var(--text-accent)]"
                >
                    <span class="tabular-nums font-semibold">{{
                        instrumentsState.toRealPrice(instrumentsState.selected?.symbol, midPrice)
                    }}</span>
                </div>

                <!-- Bids section - bottom half -->
                <div class="space-y-1 h-1/2">
                    <div
                        v-for="(b, index) in displayBids"
                        :key="'b' + index + '-' + b.price"
                        :class="[
                            'relative grid grid-cols-3 text-sm text-[var(--buy-color)]',
                            b.price === 0 ? 'opacity-30' : '',
                        ]"
                    >
                        <!-- Background bar for bid -->
                        <div
                            v-if="b.quantity > 0"
                            class="absolute inset-0 bg-[var(--buy-soft)]"
                            :style="{ width: getQuantityPercentage(b.quantity) + '%' }"
                        ></div>
                        <span class="relative z-10 tabular-nums text-left">{{
                            b.price === 0
                                ? "—"
                                : instrumentsState.toRealPrice(instrumentsState.selected?.symbol, b.price)
                        }}</span>
                        <span class="relative z-10 tabular-nums text-center">{{
                            b.quantity === 0
                                ? "—"
                                : instrumentsState.toRealQty(instrumentsState.selected?.symbol, b.quantity)
                        }}</span>
                        <span class="relative z-10 tabular-nums text-right">{{
                            b.total === 0 ? "—" : instrumentsState.toRealQty(instrumentsState.selected?.symbol, b.total)
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
