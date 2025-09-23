<script setup lang="ts">
import { computed } from "vue";
import { instrumentsState, submitOrder, useOrderFormState } from "../trade";

const { price, size, side, orderType, orderSubmit } = useOrderFormState();

const baseSymbol = computed(() => (instrumentsState.selected ? instrumentsState.selected.symbol.split("/")[0] : "")!);
</script>

<template>
    <section class="col-span-2 p-3">
        <div class="mb-3 flex gap-2">
            <button
                class="w-full rounded-md px-3 py-2 text-sm"
                :class="
                    side === 'Bid'
                        ? 'bg-emerald-500/20 text-emerald-300 border border-emerald-700'
                        : 'bg-neutral-900 text-neutral-300'
                "
                @click="() => (side = 'Bid')"
            >
                Bid
            </button>
            <button
                class="w-full rounded-md px-3 py-2 text-sm"
                :class="
                    side === 'Ask'
                        ? 'bg-rose-500/20 text-rose-300 border border-rose-700'
                        : 'bg-neutral-900 text-neutral-300'
                "
                @click="() => (side = 'Ask')"
            >
                Ask
            </button>
        </div>

        <div class="mb-3">
            <div class="mb-1 text-xs text-neutral-400">Order Type</div>
            <div class="grid grid-cols-2 gap-2">
                <button
                    class="rounded-md px-3 py-2 text-sm"
                    :class="orderType === 'Limit' ? 'bg-neutral-800 text-white' : 'bg-neutral-900 text-neutral-300'"
                    @click="() => (orderType = 'Limit')"
                >
                    Limit
                </button>
                <button
                    class="rounded-md px-3 py-2 text-sm"
                    :class="orderType === 'Market' ? 'bg-neutral-800 text-white' : 'bg-neutral-900 text-neutral-300'"
                    @click="() => (orderType = 'Market')"
                >
                    Market
                </button>
            </div>
        </div>

        <div v-if="orderType === 'Limit'" class="mb-3">
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
            <div class="mb-1 text-xs text-neutral-400">Size ({{ baseSymbol }})</div>
            <input
                :value="size ?? ''"
                @input="(e: any) => (size = e.target.value === '' ? null : Number(e.target.value))"
                type="number"
                step="0.0001"
                class="w-full rounded-md bg-neutral-900 px-3 py-2 text-sm outline-none focus:ring-1 focus:ring-neutral-700"
            />
        </div>

        <button
            class="mb-2 w-full rounded-md px-3 py-2 text-sm font-medium disabled:opacity-60 disabled:cursor-not-allowed"
            :class="
                side === 'Bid'
                    ? 'bg-emerald-600 hover:bg-emerald-500 text-white'
                    : 'bg-rose-600 hover:bg-rose-500 text-white'
            "
            :disabled="orderSubmit?.fetching"
            @click="submitOrder()"
        >
            {{ side }} {{ orderType }}
        </button>
        <div v-if="orderSubmit?.error" class="mb-2 text-xs text-rose-400">
            {{ orderSubmit.error }}
        </div>
    </section>
</template>

<style scoped>
.tabular-nums {
    font-variant-numeric: tabular-nums;
}
</style>
