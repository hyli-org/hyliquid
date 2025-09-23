<script setup lang="ts">
import type { Order } from "../trade";
defineProps<{ orders: Order[]; loading?: boolean; error?: string | null }>();
</script>

<template>
    <div class="min-h-0 grow overflow-auto rounded-md border border-neutral-800">
        <table class="w-full text-sm">
            <thead class="sticky top-0 bg-neutral-900/60 text-neutral-400">
                <tr>
                    <th class="px-3 py-2 text-left font-medium">Symbol</th>
                    <th class="px-3 py-2 text-left font-medium">Side</th>
                    <th class="px-3 py-2 text-left font-medium">Qty</th>
                    <th class="px-3 py-2 text-left font-medium">Qty Remaining</th>
                    <th class="px-3 py-2 text-left font-medium">Price</th>
                    <th class="px-3 py-2 text-left font-medium">Status</th>
                </tr>
            </thead>
            <tbody>
                <tr v-if="loading">
                    <td colspan="5" class="px-3 py-3 text-neutral-400">Loading orders…</td>
                </tr>
                <tr v-else-if="error">
                    <td colspan="5" class="px-3 py-3 text-rose-400">{{ error }}</td>
                </tr>
                <tr v-for="o in orders" :key="o.symbol + o.price + o.type" class="border-t border-neutral-900">
                    <td class="px-3 py-2">{{ o.symbol }}</td>
                    <td class="px-3 py-2" :class="o.side === 'Bid' ? 'text-emerald-400' : 'text-rose-400'">
                        {{ o.side }}
                    </td>
                    <td class="px-3 py-2 tabular-nums">{{ o.qty }}</td>
                    <td class="px-3 py-2 tabular-nums">{{ o.qty_remaining }}</td>
                    <td class="px-3 py-2 tabular-nums">{{ o.type }} @ {{ o.price }}</td>
                    <td class="px-3 py-2">{{ o.status }}</td>
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
