<script setup lang="ts">
interface Balance {
    asset: string;
    free: number;
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
    <div class="flex h-full flex-col">
        <div class="flex-1 overflow-auto">
            <div v-if="props.loading" class="flex items-center justify-center p-8">
                <div class="text-neutral-400">Loading balances...</div>
            </div>
            
            <div v-else-if="props.error" class="flex items-center justify-center p-8">
                <div class="text-red-400">Error: {{ props.error.message }}</div>
            </div>
            
            <div v-else-if="props.balances.length === 0" class="flex items-center justify-center p-8">
                <div class="text-neutral-400">No balances found</div>
            </div>
            
            <div v-else class="space-y-1">
                <div class="grid grid-cols-4 gap-4 px-4 py-2 text-xs font-medium text-neutral-400">
                    <div>Asset</div>
                    <div class="text-right">Free</div>
                    <div class="text-right">Locked</div>
                    <div class="text-right">Total</div>
                </div>
                
                <div
                    v-for="balance in props.balances"
                    :key="balance.asset"
                    class="grid grid-cols-4 gap-4 px-4 py-2 text-sm hover:bg-neutral-900"
                >
                    <div class="font-medium text-white">{{ balance.asset }}</div>
                    <div class="text-right tabular-nums text-neutral-300">
                        {{ balance.free.toFixed(8) }}
                    </div>
                    <div class="text-right tabular-nums text-neutral-300">
                        {{ balance.locked.toFixed(8) }}
                    </div>
                    <div class="text-right tabular-nums text-white">
                        {{ balance.total.toFixed(8) }}
                    </div>
                </div>
            </div>
        </div>
    </div>
</template>

<style scoped>
.tabular-nums {
    font-variant-numeric: tabular-nums;
}
</style>
