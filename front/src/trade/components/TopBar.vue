<script setup lang="ts">
import type { Market } from "../trade";

const props = defineProps<{
    selected: Market;
    leverage: number;
}>();

const emit = defineEmits<{ (e: "update:leverage", value: number): void }>();
</script>

<template>
    <div class="flex items-center justify-between border-b border-neutral-800 px-4 py-3">
        <div class="flex items-center gap-3">
            <div class="text-lg font-semibold">{{ props.selected.symbol }}</div>
            <div class="text-sm text-neutral-400">Perpetual</div>
            <div class="ml-4 text-sm">
                <span class="text-neutral-400">Mark</span>
                <span class="ml-2 tabular-nums">{{ props.selected.price.toLocaleString() }}</span>
            </div>
            <div class="text-sm" :class="props.selected.change >= 0 ? 'text-emerald-400' : 'text-rose-400'">
                {{ props.selected.change >= 0 ? "+" : "" }}{{ props.selected.change }}%
            </div>
        </div>
        <div class="flex items-center gap-4">
            <div class="flex items-center gap-2 text-sm">
                <span class="text-neutral-400">Lev</span>
                <input
                    :value="props.leverage"
                    @input="(e: any) => emit('update:leverage', Number(e.target.value))"
                    type="range"
                    min="1"
                    max="50"
                    class="h-1 w-40 cursor-pointer appearance-none rounded bg-neutral-800 accent-neutral-300"
                />
                <span class="tabular-nums">{{ props.leverage }}x</span>
            </div>
        </div>
    </div>
</template>

<style scoped>
.tabular-nums {
    font-variant-numeric: tabular-nums;
}
</style>
