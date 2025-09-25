<script setup lang="ts">
import { HyliWallet, setWalletConfig, useWallet } from "hyli-wallet-vue";
import { useApi } from "./api_call";
import { API_BASE_URL, BACKEND_API_URL } from "./config";

setWalletConfig({
    config: {
        nodeBaseUrl: "http://localhost:4321",
        walletServerBaseUrl: "http://localhost:4000",
        applicationWsUrl: "ws://localhost:8081",
    },
    forceSessionKey: true,
});

const { wallet, getOrReuseSessionKey } = useWallet();

// Temporarily here
const deposit = async () => {
    const resp2 = useApi(`${BACKEND_API_URL.value}/add_session_key`, {
        method: "POST",
        headers: {
            "x-identity": wallet.value?.address || "tx_sender",
            "x-public-key": (await getOrReuseSessionKey())?.publicKey || "",
        },
    });
    await resp2.loaded();

    const resp = useApi(`${BACKEND_API_URL.value}/deposit`, {
        method: "POST",
        body: JSON.stringify({ token: "ORANJ", amount: 1000000000 }),
        headers: { "Content-Type": "application/json", "x-identity": wallet.value?.address || "tx_sender" },
    });
    await resp.loaded();
};
</script>

<template>
    <div class="min-h-screen w-full bg-neutral-950 text-neutral-200">
        <div class="flex w-full h-16 justify-between items-center px-4">
            <h3>HYLIQUID</h3>
            <div class="flex justify-between items-center gap-4">
                <p v-if="wallet?.address">Logged in as {{ wallet?.address }}</p>
                <button @click="deposit">Deposit & add session key</button>
                <HyliWallet></HyliWallet>
            </div>
        </div>
        <RouterView />
    </div>
</template>

<style scoped></style>
