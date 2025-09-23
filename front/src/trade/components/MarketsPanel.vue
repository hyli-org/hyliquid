<script setup lang="ts">
import type { Market } from "../types";

const props = defineProps<{
    markets: Market[];
    search: string;
    selectedSymbol?: string;
}>();

const emit = defineEmits<{
    (e: "update:search", value: string): void;
    (e: "select", market: Market): void;
}>();

function onSearch(e: Event) {
    emit("update:search", (e.target as HTMLInputElement).value);
}
</script>

<template>
    <aside class="w-64 shrink-0 border-r border-neutral-800 bg-neutral-950">
        <div class="p-3 border-b border-neutral-800">
            <input
                :value="props.search"
                @input="onSearch"
                class="w-full rounded-md bg-neutral-900 px-3 py-2 text-sm placeholder-neutral-500 outline-none focus:ring-1 focus:ring-neutral-700"
                placeholder="Search markets"
            />
        </div>
        <div class="overflow-auto h-full">
            <ul>
                <li
                    v-for="m in markets"
                    :key="m.symbol"
                    @click="emit('select', m)"
                    :class="[
                        'cursor-pointer px-3 py-2 flex items-center justify-between hover:bg-neutral-900',
                        props.selectedSymbol === m.symbol ? 'bg-neutral-900' : '',
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
