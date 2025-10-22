<script setup lang="ts">
import { computed, ref, watch } from "vue";
import type { Balance } from "../trade/trade";
import { assetsState } from "../trade/trade";
import { useHyliWithdraw, toScaledAmount } from "../withdraw/useHyliWithdraw";
import { useWallet } from "hyli-wallet-vue";

const props = defineProps<{
    isOpen: boolean;
    balance: Balance | null;
}>();

const emit = defineEmits<{
    (event: "update:isOpen", value: boolean): void;
}>();

const { wallet } = useWallet();

const amountInput = ref<string>("");

const { submitWithdraw, isSubmitting, errorMessage, successMessage, clearStatus } = useHyliWithdraw();

const assetInfo = computed(() => {
    const symbol = props.balance?.asset;
    if (!symbol) return null;
    return assetsState.list.find((asset) => asset.symbol === symbol) ?? null;
});

const availableReal = computed(() => {
    const asset = assetInfo.value;
    const balance = props.balance;
    if (!asset || !balance) return 0;
    return balance.available / 10 ** asset.scale;
});

const availableFormatted = computed(() => availableReal.value.toLocaleString());

const amountNumber = computed(() => {
    const value = parseFloat(amountInput.value);
    return Number.isFinite(value) ? value : null;
});

const amountError = computed(() => {
    if (!props.balance) {
        return "No balance selected";
    }
    const asset = assetInfo.value;
    const value = amountNumber.value;
    if (!asset) {
        return `Unknown asset ${props.balance.asset}`;
    }
    if (value === null) {
        return "";
    }
    if (value === 0) {
        return "Amount cannot be zero";
    }
    if (value < 0) {
        return "Amount cannot be negative";
    }
    try {
        const scaled = toScaledAmount(value, asset.scale);
        if (scaled > props.balance.available) {
            return "Amount must be less than the available balance";
        }
    } catch (error) {
        return error instanceof Error ? error.message : "Invalid amount";
    }
    return null;
});

const canSubmit = computed(() => {
    if (!props.balance) return false;
    const value = amountNumber.value;
    if (value === null || value <= 0) return false;

    const asset = assetInfo.value;
    if (!asset) return false;

    try {
        const scaled = toScaledAmount(value, asset.scale);
        if (scaled > props.balance.available) {
            return false;
        }
    } catch {
        return false;
    }

    return !isSubmitting.value;
});

watch(
    () => props.isOpen,
    (open) => {
        if (open) {
            amountInput.value = "";
            clearStatus();
        }
    },
);

watch(
    () => props.balance,
    () => {
        if (props.isOpen) {
            amountInput.value = "";
            clearStatus();
        }
    },
);

const closeModal = () => {
    emit("update:isOpen", false);
};

const handleWithdraw = async () => {
    if (!props.balance) return;
    const value = amountNumber.value;

    if (value === null || value === 0 || value < 0) return;
    if (!canSubmit.value) return;

    const addr = wallet.value?.address;
    if (!addr) return;

    const result = await submitWithdraw(props.balance.asset, value, {
        network: "hyli",
        address: addr,
    });
    if (result.success) {
        amountInput.value = "";
        // Close modal after 2 seconds
        setTimeout(() => {
            closeModal();
        }, 2000);
    }
};

const withdrawTitle = computed(() => {
    const symbol = props.balance?.asset ?? "";
    return symbol ? `Withdraw ${symbol}` : "Withdraw";
});
</script>

<template>
    <Teleport to="body">
        <div v-if="props.isOpen" class="fixed inset-0 z-40 flex items-center justify-center">
            <div class="absolute inset-0 bg-black/60" @click="closeModal"></div>
            <div
                class="relative z-10 w-full max-w-md rounded-lg border border-neutral-800 bg-neutral-900 p-6 shadow-xl"
            >
                <header class="mb-4 flex items-start justify-between">
                    <div>
                        <h2 class="text-lg font-semibold text-neutral-100">{{ withdrawTitle }}</h2>
                        <p v-if="props.balance" class="text-xs text-neutral-500">Available: {{ availableFormatted }}</p>
                    </div>
                    <button
                        type="button"
                        class="flex h-8 w-8 items-center justify-center rounded bg-neutral-800 text-neutral-400 hover:text-neutral-200"
                        @click="closeModal"
                    >
                        ✕
                    </button>
                </header>

                <div class="space-y-4">
                    <div>
                        <label class="flex items-center justify-between text-sm text-neutral-300">
                            Amount to withdraw
                            <span class="text-xs text-neutral-500">Tokens remain on Hyli</span>
                        </label>
                        <div class="relative mt-1">
                            <input
                                v-model="amountInput"
                                type="number"
                                min="0"
                                step="any"
                                placeholder="0.0"
                                class="w-full rounded border border-neutral-700 bg-neutral-800/60 p-2 pr-12 text-neutral-100 focus:border-emerald-500 focus:outline-none"
                            />
                            <button
                                type="button"
                                class="absolute right-2 top-1/2 -translate-y-1/2 rounded bg-neutral-700 px-2 py-1 text-xs font-medium text-neutral-300 transition hover:bg-neutral-600 hover:text-neutral-100"
                                @click="amountInput = availableReal.toString()"
                            >
                                Max
                            </button>
                        </div>
                        <p v-if="amountError" class="mt-1 text-xs text-rose-400">{{ amountError }}</p>
                    </div>

                    <div
                        v-if="errorMessage"
                        class="rounded border border-rose-500/40 bg-rose-500/10 px-3 py-2 text-sm text-rose-200"
                    >
                        {{ errorMessage }}
                    </div>
                    <div
                        v-if="successMessage"
                        class="rounded border border-emerald-500/40 bg-emerald-500/10 px-3 py-2 text-sm text-emerald-200"
                    >
                        {{ successMessage }}
                    </div>

                    <button
                        type="button"
                        class="w-full rounded bg-emerald-600 px-4 py-2 text-sm font-semibold text-neutral-100 transition hover:bg-emerald-500 disabled:cursor-not-allowed disabled:bg-neutral-700 disabled:text-neutral-400"
                        :disabled="!canSubmit"
                        @click="handleWithdraw"
                    >
                        <span v-if="isSubmitting">Submitting…</span>
                        <span v-else>Withdraw</span>
                    </button>
                </div>
            </div>
        </div>
    </Teleport>
</template>
