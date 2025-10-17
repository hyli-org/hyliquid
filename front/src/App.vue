<script setup lang="ts">
import { HyliWallet, setWalletConfig, useWallet } from "hyli-wallet-vue";
import { useApi } from "./api_call";
import {
    BACKEND_API_URL,
    NODE_BASE_URL,
    WALLET_SERVER_BASE_URL,
    WALLET_WEBSOCKET_URL,
    GOOGLE_CLIENT_ID,
} from "./config";
import { assetsState, instrumentsState } from "./trade/trade";

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

// Temporarily here
const deposit = async (symbol: string) => {
    const address = wallet.value?.address;
    if (!address) throw new Error("No wallet address");

    const asset = assetsState.list.find((a) => a.symbol === symbol);
    if (!asset) throw new Error("No asset found");

    const resp = useApi(`${BACKEND_API_URL.value}/deposit`, {
        method: "POST",
        body: JSON.stringify({ symbol: symbol, amount: 100 * 10 ** asset.scale }),
        headers: { "Content-Type": "application/json", "x-identity": address },
    });
    await resp.loaded();
};
const createPair = async () => {
    const meaningfulPairs = [
        ["BTC", "USDT"],
        ["BTC", "HYLLAR"],
        ["ORANJ", "USDT"],
        ["ORANJ", "HYLLAR"],
        ["HYLLAR", "USDT"],
    ];

    // Create all meaningful pairs
    for (const pair of meaningfulPairs) {
        try {
            await fetch(`${BACKEND_API_URL.value}/create_pair`, {
                method: "POST",
                headers: { "Content-Type": "application/json", "x-identity": "fakeuser" },
                body: JSON.stringify({ pair }),
            });
            console.log(`Created pair: ${pair[0]}/${pair[1]}`);
        } catch (error) {
            console.error(`Failed to create pair ${pair[0]}/${pair[1]}:`, error);
        }
    }
};
const depositBase = async () => {
    deposit(instrumentsState.selected?.base_asset ?? "");
};
const depositQuote = async () => {
    deposit(instrumentsState.selected?.quote_asset ?? "");
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
</script>

<template>
    <div class="min-h-screen w-full flex flex-col bg-neutral-950 text-neutral-200">
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
                    @click="depositBase"
                    class="px-3 py-1 bg-blue-600 hover:bg-blue-700 rounded text-sm cursor-pointer"
                    v-if="wallet?.address && instrumentsState.selected?.base_asset"
                >
                    Deposit 100 {{ instrumentsState.selected?.base_asset }}
                </button>
                <button
                    @click="depositQuote"
                    class="px-3 py-1 bg-green-600 hover:bg-green-700 rounded text-sm cursor-pointer"
                    v-if="wallet?.address && instrumentsState.selected?.quote_asset"
                >
                    Deposit 100 {{ instrumentsState.selected?.quote_asset }}
                </button>
                <HyliWallet :on-close="onWalletClose"></HyliWallet>
            </div>
        </div>
        <RouterView />
    </div>
</template>

<style scoped></style>
