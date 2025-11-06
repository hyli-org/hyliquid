<script setup lang="ts">
import { computed } from "vue";
import { useRouter } from "vue-router";
import { instrumentsState } from "../trade";

const router = useRouter();

const filteredInstruments = computed(() => {
    const q = instrumentsState.search.trim().toLowerCase();
    const list = instrumentsState.list ?? [];
    if (!q) return list;
    return list.filter((m) => m.symbol.toLowerCase().includes(q));
});

const selectInstrument = (instrument: any) => {
    instrumentsState.selected = instrument;
    // Update URL to include the instrument parameter (encode the symbol for URL safety)
    const encodedSymbol = encodeURIComponent(instrument.symbol);
    router.push(`/trade/${encodedSymbol}`);
};
</script>

<template>
    <aside class="w-64 shrink-0 border-r border-[var(--border-default)]">
        <div class="p-3">
            <input
                :value="instrumentsState.search"
                @input="(e: any) => (instrumentsState.search = e.target.value)"
                class="w-full rounded-md border border-[var(--border-default)] bg-[var(--surface-input)] px-3 py-2 text-sm text-[var(--text-secondary)] placeholder-[var(--text-muted)] outline-none transition focus:border-[var(--accent)] focus:ring-1 focus:ring-[var(--accent-soft)]"
                placeholder="Search instruments"
            />
        </div>
        <div class="overflow-auto h-full">
            <div v-if="instrumentsState.fetching" class="px-3 py-2 text-xs text-[var(--text-muted)]">
                Loading instruments…
            </div>
            <div v-else-if="instrumentsState.error" class="px-3 py-2 text-xs text-[var(--sell-color)]">
                {{ instrumentsState.error }}
            </div>
            <ul>
                <li
                    v-for="m in filteredInstruments"
                    :key="m.symbol"
                    @click="() => selectInstrument(m)"
                    :class="[
                        'group relative flex cursor-pointer items-center justify-between px-4 py-3 transition',
                        instrumentsState.selected?.symbol === m.symbol
                            ? 'border-l-2 border-[var(--accent)] bg-[var(--accent-soft)] shadow-inner'
                            : 'hover:bg-[var(--surface-header)]',
                    ]"
                >
                    <div>
                        <div
                            :class="[
                                'text-sm font-semibold transition',
                                instrumentsState.selected?.symbol === m.symbol
                                    ? 'text-[var(--text-accent)]'
                                    : 'text-[var(--text-primary)]',
                            ]"
                        >
                            {{ m.symbol }}
                        </div>
                        <div class="text-xs text-[var(--text-muted)]">Vol ${{ (m.vol / 1_000_000).toFixed(1) }}M</div>
                    </div>
                    <div class="text-right">
                        <div class="tabular-nums text-sm text-[var(--text-secondary)]">
                            {{ instrumentsState.toRealPrice(m.symbol, m.price) }}
                        </div>
                        <div
                            :class="[
                                'text-xs font-medium',
                                m.change >= 0 ? 'text-[var(--buy-color)]' : 'text-[var(--sell-color)]',
                            ]"
                        >
                            {{ m.change >= 0 ? "+" : "" }}{{ instrumentsState.toRealPrice(m.symbol, m.change) }}%
                        </div>
                    </div>
                </li>
            </ul>
        </div>
    </aside>
</template>

<style scoped>
.tabular-nums {
    font-variant-numeric: tabular-nums;
}
</style>
