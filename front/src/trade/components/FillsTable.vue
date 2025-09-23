<script setup lang="ts">
import type { Fill } from "../types";
defineProps<{ fills: Fill[] }>();
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
                    <th class="px-3 py-2 text-left font-medium">Time</th>
                </tr>
            </thead>
            <tbody>
                <tr v-for="f in fills" :key="f.symbol + f.time" class="border-t border-neutral-900">
                    <td class="px-3 py-2">{{ f.symbol }}</td>
                    <td class="px-3 py-2" :class="f.side === 'Long' ? 'text-emerald-400' : 'text-rose-400'">
                        {{ f.side }}
                    </td>
                    <td class="px-3 py-2 tabular-nums">{{ f.size }}</td>
                    <td class="px-3 py-2 tabular-nums">@ {{ f.price }}</td>
                    <td class="px-3 py-2">{{ f.time }}</td>
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
