/// <reference types="vite/client" />

import type { CollateralNetworkConfig } from "./config/defaults";

declare module "*.vue" {
    import type { DefineComponent } from "vue";
    const component: DefineComponent<{}, {}, any>;
    export default component;
}

declare global {
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
            COLLATERAL_NETWORKS?: string | CollateralNetworkConfig[];
        };
    }
}

export {};
