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
            ETH_COLLATERAL_TOKEN_ADDRESS?: string;
            HYLI_VAULT_ADDRESS?: string;
            COLLATERAL_NETWORKS?: string;
        };
    }
}

export const NODE_BASE_URL =
    window.__CONFIG__?.NODE_BASE_URL || import.meta.env.VITE_NODE_BASE_URL || "http://localhost:4321";

export const WALLET_SERVER_BASE_URL =
    window.__CONFIG__?.WALLET_SERVER_BASE_URL || import.meta.env.VITE_WALLET_SERVER_BASE_URL || "http://localhost:4000";

export const WALLET_WEBSOCKET_URL =
    window.__CONFIG__?.WALLET_WEBSOCKET_URL || import.meta.env.VITE_WALLET_WEBSOCKET_URL || "ws://localhost:8081";

export const API_BASE_URL =
    window.__CONFIG__?.API_BASE_URL || import.meta.env.VITE_API_BASE_URL || "http://localhost:3000";

export const BACKEND_API_URL = ref(
    window.__CONFIG__?.BACKEND_API_URL || import.meta.env.VITE_BACKEND_API_URL || "http://localhost:9002",
);

export const WEBSOCKET_URL =
    window.__CONFIG__?.WEBSOCKET_URL || import.meta.env.VITE_WEBSOCKET_URL || "ws://localhost:3000/ws";

export const GOOGLE_CLIENT_ID =
    window.__CONFIG__?.GOOGLE_CLIENT_ID || import.meta.env.VITE_GOOGLE_CLIENT_ID || undefined;

export const ETH_COLLATERAL_TOKEN_ADDRESS =
    window.__CONFIG__?.ETH_COLLATERAL_TOKEN_ADDRESS || import.meta.env.VITE_ETH_COLLATERAL_TOKEN_ADDRESS || "";

export const HYLI_VAULT_ADDRESS =
    window.__CONFIG__?.HYLI_VAULT_ADDRESS || import.meta.env.VITE_HYLI_VAULT_ADDRESS || "0x15d34aaf54267db7d7c367839aaf71a00a2c6a65";

export interface CollateralNetworkConfig {
    id: string;
    name: string;
    chainId: string;
    tokenAddress: string;
    vaultAddress: string;
    rpcUrl: string;
    blockExplorerUrl?: string;
}

const DEFAULT_COLLATERAL_NETWORKS: CollateralNetworkConfig[] = [
    {
        id: "ethereum-sepolia",
        name: "Ethereum Sepolia",
        chainId: "0xaa36a7",
        tokenAddress: ETH_COLLATERAL_TOKEN_ADDRESS,
        vaultAddress: HYLI_VAULT_ADDRESS,
        rpcUrl: "https://0xrpc.io/sep",
    },
    {
        id: "local-anvil",
        name: "Local (Anvil)",
        chainId: "0x7a69",
        tokenAddress: ETH_COLLATERAL_TOKEN_ADDRESS || "0x6d6Fc2b5B6F71B84838C70ED1719C9D498FdB083",
        vaultAddress: HYLI_VAULT_ADDRESS || "0x15d34aaf54267db7d7c367839aaf71a00a2c6a65",
        rpcUrl: "http://localhost:8545",
    },
];

const parseCollateralNetworks = (): CollateralNetworkConfig[] => {
    const raw =
        window.__CONFIG__?.COLLATERAL_NETWORKS || import.meta.env.VITE_COLLATERAL_NETWORKS || undefined;
    if (!raw) {
        return DEFAULT_COLLATERAL_NETWORKS;
    }

    try {
        const parsed = JSON.parse(raw);
        if (Array.isArray(parsed)) {
            return parsed.filter(
                (network): network is CollateralNetworkConfig =>
                    typeof network === "object" &&
                    network !== null &&
                    typeof network.id === "string" &&
                    typeof network.name === "string" &&
                    typeof network.chainId === "string" &&
                    typeof network.tokenAddress === "string" &&
                    typeof network.vaultAddress === "string",
            );
        }
    } catch (error) {
        console.warn("Failed to parse COLLATERAL_NETWORKS config", error);
    }

    return DEFAULT_COLLATERAL_NETWORKS;
};

export const COLLATERAL_NETWORKS = parseCollateralNetworks();
