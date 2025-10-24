import { ref } from "vue";

// Runtime configuration from window object (set by config.js in production)
// Falls back to environment variables for development
declare global {
    interface Window {
        __CONFIG__?: {
            API_BASE_URL?: string;
            BACKEND_API_URL?: string;
            WEBSOCKET_URL?: string;
            NODE_BASE_URL?: string;
            WALLET_SERVER_BASE_URL?: string;
            WALLET_WEBSOCKET_URL?: string;
            GOOGLE_CLIENT_ID?: string;
            TOKEN_ADDRESS?: string;
            VAULT_ADDRESS?: string;
            RPC_URL?: string;
        };
    }
}

export const NODE_BASE_URL =
    window.__CONFIG__?.NODE_BASE_URL || import.meta.env.VITE_NODE_BASE_URL;

export const WALLET_SERVER_BASE_URL =
    window.__CONFIG__?.WALLET_SERVER_BASE_URL || import.meta.env.VITE_WALLET_SERVER_BASE_URL;

export const WALLET_WEBSOCKET_URL =
    window.__CONFIG__?.WALLET_WEBSOCKET_URL || import.meta.env.VITE_WALLET_WEBSOCKET_URL;

export const API_BASE_URL =
    window.__CONFIG__?.API_BASE_URL || import.meta.env.VITE_API_BASE_URL;

export const BACKEND_API_URL = ref(
    window.__CONFIG__?.BACKEND_API_URL || import.meta.env.VITE_BACKEND_API_URL,
);

export const WEBSOCKET_URL =
    window.__CONFIG__?.WEBSOCKET_URL || import.meta.env.VITE_WEBSOCKET_URL;

export const GOOGLE_CLIENT_ID =
    window.__CONFIG__?.GOOGLE_CLIENT_ID || import.meta.env.VITE_GOOGLE_CLIENT_ID;

export const NETWORK = {
    id: "ethereum-sepolia",
    name: "Ethereum Sepolia",
    chainId: "0xaa36a7",
    tokenAddress: window.__CONFIG__?.TOKEN_ADDRESS || import.meta.env.VITE_TOKEN_ADDRESS || "0x22CE25BFa5Dcd58A3B52c2A5fa262bDF079A5456",
    vaultAddress: window.__CONFIG__?.VAULT_ADDRESS || import.meta.env.VITE_VAULT_ADDRESS || "0x2ffCC85Db88Dbb4047d4d1528CE7739CFB961302",
    rpcUrl: window.__CONFIG__?.RPC_URL || import.meta.env.VITE_RPC_URL || "https://0xrpc.io/sep",
    blockExplorerUrl: "https://sepolia.etherscan.io",
};
