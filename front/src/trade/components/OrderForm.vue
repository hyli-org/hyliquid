<script setup lang="ts">
import { computed } from "vue";
import type { OrderType, Side } from "../types";

const props = defineProps<{
    side: Side;
    orderType: OrderType;
    price: number | null;
    size: number | null;
    leverage: number;
    baseSymbol: string; // e.g., BTC from BTC-PERP
}>();

const emit = defineEmits<{
    (e: "update:side", v: Side): void;
    (e: "update:orderType", v: OrderType): void;
    (e: "update:price", v: number | null): void;
    (e: "update:size", v: number | null): void;
    (e: "submit"): void;
}>();

const notional = computed(() => (props.size ?? 0) * (props.price ?? 0));
const estInitialMargin = computed(() => (notional.value ? notional.value / props.leverage : 0));
</script>

<template>
    <section class="col-span-2 p-3">
        <div class="mb-3 flex gap-2">
            <button
                class="w-full rounded-md px-3 py-2 text-sm"
                :class="
                    props.side === 'Long'
                        ? 'bg-emerald-500/20 text-emerald-300 border border-emerald-700'
                        : 'bg-neutral-900 text-neutral-300'
                "
                @click="emit('update:side', 'Long')"
            >
                Long
            </button>
            <button
                class="w-full rounded-md px-3 py-2 text-sm"
                :class="
                    props.side === 'Short'
                        ? 'bg-rose-500/20 text-rose-300 border border-rose-700'
                        : 'bg-neutral-900 text-neutral-300'
                "
                @click="emit('update:side', 'Short')"
            >
                Short
            </button>
        </div>

        <div class="mb-3">
            <div class="mb-1 text-xs text-neutral-400">Order Type</div>
            <div class="grid grid-cols-2 gap-2">
                <button
                    class="rounded-md px-3 py-2 text-sm"
                    :class="
                        props.orderType === 'Limit' ? 'bg-neutral-800 text-white' : 'bg-neutral-900 text-neutral-300'
                    "
                    @click="emit('update:orderType', 'Limit')"
                >
                    Limit
                </button>
                <button
                    class="rounded-md px-3 py-2 text-sm"
                    :class="
                        props.orderType === 'Market' ? 'bg-neutral-800 text-white' : 'bg-neutral-900 text-neutral-300'
                    "
                    @click="emit('update:orderType', 'Market')"
                >
                    Market
                </button>
            </div>
        </div>

        <div v-if="props.orderType === 'Limit'" class="mb-3">
            <div class="mb-1 text-xs text-neutral-400">Price</div>
            <input
                :value="props.price ?? ''"
                @input="(e: any) => emit('update:price', e.target.value === '' ? null : Number(e.target.value))"
                type="number"
                step="0.01"
                class="w-full rounded-md bg-neutral-900 px-3 py-2 text-sm outline-none focus:ring-1 focus:ring-neutral-700"
            />
        </div>

        <div class="mb-3">
            <div class="mb-1 text-xs text-neutral-400">Size ({{ props.baseSymbol }})</div>
            <input
                :value="props.size ?? ''"
                @input="(e: any) => emit('update:size', e.target.value === '' ? null : Number(e.target.value))"
                type="number"
                step="0.0001"
                class="w-full rounded-md bg-neutral-900 px-3 py-2 text-sm outline-none focus:ring-1 focus:ring-neutral-700"
            />
        </div>

        <div class="mb-3 flex items-center justify-between text-sm text-neutral-400">
            <span>Notional</span>
            <span class="tabular-nums">${{ notional.toFixed(2) }}</span>
        </div>
        <div class="mb-4 flex items-center justify-between text-sm text-neutral-400">
            <span>Est. Initial Margin</span>
            <span class="tabular-nums">${{ estInitialMargin.toFixed(2) }}</span>
        </div>

        <button
            class="mb-2 w-full rounded-md px-3 py-2 text-sm font-medium"
            :class="
                props.side === 'Long'
                    ? 'bg-emerald-600 hover:bg-emerald-500 text-white'
                    : 'bg-rose-600 hover:bg-rose-500 text-white'
            "
            @click="emit('submit')"
        >
            {{ props.side }} {{ props.orderType }}
        </button>
        <div class="text-xs text-neutral-500">
            Leverage: <span class="tabular-nums">{{ props.leverage }}x</span>
        </div>
    </section>
</template>

<style scoped>
.tabular-nums {
    font-variant-numeric: tabular-nums;
}
</style>
