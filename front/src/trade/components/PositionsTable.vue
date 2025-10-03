<script setup lang="ts">
import type { PerpPosition } from "../trade";
defineProps<{ positions: PerpPosition[]; loading?: boolean; error?: string | null }>();
</script>

<template>
    <div class="min-h-0 grow overflow-auto rounded-md border border-neutral-800">
        <table class="w-full text-sm">
            <thead class="sticky top-0 bg-neutral-900/60 text-neutral-400">
                <tr>
                    <th class="px-3 py-2 text-left font-medium">Symbol</th>
                    <th class="px-3 py-2 text-left font-medium">Side</th>
                    <th class="px-3 py-2 text-left font-medium">Size</th>
                    <th class="px-3 py-2 text-left font-medium">Price</th>
                    <th class="px-3 py-2 text-left font-medium">Extra</th>
                </tr>
            </thead>
            <tbody>
                <tr v-if="loading">
                    <td colspan="5" class="px-3 py-3 text-neutral-400">Loading positions…</td>
                </tr>
                <tr v-else-if="error">
                    <td colspan="5" class="px-3 py-3 text-rose-400">{{ error }}</td>
                </tr>
                <tr v-for="p in positions" :key="p.symbol" class="border-t border-neutral-900">
                    <td class="px-3 py-2">{{ p.symbol }}</td>
                    <td class="px-3 py-2" :class="p.side === 'bid' ? 'text-emerald-400' : 'text-rose-400'">
                        {{ p.side }}
                    </td>
                    <td class="px-3 py-2 tabular-nums">{{ p.size }}</td>
                    <td class="px-3 py-2 tabular-nums">Entry {{ p.entry.toLocaleString() }}</td>
                    <td class="px-3 py-2 tabular-nums">PnL {{ p.pnl.toFixed(2) }}</td>
                </tr>
            </tbody>
        </table>
    </div>
</template>

<style scoped>
.tabular-nums {
    font-variant-numeric: tabular-nums;
}
</style>
