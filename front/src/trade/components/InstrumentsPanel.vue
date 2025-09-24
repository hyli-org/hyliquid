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
    <aside class="w-64 shrink-0 border-r border-neutral-800 bg-neutral-950">
        <div class="p-3 border-b border-neutral-800">
            <input
                :value="instrumentsState.search"
                @input="(e: any) => (instrumentsState.search = e.target.value)"
                class="w-full rounded-md bg-neutral-900 px-3 py-2 text-sm placeholder-neutral-500 outline-none focus:ring-1 focus:ring-neutral-700"
                placeholder="Search instruments"
            />
        </div>
        <div class="overflow-auto h-full">
            <div v-if="instrumentsState.fetching" class="px-3 py-2 text-xs text-neutral-400">Loading instruments…</div>
            <div v-else-if="instrumentsState.error" class="px-3 py-2 text-xs text-rose-400">
                {{ instrumentsState.error }}
            </div>
            <ul>
                <li
                    v-for="m in filteredInstruments"
                    :key="m.symbol"
                    @click="() => selectInstrument(m)"
                    :class="[
                        'cursor-pointer px-3 py-2 flex items-center justify-between hover:bg-neutral-900',
                        instrumentsState.selected?.symbol === m.symbol ? 'bg-neutral-900' : '',
                    ]"
                >
                    <div>
                        <div class="text-sm font-medium">{{ m.symbol }}</div>
                        <div class="text-xs text-neutral-500">Vol ${{ (m.vol / 1_000_000).toFixed(1) }}M</div>
                    </div>
                    <div class="text-right">
                        <div class="text-sm tabular-nums">{{ m.price.toLocaleString() }}</div>
                        <div :class="['text-xs', m.change >= 0 ? 'text-emerald-400' : 'text-rose-400']">
                            {{ m.change >= 0 ? "+" : "" }}{{ m.change }}%
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
