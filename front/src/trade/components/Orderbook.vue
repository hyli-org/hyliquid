<script setup lang="ts">
import type { OrderbookLevel } from "../trade";

defineProps<{
    asks: OrderbookLevel[];
    bids: OrderbookLevel[];
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
            <div v-for="a in asks" :key="'a' + a.p" class="flex justify-between text-sm text-rose-300">
                <span class="tabular-nums">{{ a.p.toLocaleString() }}</span>
                <span class="tabular-nums">{{ a.q }}</span>
            </div>
        </div>
        <div class="my-2 border-t border-b border-neutral-800 py-1 text-center text-neutral-300">
            <span class="tabular-nums">{{ mid.toLocaleString() }}</span>
        </div>
        <div class="space-y-1">
            <div v-for="b in bids" :key="'b' + b.p" class="flex justify-between text-sm text-emerald-300">
                <span class="tabular-nums">{{ b.p.toLocaleString() }}</span>
                <span class="tabular-nums">{{ b.q }}</span>
            </div>
        </div>
    </section>
</template>

<style scoped>
.tabular-nums {
    font-variant-numeric: tabular-nums;
}
</style>
