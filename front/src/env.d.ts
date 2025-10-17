/// <reference types="vite/client" />

declare module "*.vue" {
    import type { DefineComponent } from "vue";
    const component: DefineComponent<{}, {}, any>;
    export default component;
}

interface EthereumProvider {
    isMetaMask?: boolean;
    request<T = unknown>(args: { method: string; params?: unknown[] }): Promise<T>;
    on?(event: string, handler: (...args: unknown[]) => void): void;
    removeListener?(event: string, handler: (...args: unknown[]) => void): void;
}

interface Window {
    ethereum?: EthereumProvider;
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
