<script setup lang="ts">
import { computed } from "vue";
import { marketsState } from "../trade";

const selectedMarket = computed(() => {
    return marketsState.selected!;
});
</script>

<template>
    <div class="flex items-center justify-between border-b border-neutral-800 px-4 py-3">
        <div class="flex items-center gap-3">
            <div class="text-lg font-semibold">{{ selectedMarket.symbol }}</div>
            <div class="text-sm text-neutral-400">Perpetual</div>
            <div class="ml-4 text-sm">
                <span class="text-neutral-400">Mark</span>
                <span class="ml-2 tabular-nums">{{ selectedMarket.price.toLocaleString() }}</span>
            </div>
            <div class="text-sm" :class="selectedMarket.change >= 0 ? 'text-emerald-400' : 'text-rose-400'">
                {{ selectedMarket.change >= 0 ? "+" : "" }}{{ selectedMarket.change }}%
            </div>
        </div>
        <div class="flex items-center gap-4">
            <div class="flex items-center gap-2 text-sm">
                <span class="text-neutral-400">Lev x40</span>
            </div>
        </div>
    </div>
</template>

<style scoped>
.tabular-nums {
    font-variant-numeric: tabular-nums;
}
</style>
