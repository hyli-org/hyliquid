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
            DEFAULT_NETWORK?: CollateralNetworkConfig;
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

export interface CollateralNetworkConfig {
    id: string;
    name: string;
    chainId: string;
    tokenAddress: string;
    vaultAddress: string;
    rpcUrl: string;
    blockExplorerUrl: string;
}

// Default configuration with Sepolia listed first (default)
export const COLLATERAL_NETWORKS: CollateralNetworkConfig[] = [
    {
        id: "ethereum-sepolia",
        name: "Ethereum Sepolia",
        chainId: "0xaa36a7",
        tokenAddress: "0x22CE25BFa5Dcd58A3B52c2A5fa262bDF079A5456",
        vaultAddress: "0x2ffCC85Db88Dbb4047d4d1528CE7739CFB961302",
        rpcUrl: "https://0xrpc.io/sep",
        blockExplorerUrl: "https://sepolia.etherscan.io",
    },
    {
        id: "ethereum-mainnet",
        name: "Ethereum Mainnet",
        chainId: "0x1",
        tokenAddress: "TBD",
        vaultAddress: "0x2ffCC85Db88Dbb4047d4d1528CE7739CFB961302",
        rpcUrl: "https://tbd",
        blockExplorerUrl: "https://etherscan.io",
    },
];

// Default network (Sepolia)
export const DEFAULT_NETWORK =
    window.__CONFIG__?.DEFAULT_NETWORK || import.meta.env.VITE_DEFAULT_NETWORK || COLLATERAL_NETWORKS[0];
