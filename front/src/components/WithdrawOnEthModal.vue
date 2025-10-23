<script setup lang="ts">
import { computed, ref, watch } from "vue";
import type { Balance } from "../trade/trade";
import { assetsState } from "../trade/trade";
import { useHyliWithdraw, toScaledAmount } from "../withdraw/useHyliWithdraw";
import { useEthereumBridge } from "../deposit/useEthereumBridge";

const props = defineProps<{
    isOpen: boolean;
    balance: Balance | null;
}>();

const emit = defineEmits<{
    (event: "update:isOpen", value: boolean): void;
}>();

const amountInput = ref<string>("");

const { submitWithdraw, isSubmitting, errorMessage, successMessage, clearStatus } = useHyliWithdraw();

const {
    associatedAddress,
    needsManualAssociation,
    needsBridgeClaim,
    bridgeClaimed,
    requestManualAssociation,
    refreshAssociation,
    loadingAssociation,
    associationError,
    submittingAssociation,
    submitError,
    claimStatusLoading,
    claimStatusError,
    providerAvailable,
    manualAssociation,
    currentNetwork,
    isWrongNetwork,
    switchNetworkError,
    isSwitchingNetwork,
    switchToSepoliaNetwork,
    checkNetworkMatch,
    setupNetworkListener,
} = useEthereumBridge();

let networkListenerCleanup: (() => void) | undefined;

watch(
    () => props.isOpen,
    async (open) => {
        if (open) {
            amountInput.value = "";
            clearStatus();
            await refreshAssociation();
            await checkNetworkMatch();
            networkListenerCleanup = setupNetworkListener();
        } else {
            if (networkListenerCleanup) {
                networkListenerCleanup();
                networkListenerCleanup = undefined;
            }
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

const scaledAmount = computed(() => {
    const value = amountNumber.value;
    const asset = assetInfo.value;
    const balance = props.balance;
    if (!asset || !balance || value === null || value <= 0) return null;
    try {
        const scaled = toScaledAmount(value, asset.scale);
        if (scaled > balance.available) {
            return null;
        }
        return scaled;
    } catch {
        return null;
    }
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

const canSubmitWithdraw = computed(() => {
    if (!props.balance) return false;
    if (!scaledAmount.value) return false;
    if (amountError.value) return false;
    if (needsManualAssociation.value || needsBridgeClaim.value) return false;
    if (!bridgeClaimed.value || !associatedAddress.value) return false;
    if (loadingAssociation.value || submittingAssociation.value || claimStatusLoading.value) return false;
    return !isSubmitting.value;
});

const closeModal = () => {
    emit("update:isOpen", false);
};

const handleWithdraw = async () => {
    if (!props.balance) return;
    const amount = amountNumber.value;

    if (amount === null || amount === 0 || amount < 0) return;
    if (!canSubmitWithdraw.value) return;

    const address = associatedAddress.value;
    if (!address) {
        return;
    }

    const result = await submitWithdraw(props.balance.asset, amount, {
        network: "ethereum-sepolia",
        address,
    });
    if (result.success) {
        amountInput.value = "";
        // Close modal after 2 seconds
        setTimeout(() => {
            closeModal();
        }, 2000);
    }
};

const handleClaimAddress = async () => {
    try {
        await requestManualAssociation();
        await refreshAssociation();
    } catch {
        // Error messages are handled by the hook
    }
};

const withdrawTitle = computed(() => {
    const symbol = props.balance?.asset ?? "";
    return symbol ? `Withdraw ${symbol} on Ethereum` : "Withdraw on Ethereum";
});
</script>

<template>
    <Teleport to="body">
        <div v-if="props.isOpen" class="fixed inset-0 z-50 flex items-center justify-center">
            <div class="absolute inset-0 bg-black/60" @click="closeModal"></div>
            <div
                class="relative z-10 w-full max-w-lg rounded-lg border border-neutral-800 bg-neutral-900 p-6 shadow-xl"
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
                            <span class="text-xs text-neutral-500"
                                >Balance settles to your associated Ethereum address</span
                            >
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

                    <section class="space-y-3 rounded border border-neutral-800 bg-neutral-800/40 p-3 text-sm">
                        <header class="flex items-center justify-between">
                            <span class="text-neutral-300">Ethereum association</span>
                            <button
                                type="button"
                                class="text-xs text-cyan-400 hover:text-cyan-200"
                                :disabled="loadingAssociation"
                                @click="refreshAssociation"
                            >
                                <span v-if="loadingAssociation">Refreshing…</span>
                                <span v-else>Refresh</span>
                            </button>
                        </header>

                        <div class="space-y-2">
                            <div class="space-y-1 text-xs text-neutral-500">
                                <p class="flex items-center justify-between">
                                    Target network
                                    <span class="font-mono text-neutral-300">{{ currentNetwork.name }}</span>
                                </p>
                            </div>
                            <button
                                v-if="isWrongNetwork"
                                type="button"
                                class="w-full rounded border border-cyan-600 px-2 py-1 text-xs font-semibold text-cyan-200 transition hover:bg-cyan-600/20 disabled:cursor-not-allowed disabled:border-neutral-700 disabled:text-neutral-500"
                                :disabled="isSwitchingNetwork"
                                @click="switchToSepoliaNetwork"
                            >
                                <span v-if="isSwitchingNetwork">Switching network…</span>
                                <span v-else>Switch to {{ currentNetwork.name }} in wallet</span>
                            </button>
                            <p v-if="switchNetworkError" class="text-xs text-rose-400">
                                {{ switchNetworkError }}
                            </p>
                        </div>

                        <div>
                            <p v-if="associatedAddress" class="text-xs text-neutral-400">
                                Linked address:
                                <span class="font-mono text-neutral-200">{{ associatedAddress }}</span>
                            </p>
                            <p v-else class="text-xs text-amber-400">No Ethereum address linked yet.</p>
                        </div>

                        <p v-if="claimStatusLoading" class="text-xs text-neutral-500">Checking bridge claim status…</p>
                        <p v-else-if="needsBridgeClaim" class="text-xs text-amber-400">
                            This identity is not claimed yet. Claim it before withdrawing.
                        </p>
                        <p v-else-if="needsManualAssociation" class="text-xs text-amber-400">
                            Link an Ethereum address to withdraw this balance.
                        </p>
                        <p v-else class="text-xs text-emerald-400">Bridge claim registered.</p>

                        <div
                            v-if="associationError"
                            class="rounded border border-rose-500/40 bg-rose-500/10 px-3 py-2 text-xs text-rose-200"
                        >
                            {{ associationError }}
                        </div>
                        <div
                            v-if="submitError"
                            class="rounded border border-rose-500/40 bg-rose-500/10 px-3 py-2 text-xs text-rose-200"
                        >
                            {{ submitError }}
                        </div>
                        <div
                            v-if="claimStatusError"
                            class="rounded border border-rose-500/40 bg-rose-500/10 px-3 py-2 text-xs text-rose-200"
                        >
                            {{ claimStatusError }}
                        </div>

                        <div
                            v-if="manualAssociation"
                            class="rounded border border-emerald-500/40 bg-emerald-500/10 px-3 py-2 text-xs text-emerald-200"
                        >
                            Claim signature recorded. Keep it for your records.
                        </div>

                        <div
                            v-if="!providerAvailable"
                            class="rounded border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-xs text-amber-200"
                        >
                            Connect an Ethereum wallet (e.g. MetaMask) to manage the bridge claim.
                        </div>

                        <button
                            v-if="providerAvailable && needsBridgeClaim"
                            type="button"
                            class="w-full rounded border border-cyan-600 px-3 py-2 text-xs font-semibold text-cyan-200 transition hover:bg-cyan-600/20 disabled:cursor-not-allowed disabled:border-neutral-700 disabled:text-neutral-500"
                            :disabled="submittingAssociation || claimStatusLoading"
                            @click="handleClaimAddress"
                        >
                            <span v-if="submittingAssociation">Awaiting signature…</span>
                            <span v-else>Claim Ethereum address</span>
                        </button>
                        <p v-else-if="needsBridgeClaim" class="text-xs text-neutral-500">
                            Connect an Ethereum wallet to claim this identity.
                        </p>
                    </section>

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
                        :disabled="!canSubmitWithdraw"
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
