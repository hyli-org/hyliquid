<script setup lang="ts">
import { computed } from "vue";
import { instrumentsState } from "../trade";

const selectedInstrument = computed(() => {
    return instrumentsState.selected!;
});
</script>

<template>
    <div class="flex items-center justify-between border-b border-[var(--border-strong)] px-5 py-3 shadow-sm">
        <div class="flex items-center gap-3">
            <div class="text-lg font-semibold text-[var(--text-accent)]">{{ selectedInstrument.symbol }}</div>
            <div class="text-sm uppercase tracking-wide text-[var(--text-muted)]">Perpetual</div>
            <div class="ml-4 text-sm text-[var(--text-secondary)]">
                <span class="text-[var(--text-muted)]">Mark</span>
                <span class="ml-2 tabular-nums text-[var(--text-accent)]">{{
                    instrumentsState.toRealPrice(selectedInstrument.symbol, selectedInstrument.price)
                }}</span>
            </div>
            <div
                class="text-sm"
                :class="selectedInstrument.change >= 0 ? 'text-[var(--buy-color)]' : 'text-[var(--sell-color)]'"
            >
                {{ selectedInstrument.change >= 0 ? "+" : ""
                }}{{ instrumentsState.toRealPrice(selectedInstrument.symbol, selectedInstrument.change) }}%
            </div>
        </div>
        <div class="flex items-center gap-4 text-sm text-[var(--text-secondary)]">
            <div class="flex items-center gap-2">
                <span
                    class="rounded-full border border-[var(--border-default)] px-3 py-1 text-xs uppercase tracking-wide text-[var(--text-muted)]"
                >
                    Lev x40
                </span>
            </div>
        </div>
    </div>
</template>

<style scoped>
.tabular-nums {
    font-variant-numeric: tabular-nums;
}
</style>
