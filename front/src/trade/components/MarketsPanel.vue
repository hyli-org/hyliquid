<script setup lang="ts">
import { computed } from "vue";
import { marketsState } from "../trade";

const filteredMarkets = computed(() => {
    const q = marketsState.search.trim().toLowerCase();
    const list = marketsState.list ?? [];
    if (!q) return list;
    return list.filter((m) => m.symbol.toLowerCase().includes(q));
});
</script>

<template>
    <aside class="w-64 shrink-0 border-r border-neutral-800 bg-neutral-950">
        <div class="p-3 border-b border-neutral-800">
            <input
                :value="marketsState.search"
                @input="(e: any) => (marketsState.search = e.target.value)"
                class="w-full rounded-md bg-neutral-900 px-3 py-2 text-sm placeholder-neutral-500 outline-none focus:ring-1 focus:ring-neutral-700"
                placeholder="Search markets"
            />
        </div>
        <div class="overflow-auto h-full">
            <div v-if="marketsState.fetching" class="px-3 py-2 text-xs text-neutral-400">Loading markets…</div>
            <div v-else-if="marketsState.error" class="px-3 py-2 text-xs text-rose-400">{{ marketsState.error }}</div>
            <ul>
                <li
                    v-for="m in filteredMarkets"
                    :key="m.symbol"
                    @click="() => (marketsState.selected = m)"
                    :class="[
                        'cursor-pointer px-3 py-2 flex items-center justify-between hover:bg-neutral-900',
                        marketsState.selected?.symbol === m.symbol ? 'bg-neutral-900' : '',
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
