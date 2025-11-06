<script setup lang="ts">
import { computed } from "vue";
import { activityState, assetsState, instrumentsState, submitOrder, useOrderFormState } from "../trade";

const { price, size, side, orderType, orderSubmit } = useOrderFormState();
const baseSymbol = computed(() => (instrumentsState.selected ? instrumentsState.selected.base_asset : "")!);
const quoteSymbol = computed(() => (instrumentsState.selected ? instrumentsState.selected.quote_asset : "")!);

const consumedSymbol = computed(() => (side.value === "ask" ? baseSymbol.value : quoteSymbol.value));

const neededBalance = computed(() => {
    if (side.value === "bid") {
        return (size.value ?? 0) * (price.value ?? 0);
    } else {
        return size.value ?? 0;
    }
});

const availableBalance = computed(() => {
    if (side.value === "ask") {
        return activityState.balances.find((b) => b.asset === baseSymbol.value)?.available ?? 0;
    } else {
        return activityState.balances.find((b) => b.asset === quoteSymbol.value)?.available ?? 0;
    }
});
</script>

<template>
    <section class="col-span-2 p-3">
        <div class="mb-3 flex gap-2">
            <button
                class="w-full rounded-md px-3 py-2 text-sm transition"
                :class="
                    side === 'bid'
                        ? 'border border-[var(--buy-border)] bg-[var(--buy-soft)] text-[var(--buy-color)] shadow-sm'
                        : 'border border-[var(--border-default)] bg-[var(--surface-input)] text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
                "
                @click="() => (side = 'bid')"
            >
                Buy
            </button>
            <button
                class="w-full rounded-md px-3 py-2 text-sm transition"
                :class="
                    side === 'ask'
                        ? 'border border-[var(--sell-border)] bg-[var(--sell-soft)] text-[var(--sell-color)] shadow-sm'
                        : 'border border-[var(--border-default)] bg-[var(--surface-input)] text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
                "
                @click="() => (side = 'ask')"
            >
                Sell
            </button>
        </div>

        <div class="mb-3">
            <div class="mb-1 text-xs font-semibold uppercase tracking-wide text-[var(--text-muted)]">Order Type</div>
            <div class="grid grid-cols-2 gap-2">
                <button
                    class="rounded-md px-3 py-2 text-sm transition"
                    :class="
                        orderType === 'limit'
                            ? 'border border-[var(--border-accent)] bg-[var(--accent-soft)] text-[var(--text-accent)] shadow-sm'
                            : 'border border-[var(--border-default)] bg-[var(--surface-input)] text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
                    "
                    @click="() => (orderType = 'limit')"
                >
                    Limit
                </button>
                <button
                    class="rounded-md px-3 py-2 text-sm transition"
                    :class="
                        orderType === 'market'
                            ? 'border border-[var(--border-accent)] bg-[var(--accent-soft)] text-[var(--text-accent)] shadow-sm'
                            : 'border border-[var(--border-default)] bg-[var(--surface-input)] text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
                    "
                    @click="() => (orderType = 'market')"
                >
                    Market
                </button>
            </div>
        </div>

        <div v-if="orderType === 'limit'" class="mb-3">
            <div class="mb-1 text-xs text-[var(--text-muted)]">Price</div>
            <input
                :value="price ?? ''"
                @input="(e: any) => (price = e.target.value === '' ? null : Number(e.target.value))"
                type="number"
                step="0.01"
                class="w-full rounded-md border border-[var(--border-default)] bg-[var(--surface-input)] px-3 py-2 text-sm text-[var(--text-primary)] outline-none transition focus:border-[var(--accent)] focus:ring-1 focus:ring-[var(--accent-soft)]"
            />
        </div>

        <div class="mb-3">
            <div class="mb-1 text-xs text-[var(--text-muted)]">Quantity ({{ baseSymbol }})</div>
            <input
                :value="size ?? ''"
                @input="(e: any) => (size = e.target.value === '' ? null : Number(e.target.value))"
                type="number"
                class="w-full rounded-md border border-[var(--border-default)] bg-[var(--surface-input)] px-3 py-2 text-sm text-[var(--text-primary)] outline-none transition focus:border-[var(--accent)] focus:ring-1 focus:ring-[var(--accent-soft)]"
            />
        </div>
        <div class="mb-3">
            <div class="text-xs text-[var(--text-muted)]">
                <div class="mb-1">Needed balance</div>
                <div class="text-sm text-[var(--text-secondary)]">{{ neededBalance }} {{ consumedSymbol }}</div>
            </div>
        </div>
        <div class="mb-3">
            <div class="mb-1 text-xs text-[var(--text-muted)]">Available Balance</div>
            <div class="text-sm text-[var(--text-secondary)]">
                {{ assetsState.toRealQty(consumedSymbol, availableBalance) }}
                {{ consumedSymbol }}<br />
            </div>
        </div>

        <button
            class="mb-2 w-full rounded-md px-3 py-2 text-sm font-medium disabled:opacity-60 disabled:cursor-not-allowed"
            :class="
                side === 'bid'
                    ? 'bg-[var(--buy-color)] hover:bg-[var(--buy-strong)] text-[var(--text-on-accent)]'
                    : 'bg-[var(--sell-color)] hover:bg-[var(--sell-strong)] text-[var(--text-on-accent)]'
            "
            :disabled="orderSubmit?.fetching"
            @click="submitOrder()"
        >
            {{ side === "bid" ? "Buy" : "Sell" }} {{ orderType }}
        </button>
        <div v-if="orderSubmit?.error" class="mb-2 text-xs text-[var(--sell-color)]">
            {{ orderSubmit.error }}
        </div>
        <div v-if="orderSubmit?.data" class="mb-2 text-xs text-[var(--buy-color)]">
            Order submitted successfully
            <a
                class="underline"
                :href="`https://explorer.hyli.org/tx/${orderSubmit.data.tx_hash}?network=localhost`"
                target="_blank"
            >
                See tx on explorer
            </a>
        </div>
    </section>
</template>

<style scoped>
.tabular-nums {
    font-variant-numeric: tabular-nums;
}
</style>
