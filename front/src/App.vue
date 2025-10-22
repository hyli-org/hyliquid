<script setup lang="ts">
import { ref } from "vue";
import { HyliWallet, setWalletConfig, useWallet } from "hyli-wallet-vue";
import { useApi } from "./api_call";
import {
    BACKEND_API_URL,
    NODE_BASE_URL,
    WALLET_SERVER_BASE_URL,
    WALLET_WEBSOCKET_URL,
    GOOGLE_CLIENT_ID,
} from "./config";
import DepositModal from "./components/DepositModal.vue";

setWalletConfig({
    config: {
        nodeBaseUrl: NODE_BASE_URL,
        walletServerBaseUrl: WALLET_SERVER_BASE_URL,
        applicationWsUrl: WALLET_WEBSOCKET_URL,
        providers: {
            password: {
                enabled: true,
            },
            metamask: {
                enabled: true,
            },
            google: GOOGLE_CLIENT_ID
                ? {
                      clientId: GOOGLE_CLIENT_ID,
                  }
                : undefined,
        },
    },
    forceSessionKey: true,
});

const { wallet, getOrReuseSessionKey } = useWallet();
const createPair = async () => {
    const meaningfulPairs = [
        ["bitcoin", "usdt"],
        ["bitcoin", "hyllar"],
        ["oranj", "usdt"],
        ["oranj", "hyllar"],
        ["hyllar", "usdt"],
    ];

    // Create all meaningful pairs
    for (const pair of meaningfulPairs) {
        try {
            await fetch(`${BACKEND_API_URL.value}/create_pair`, {
                method: "POST",
                headers: { "Content-Type": "application/json", "x-identity": "fakeuser" },
                body: JSON.stringify({ base_contract: pair[0], quote_contract: pair[1] }),
            });
            console.log(`Created pair: ${pair[0]}/${pair[1]}`);
        } catch (error) {
            console.error(`Failed to create pair ${pair[0]}/${pair[1]}:`, error);
        }
    }
};
const addSessionKey = async () => {
    const address = wallet.value?.address;
    if (!address) throw new Error("No wallet address");
    const resp2 = useApi(`${BACKEND_API_URL.value}/add_session_key`, {
        method: "POST",
        headers: {
            "x-identity": address,
            "x-public-key": (await getOrReuseSessionKey())?.publicKey || "",
        },
    });
    await resp2.loaded();
};
const onWalletClose = () => {
    const address = wallet.value?.address;
    if (!address) return;
    addSessionKey();
};

const isDepositOpen = ref(false);
</script>

<template>
    <div class="min-h-screen w-full flex flex-col bg-neutral-950 text-neutral-200">
        <DepositModal v-model:is-open="isDepositOpen" />
        <div class="flex w-full h-16 justify-between items-center px-4">
            <h3>HYLILILILILIQUID</h3>
            <div class="flex justify-between items-center gap-4">
                <button
                    @click="createPair"
                    class="px-3 py-1 bg-cyan-600 hover:bg-cyan-700 rounded text-sm cursor-pointer"
                >
                    Create all pairs
                </button>
                <p v-if="wallet?.address">Logged in as {{ wallet?.address }}</p>
                <button
                    v-if="wallet?.address"
                    @click="isDepositOpen = true"
                    class="px-3 py-1 rounded bg-indigo-600 text-sm font-semibold text-neutral-100 transition hover:bg-indigo-500"
                >
                    Deposit
                </button>
                <HyliWallet :on-close="onWalletClose"></HyliWallet>
            </div>
        </div>
        <RouterView />
    </div>
</template>

<style scoped></style>
