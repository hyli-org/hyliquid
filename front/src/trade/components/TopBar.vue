<script setup lang="ts">
import { computed } from "vue";
import { instrumentsState } from "../trade";

const selectedInstrument = computed(() => {
    return instrumentsState.selected!;
});
</script>

<template>
    <div class="flex items-center justify-between border-b border-neutral-800 px-4 py-3">
        <div class="flex items-center gap-3">
            <div class="text-lg font-semibold">{{ selectedInstrument.symbol }}</div>
            <div class="text-sm text-neutral-400">Perpetual</div>
            <div class="ml-4 text-sm">
                <span class="text-neutral-400">Mark</span>
                <span class="ml-2 tabular-nums">{{ selectedInstrument.price.toLocaleString() }}</span>
            </div>
            <div class="text-sm" :class="selectedInstrument.change >= 0 ? 'text-emerald-400' : 'text-rose-400'">
                {{ selectedInstrument.change >= 0 ? "+" : "" }}{{ selectedInstrument.change }}%
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
