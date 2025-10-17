<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { assetsState, instrumentsState } from "../trade/trade";
import { useHyliDeposit } from "../deposit/useHyliDeposit";
import { useEthereumBridge } from "../deposit/useEthereumBridge";
import { useWallet } from "hyli-wallet-vue";

const props = defineProps<{
    isOpen: boolean;
}>();

const emit = defineEmits<{
    (event: "update:isOpen", value: boolean): void;
}>();

type DepositTab = "hyli" | "ethereum";

const activeTab = ref<DepositTab>("hyli");
const hyliAmount = ref<number>(0);
const hyliAsset = ref<string>("");
const tokenAmount = ref<number>(0);

const { wallet } = useWallet();

const {
    submitDeposit,
    isSubmitting: hyliSubmitting,
    errorMessage: hyliError,
    successMessage: hyliSuccess,
    clearStatus: clearHyliStatus,
} = useHyliDeposit();

const {
    loadingAssociation,
    associationError,
    needsManualAssociation,
    associatedAddress,
    submittingAssociation,
    submitError,
    providerAvailable,
    manualAssociation,
    txHash,
    depositError,
    isSendingTransaction,
    networkError,
    isSwitchingNetwork,
    availableNetworks,
    selectedNetwork,
    selectedNetworkId,
    setSelectedNetwork,
    needsBridgeClaim,
    hasBridgeIdentity,
    bridgeClaimed,
    claimStatusLoading,
    claimStatusError,
    refreshAssociation,
    requestManualAssociation,
    sendDepositTransaction,
} = useEthereumBridge();

const resetFormState = () => {
    const defaultAsset =
        instrumentsState.selected?.base_asset ??
        instrumentsState.selected?.quote_asset ??
        assetsState.list[0]?.symbol ??
        "";
    hyliAsset.value = defaultAsset;
    hyliAmount.value = 0;
    tokenAmount.value = 0;
    activeTab.value = "hyli";
    clearHyliStatus();
    depositError.value = null;
    networkError.value = null;
    claimStatusError.value = null;
};

watch(
    () => props.isOpen,
    (open) => {
        if (open) {
            if (hasNetworks.value && !selectedNetwork.value) {
                setSelectedNetwork(availableNetworks[0]!.id);
            }
            resetFormState();
            refreshAssociation();
        }
    },
);

watch(
    () => selectedNetworkId.value,
    () => {
        tokenAmount.value = 0;
        txHash.value = null;
        networkError.value = null;
        depositError.value = null;
    },
);

const closeModal = () => {
    emit("update:isOpen", false);
};

const hyliAssetsOptions = computed(() => assetsState.list.map((asset) => asset.symbol));

const hyliFormValid = computed(() => {
    if (!hyliAsset.value) return false;
    const amount = Number(hyliAmount.value);
    return Number.isFinite(amount) && amount > 0;
});

const handleHyliDeposit = async () => {
    if (!hyliFormValid.value) return;
    const result = await submitDeposit(hyliAsset.value, hyliAmount.value);
    if (result.success) {
        hyliAmount.value = 0;
    }
};

const canClaimAddress = computed(() => providerAvailable.value && needsBridgeClaim.value);

const hasNetworks = computed(() => availableNetworks.length > 0);

const effectiveNetworkId = computed({
    get: () => selectedNetworkId.value || undefined,
    set: (value: string | undefined) => {
        if (value) {
            setSelectedNetwork(value);
        }
    },
});

const networkTokenAddress = computed(() => selectedNetwork.value?.tokenAddress ?? "");
const networkVaultAddress = computed(() => selectedNetwork.value?.vaultAddress ?? "");

const tokenAmountValid = computed(() => {
    const amount = Number(tokenAmount.value);
    return Number.isFinite(amount) && amount > 0;
});

const canSendTokenDeposit = computed(() => {
    return (
        hasNetworks.value &&
        tokenAmountValid.value &&
        Boolean(associatedAddress.value) &&
        providerAvailable.value &&
        Boolean(selectedNetwork.value?.tokenAddress) &&
        Boolean(selectedNetwork.value?.vaultAddress) &&
        !needsManualAssociation.value &&
        bridgeClaimed.value &&
        !claimStatusLoading.value &&
        !isSwitchingNetwork.value
    );
});

const currentUsername = computed(() => wallet.value?.username || "current user");

const handleEthereumDeposit = async () => {
    if (!canSendTokenDeposit.value || isSwitchingNetwork.value) return;
    await sendDepositTransaction(tokenAmount.value);
    if (txHash.value) {
        tokenAmount.value = 0;
    }
};
</script>

<template>
    <Teleport to="body">
        <div v-if="props.isOpen" class="fixed inset-0 z-50 flex items-center justify-center">
            <div class="absolute inset-0 bg-black/60" @click="closeModal"></div>
            <div
                class="relative z-10 w-full max-w-xl rounded-lg border border-neutral-800 bg-neutral-900 p-6 shadow-xl"
            >
                <header class="mb-4 flex items-start justify-between">
                    <div>
                        <h2 class="text-lg font-semibold text-neutral-100">Deposit</h2>
                    </div>
                    <button
                        type="button"
                        class="flex h-8 w-8 items-center justify-center rounded bg-neutral-800 text-neutral-400 hover:text-neutral-200"
                        @click="closeModal"
                        aria-label="Close deposit modal"
                    >
                        ×
                    </button>
                </header>

                <nav class="mb-6 flex gap-2 rounded bg-neutral-800/40 p-1 text-sm">
                    <button
                        type="button"
                        class="flex-1 rounded px-3 py-2 transition"
                        :class="
                            activeTab === 'hyli'
                                ? 'bg-neutral-800 text-neutral-100'
                                : 'text-neutral-400 hover:text-neutral-200'
                        "
                        @click="activeTab = 'hyli'"
                    >
                        Hyli Tokens
                    </button>
                    <button
                        type="button"
                        class="flex-1 rounded px-3 py-2 transition"
                        :class="
                            activeTab === 'ethereum'
                                ? 'bg-neutral-800 text-neutral-100'
                                : 'text-neutral-400 hover:text-neutral-200'
                        "
                        @click="activeTab = 'ethereum'"
                    >
                        Ethereum Token
                    </button>
                </nav>

                <section v-if="activeTab === 'hyli'" class="space-y-4">
                    <div>
                        <label class="flex items-center justify-between text-sm text-neutral-300">
                            Asset
                            <span class="text-xs text-neutral-500">Deposit from your Hyli wallet</span>
                        </label>
                        <select
                            v-if="hyliAssetsOptions.length"
                            v-model="hyliAsset"
                            class="mt-1 w-full rounded border border-neutral-700 bg-neutral-800/60 p-2 text-neutral-100 focus:border-cyan-500 focus:outline-none"
                        >
                            <option v-for="asset in hyliAssetsOptions" :key="asset" :value="asset">
                                {{ asset }}
                            </option>
                        </select>
                        <p v-else class="mt-2 text-sm text-neutral-500">No assets available to deposit yet.</p>
                    </div>

                    <div>
                        <label class="flex items-center justify-between text-sm text-neutral-300">
                            Amount
                            <span class="text-xs text-neutral-500">Decimals allowed per asset settings</span>
                        </label>
                        <input
                            v-model="hyliAmount"
                            type="number"
                            min="0"
                            step="any"
                            placeholder="0.0"
                            class="mt-1 w-full rounded border border-neutral-700 bg-neutral-800/60 p-2 text-neutral-100 focus:border-cyan-500 focus:outline-none"
                        />
                    </div>

                    <div
                        v-if="hyliError"
                        class="rounded border border-rose-500/40 bg-rose-500/10 px-3 py-2 text-sm text-rose-200"
                    >
                        {{ hyliError }}
                    </div>
                    <div
                        v-if="hyliSuccess"
                        class="rounded border border-emerald-500/40 bg-emerald-500/10 px-3 py-2 text-sm text-emerald-200"
                    >
                        {{ hyliSuccess }}
                    </div>

                    <button
                        type="button"
                        class="w-full rounded bg-cyan-600 px-4 py-2 text-sm font-semibold text-neutral-100 transition hover:bg-cyan-500 disabled:cursor-not-allowed disabled:bg-neutral-700 disabled:text-neutral-400"
                        :disabled="!hyliFormValid || hyliSubmitting"
                        @click="handleHyliDeposit"
                    >
                        <span v-if="hyliSubmitting">Submitting…</span>
                        <span v-else>Deposit to Hyli</span>
                    </button>
                </section>

                <section v-else class="space-y-4">
                    <div class="space-y-3">
                        <div>
                            <label class="flex items-center justify-between text-sm text-neutral-300">
                                Network
                                <span class="text-xs text-neutral-500">Choose the origin chain for the transfer</span>
                            </label>
                            <select
                                v-if="hasNetworks"
                                v-model="effectiveNetworkId"
                                class="mt-1 w-full rounded border border-neutral-700 bg-neutral-800/60 p-2 text-neutral-100 focus:border-cyan-500 focus:outline-none"
                            >
                                <option v-for="network in availableNetworks" :key="network.id" :value="network.id">
                                    {{ network.name }}
                                </option>
                            </select>
                            <p v-else class="mt-2 text-sm text-neutral-500">
                                No collateral networks configured. Update the frontend configuration to enable token
                                transfers.
                            </p>
                        </div>

                        <div class="rounded border border-neutral-800 bg-neutral-800/40 p-3 text-sm text-neutral-300">
                            <div class="space-y-1 text-xs text-neutral-500">
                                <p class="flex items-center justify-between">
                                    Token contract
                                    <span class="font-mono text-neutral-300">
                                        {{ networkTokenAddress || "Configure in env" }}
                                    </span>
                                </p>
                                <p class="flex items-center justify-between">
                                    Vault address
                                    <span class="font-mono text-neutral-300">
                                        {{ networkVaultAddress || "Configure in env" }}
                                    </span>
                                </p>
                            </div>
                        </div>
                    </div>

                    <div
                        v-if="!providerAvailable"
                        class="rounded border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-sm text-amber-200"
                    >
                        An Ethereum provider (e.g. MetaMask) is required to transfer collateral.
                    </div>

                    <div
                        v-if="networkError"
                        class="rounded border border-rose-500/40 bg-rose-500/10 px-3 py-2 text-sm text-rose-200"
                    >
                        {{ networkError }}
                    </div>

                    <div>
                        <label class="flex items-center justify-between text-sm text-neutral-300">
                            Token amount
                            <span class="text-xs text-neutral-500">Amount of ERC-20 tokens to transfer</span>
                        </label>
                        <input
                            v-model="tokenAmount"
                            type="number"
                            min="0"
                            step="any"
                            placeholder="0.0"
                            class="mt-1 w-full rounded border border-neutral-700 bg-neutral-800/60 p-2 text-neutral-100 focus:border-cyan-500 focus:outline-none"
                        />
                    </div>

                    <div class="space-y-2 rounded border border-neutral-800 bg-neutral-800/40 p-3 text-sm">
                        <header class="flex items-center justify-between">
                            <span class="text-neutral-300">Associated address for {{ currentUsername }}</span>
                            <button
                                type="button"
                                class="text-xs text-cyan-400 hover:text-cyan-200"
                                @click="refreshAssociation"
                                :disabled="loadingAssociation"
                            >
                                <span v-if="loadingAssociation">Refreshing…</span>
                                <span v-else>Refresh</span>
                            </button>
                        </header>
                        <p v-if="associatedAddress" class="text-xs text-neutral-400">
                            Linked address:
                            <span class="font-mono text-neutral-200">{{ associatedAddress }}</span>
                        </p>
                        <p v-else class="text-xs text-amber-400">No Ethereum address linked yet.</p>

                        <p v-if="!hasBridgeIdentity" class="text-xs text-neutral-500">
                            Connect your Hyli wallet to check the bridge claim status.
                        </p>
                        <p v-else-if="claimStatusLoading" class="text-xs text-neutral-500">
                            Checking bridge claim status…
                        </p>
                        <p v-else-if="needsBridgeClaim" class="text-xs text-amber-400">
                            This identity is not claimed yet. Claim it before sending a deposit.
                        </p>
                        <p v-else class="text-xs text-emerald-400">Bridge claim already registered.</p>

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

                        <button
                            v-if="canClaimAddress"
                            type="button"
                            class="w-full rounded border border-cyan-600 px-3 py-2 text-xs font-semibold text-cyan-200 transition hover:bg-cyan-600/20 disabled:cursor-not-allowed disabled:border-neutral-700 disabled:text-neutral-500"
                            :disabled="submittingAssociation || claimStatusLoading"
                            @click="requestManualAssociation"
                        >
                            <span v-if="submittingAssociation">Awaiting signature…</span>
                            <span v-else>Claim Ethereum address</span>
                        </button>
                        <p
                            v-else-if="needsBridgeClaim"
                            class="text-xs text-neutral-500"
                        >
                            Connect an Ethereum wallet to claim this identity.
                        </p>
                    </div>

                    <div
                        v-if="depositError && depositError !== networkError"
                        class="rounded border border-rose-500/40 bg-rose-500/10 px-3 py-2 text-sm text-rose-200"
                    >
                        {{ depositError }}
                    </div>
                    <div
                        v-if="txHash"
                        class="rounded border border-emerald-500/40 bg-emerald-500/10 px-3 py-2 text-sm text-emerald-200"
                    >
                        Deposit transaction submitted:
                        <span class="font-mono text-xs">{{ txHash }}</span>
                    </div>

                    <button
                        type="button"
                        class="w-full rounded bg-indigo-600 px-4 py-2 text-sm font-semibold text-neutral-100 transition hover:bg-indigo-500 disabled:cursor-not-allowed disabled:bg-neutral-700 disabled:text-neutral-400"
                        :disabled="!canSendTokenDeposit || isSendingTransaction || isSwitchingNetwork"
                        @click="handleEthereumDeposit"
                    >
                        <span v-if="isSwitchingNetwork">Switching network…</span>
                        <span v-else-if="isSendingTransaction">Sending transaction…</span>
                        <span v-else>Transfer collateral token</span>
                    </button>
                </section>
            </div>
        </div>
    </Teleport>
</template>
