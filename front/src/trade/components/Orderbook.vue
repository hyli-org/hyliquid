<script setup lang="ts">
import type { OrderbookEntry } from "../trade";

defineProps<{
    asks: OrderbookEntry[];
    bids: OrderbookEntry[];
    mid: number;
    loading?: boolean;
    error?: string | null;
}>();
</script>

<template>
    <section class="col-span-2 border-r border-neutral-800 p-3">
        <div class="text-xs mb-2 text-neutral-400">Orderbook</div>
        <div v-if="loading" class="text-xs text-neutral-400">Loading…</div>
        <div v-else-if="error" class="text-xs text-rose-400">{{ error }}</div>
        <div class="space-y-1">
            <div v-for="a in asks" :key="'a' + a.price" class="flex justify-between text-sm text-rose-300">
                <span class="tabular-nums">{{ a.price.toLocaleString() }}</span>
                <span class="tabular-nums">{{ a.quantity }}</span>
            </div>
        </div>
        <div class="my-2 border-t border-b border-neutral-800 py-1 text-center text-neutral-300">
            <span class="tabular-nums">{{ mid.toLocaleString() }}</span>
        </div>
        <div class="space-y-1">
            <div v-for="b in bids" :key="'b' + b.price" class="flex justify-between text-sm text-emerald-300">
                <span class="tabular-nums">{{ b.price.toLocaleString() }}</span>
                <span class="tabular-nums">{{ b.quantity }}</span>
            </div>
        </div>
    </section>
</template>

<style scoped>
.tabular-nums {
    font-variant-numeric: tabular-nums;
}
</style>
