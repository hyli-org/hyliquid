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
const emit = defineEmits<{
    (event: "withdraw", balance: Balance): void;
    (event: "withdraw-on-eth", balance: Balance): void;
}>();

const handleWithdrawClick = (balance: Balance) => {
    emit("withdraw", balance);
};

const handleWithdrawOnEthClick = (balance: Balance) => {
    emit("withdraw-on-eth", balance);
};
</script>

<template>
    <div class="min-h-0 grow overflow-auto">
        <table class="w-full text-sm">
            <thead class="sticky top-0 bg-[var(--surface-header)] text-[var(--text-muted)]">
                <tr>
                    <th class="px-3 py-2 text-left font-medium">Asset</th>
                    <th class="px-3 py-2 text-right font-medium">Available</th>
                    <th class="px-3 py-2 text-right font-medium">Locked</th>
                    <th class="px-3 py-2 text-right font-medium">Total</th>
                </tr>
            </thead>
            <tbody>
                <tr v-if="props.loading">
                    <td colspan="4" class="px-3 py-3 text-[var(--text-muted)]">Loading balances…</td>
                </tr>
                <tr v-else-if="props.error">
                    <td colspan="4" class="px-3 py-3 text-[var(--sell-color)]">Error: {{ props.error.message }}</td>
                </tr>
                <tr v-else-if="props.balances.length === 0">
                    <td colspan="4" class="px-3 py-3 text-[var(--text-muted)]">No balances found</td>
                </tr>
                <tr
                    v-for="balance in props.balances"
                    :key="balance.asset"
                    class="border-t border-[var(--table-row-border)]"
                >
                    <td class="px-3 py-2 font-medium text-[var(--text-primary)]">
                        <div class="flex items-center gap-2">
                            <span>{{ balance.asset }}</span>
                            <button
                                type="button"
                                class="rounded border border-[var(--border-accent)] px-2 py-1 text-xs font-semibold text-[var(--text-accent)] transition hover:border-[var(--accent)] hover:text-[var(--text-primary)]"
                                @click="handleWithdrawClick(balance)"
                            >
                                withdraw
                            </button>
                            <button
                                v-if="balance.asset === 'ORANJ'"
                                type="button"
                                class="rounded bg-[var(--buy-color)] px-2 py-1 text-xs font-semibold text-[var(--text-on-accent)] transition hover:bg-[var(--buy-strong)]"
                                @click="handleWithdrawOnEthClick(balance)"
                            >
                                withdraw on eth
                            </button>
                        </div>
                    </td>
                    <td class="px-3 py-2 text-right tabular-nums text-[var(--text-secondary)]">
                        {{ assetsState.toRealQty(balance.asset, balance.available).toLocaleString() }}
                    </td>
                    <td class="px-3 py-2 text-right tabular-nums text-[var(--text-secondary)]">
                        {{ assetsState.toRealQty(balance.asset, balance.locked).toLocaleString() }}
                    </td>
                    <td class="px-3 py-2 text-right tabular-nums text-[var(--text-primary)]">
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
