<script setup lang="ts">
import { computed, ref } from "vue";
import { activityState, assetsState, instrumentsState, submitOrder, useOrderFormState } from "../trade";
import { v7 as uuidv7 } from "uuid";

const { price, size, side, orderType, orderSubmit } = useOrderFormState();
const debug = ref(false);
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
                class="w-full rounded-md px-3 py-2 text-sm"
                :class="
                    side === 'bid'
                        ? 'bg-emerald-500/20 text-emerald-300 border border-emerald-700'
                        : 'bg-neutral-900 text-neutral-300'
                "
                @click="() => (side = 'bid')"
            >
                Buy
            </button>
            <button
                class="w-full rounded-md px-3 py-2 text-sm"
                :class="
                    side === 'ask'
                        ? 'bg-rose-500/20 text-rose-300 border border-rose-700'
                        : 'bg-neutral-900 text-neutral-300'
                "
                @click="() => (side = 'ask')"
            >
                Sell
            </button>
        </div>

        <div class="mb-3">
            <div class="mb-1 text-xs text-neutral-400">Order Type</div>
            <div class="grid grid-cols-2 gap-2">
                <button
                    class="rounded-md px-3 py-2 text-sm"
                    :class="orderType === 'limit' ? 'bg-neutral-800 text-white' : 'bg-neutral-900 text-neutral-300'"
                    @click="() => (orderType = 'limit')"
                >
                    Limit
                </button>
                <button
                    class="rounded-md px-3 py-2 text-sm"
                    :class="orderType === 'market' ? 'bg-neutral-800 text-white' : 'bg-neutral-900 text-neutral-300'"
                    @click="() => (orderType = 'market')"
                >
                    Market
                </button>
            </div>
        </div>

        <div v-if="orderType === 'limit'" class="mb-3">
            <div class="mb-1 text-xs text-neutral-400">Price</div>
            <input
                :value="price ?? ''"
                @input="(e: any) => (price = e.target.value === '' ? null : Number(e.target.value))"
                type="number"
                step="0.01"
                class="w-full rounded-md bg-neutral-900 px-3 py-2 text-sm outline-none focus:ring-1 focus:ring-neutral-700"
            />
        </div>

        <div class="mb-3">
            <div class="mb-1 text-xs text-neutral-400">Quantity ({{ baseSymbol }})</div>
            <input
                :value="size ?? ''"
                @input="(e: any) => (size = e.target.value === '' ? null : Number(e.target.value))"
                type="number"
                class="w-full rounded-md bg-neutral-900 px-3 py-2 text-sm outline-none focus:ring-1 focus:ring-neutral-700"
            />
        </div>
        <div class="mb-3">
            <div class="text-xs text-neutral-400">
                <div class="mb-1">Needed balance</div>
                <div class="text-sm text-neutral-400">{{ neededBalance }} {{ consumedSymbol }}</div>
            </div>
        </div>
        <div class="mb-3">
            <div class="mb-1 text-xs text-neutral-400">Available Balance</div>
            <div class="text-sm text-neutral-400">
                {{ assetsState.toRealQty(consumedSymbol, availableBalance) }}
                {{ consumedSymbol }}<br />
                integer: {{ availableBalance.toLocaleString() }}
            </div>
        </div>

        <button
            class="mb-2 w-full rounded-md px-3 py-2 text-sm font-medium disabled:opacity-60 disabled:cursor-not-allowed"
            :class="
                side === 'bid'
                    ? 'bg-emerald-600 hover:bg-emerald-500 text-white'
                    : 'bg-rose-600 hover:bg-rose-500 text-white'
            "
            :disabled="orderSubmit?.fetching"
            @click="submitOrder()"
        >
            {{ side === "bid" ? "Buy" : "Sell" }} {{ orderType }}
        </button>
        <div v-if="orderSubmit?.error" class="mb-2 text-xs text-rose-400">
            {{ orderSubmit.error }}
        </div>
        <div class="mb-2">
            <button class="text-xs text-neutral-400" @click="debug = !debug">
                {{ debug ? "Hide debug" : "Show debug" }}
            </button>
        </div>
        <div v-show="debug" class="mb-2 text-xs text-neutral-400">
            Side: {{ side }}<br />
            Order type: {{ orderType }}<br />
            Integer price: {{ instrumentsState.toIntPrice(instrumentsState.selected?.symbol, price ?? 0) }}<br />
            Integer quantity: {{ instrumentsState.toIntQty(instrumentsState.selected?.symbol, size ?? 0) }}<br />

            <code>
                create-order --order-id {{ uuidv7() }} --order-side {{ side.toLowerCase() }} --order-type
                {{ orderType.toLowerCase() }} --contract-name1 {{ baseSymbol }} --contract-name2
                {{ quoteSymbol }} --quantity
                {{ instrumentsState.toIntQty(instrumentsState.selected?.symbol, size ?? 0) }} --price
                {{ instrumentsState.toIntPrice(instrumentsState.selected?.symbol, price ?? 0) }}
            </code>
        </div>
        <div v-if="orderSubmit?.data" class="mb-2 text-xs text-emerald-400">
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
