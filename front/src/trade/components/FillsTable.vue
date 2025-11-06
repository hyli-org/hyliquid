<script setup lang="ts">
import type { Fill } from "../trade";
import { instrumentsState } from "../trade";
defineProps<{ fills: Fill[]; loading?: boolean; error?: string | null }>();
</script>

<template>
    <div class="min-h-0 grow overflow-auto">
        <table class="w-full text-sm">
            <thead class="sticky top-0 bg-[var(--surface-header)] text-[var(--text-muted)]">
                <tr>
                    <th class="px-3 py-2 text-left font-medium">Symbol</th>
                    <th class="px-3 py-2 text-left font-medium">Side</th>
                    <th class="px-3 py-2 text-left font-medium">Size</th>
                    <th class="px-3 py-2 text-left font-medium">Price</th>
                    <th class="px-3 py-2 text-left font-medium">Time</th>
                </tr>
            </thead>
            <tbody>
                <tr v-if="loading">
                    <td colspan="5" class="px-3 py-3 text-[var(--text-muted)]">Loading fills…</td>
                </tr>
                <tr v-else-if="error">
                    <td colspan="5" class="px-3 py-3 text-[var(--sell-color)]">{{ error }}</td>
                </tr>
                <tr v-for="f in fills" :key="f.symbol + f.time" class="border-t border-[var(--table-row-border)]">
                    <td class="px-3 py-2 text-[var(--text-primary)]">{{ f.symbol }}</td>
                    <td
                        class="px-3 py-2"
                        :class="f.side === 'bid' ? 'text-[var(--buy-color)]' : 'text-[var(--sell-color)]'"
                    >
                        {{ f.side }}
                    </td>
                    <td class="px-3 py-2 tabular-nums text-[var(--text-secondary)]">
                        {{ instrumentsState.toRealQty(f.symbol, f.size) }}
                    </td>
                    <td class="px-3 py-2 tabular-nums text-[var(--text-secondary)]">
                        {{ instrumentsState.toRealPrice(f.symbol, f.price) }}
                    </td>
                    <td class="px-3 py-2 text-[var(--text-secondary)]">{{ f.time }}</td>
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
