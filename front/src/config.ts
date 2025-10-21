import { ref } from "vue";

// Simplified configuration with Sepolia as the default
export const NODE_BASE_URL = import.meta.env.VITE_NODE_BASE_URL || "http://localhost:4321";

export const WALLET_SERVER_BASE_URL = import.meta.env.VITE_WALLET_SERVER_BASE_URL || "http://localhost:4000";

export const WALLET_WEBSOCKET_URL = import.meta.env.VITE_WALLET_WEBSOCKET_URL || "ws://localhost:8081";

export const API_BASE_URL = import.meta.env.VITE_API_BASE_URL || "http://localhost:3000";

export const BACKEND_API_URL = ref(
    import.meta.env.VITE_BACKEND_API_URL || "http://localhost:9002",
);

export const WEBSOCKET_URL = import.meta.env.VITE_WEBSOCKET_URL || "ws://localhost:3000/ws";

export const GOOGLE_CLIENT_ID = import.meta.env.VITE_GOOGLE_CLIENT_ID || undefined;

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
        tokenAddress: "0x6d6Fc2b5B6F71B84838C70ED1719C9D498FdB083",
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
export const DEFAULT_NETWORK = COLLATERAL_NETWORKS[0];
