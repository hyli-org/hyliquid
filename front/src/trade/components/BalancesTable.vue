<script setup lang="ts">
import { assetsState } from "../trade";

interface Balance {
    asset: string;
    available: number;
    locked: number;
    total: number;
}

interface Props {
    balances: Balance[];
    loading: boolean;
    error: Error | null;
}

const props = defineProps<Props>();
</script>

<template>
    <div class="min-h-0 grow overflow-auto rounded-md border border-neutral-800">
        <table class="w-full text-sm">
            <thead class="sticky top-0 bg-neutral-900/60 text-neutral-400">
                <tr>
                    <th class="px-3 py-2 text-left font-medium">Asset</th>
                    <th class="px-3 py-2 text-right font-medium">Available</th>
                    <th class="px-3 py-2 text-right font-medium">Locked</th>
                    <th class="px-3 py-2 text-right font-medium">Total</th>
                </tr>
            </thead>
            <tbody>
                <tr v-if="props.loading">
                    <td colspan="4" class="px-3 py-3 text-neutral-400">Loading balances…</td>
                </tr>
                <tr v-else-if="props.error">
                    <td colspan="4" class="px-3 py-3 text-rose-400">Error: {{ props.error.message }}</td>
                </tr>
                <tr v-else-if="props.balances.length === 0">
                    <td colspan="4" class="px-3 py-3 text-neutral-400">No balances found</td>
                </tr>
                <tr v-for="balance in props.balances" :key="balance.asset" class="border-t border-neutral-900">
                    <td class="px-3 py-2 font-medium text-white">{{ balance.asset }}</td>
                    <td class="px-3 py-2 text-right tabular-nums text-neutral-300">
                        {{ assetsState.toRealQty(balance.asset, balance.available).toLocaleString() }}
                    </td>
                    <td class="px-3 py-2 text-right tabular-nums text-neutral-300">
                        {{ assetsState.toRealQty(balance.asset, balance.locked).toLocaleString() }}
                    </td>
                    <td class="px-3 py-2 text-right tabular-nums text-white">
                        {{ assetsState.toRealQty(balance.asset, balance.total).toLocaleString() }}
                    </td>
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
