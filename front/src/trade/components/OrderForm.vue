<script setup lang="ts">
import { computed } from "vue";
import { orderFormState, type OrderType, type Side } from "../trade";

const props = defineProps<{
    baseSymbol: string; // e.g., BTC from BTC-PERP
}>();

const formState = computed(() => orderFormState);

const emit = defineEmits<{
    (e: "update:side", v: Side): void;
    (e: "update:orderType", v: OrderType): void;
    (e: "update:price", v: number | null): void;
    (e: "update:size", v: number | null): void;
    (e: "submit"): void;
}>();

const notional = computed(() => (formState.value.size ?? 0) * (formState.value.price ?? 0));
const estInitialMargin = computed(() => (notional.value ? notional.value / formState.value.leverage : 0));
</script>

<template>
    <section class="col-span-2 p-3">
        <div class="mb-3 flex gap-2">
            <button
                class="w-full rounded-md px-3 py-2 text-sm"
                :class="
                    formState.side === 'Long'
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
                    formState.side === 'Short'
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
                        formState.orderType === 'Limit'
                            ? 'bg-neutral-800 text-white'
                            : 'bg-neutral-900 text-neutral-300'
                    "
                    @click="emit('update:orderType', 'Limit')"
                >
                    Limit
                </button>
                <button
                    class="rounded-md px-3 py-2 text-sm"
                    :class="
                        formState.orderType === 'Market'
                            ? 'bg-neutral-800 text-white'
                            : 'bg-neutral-900 text-neutral-300'
                    "
                    @click="emit('update:orderType', 'Market')"
                >
                    Market
                </button>
            </div>
        </div>

        <div v-if="formState.orderType === 'Limit'" class="mb-3">
            <div class="mb-1 text-xs text-neutral-400">Price</div>
            <input
                :value="formState.price ?? ''"
                @input="(e: any) => emit('update:price', e.target.value === '' ? null : Number(e.target.value))"
                type="number"
                step="0.01"
                class="w-full rounded-md bg-neutral-900 px-3 py-2 text-sm outline-none focus:ring-1 focus:ring-neutral-700"
            />
        </div>

        <div class="mb-3">
            <div class="mb-1 text-xs text-neutral-400">Size ({{ props.baseSymbol }})</div>
            <input
                :value="formState.size ?? ''"
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
            class="mb-2 w-full rounded-md px-3 py-2 text-sm font-medium disabled:opacity-60 disabled:cursor-not-allowed"
            :class="
                formState.side === 'Long'
                    ? 'bg-emerald-600 hover:bg-emerald-500 text-white'
                    : 'bg-rose-600 hover:bg-rose-500 text-white'
            "
            :disabled="formState.orderSubmit?.fetching"
            @click="emit('submit')"
        >
            {{ formState.side }} {{ formState.orderType }}
        </button>
        <div v-if="formState.orderSubmit?.error" class="mb-2 text-xs text-rose-400">
            {{ formState.orderSubmit.error }}
        </div>
        <div class="text-xs text-neutral-500">
            Leverage: <span class="tabular-nums">{{ formState.leverage }}x</span>
        </div>
    </section>
</template>

<style scoped>
.tabular-nums {
    font-variant-numeric: tabular-nums;
}
</style>
