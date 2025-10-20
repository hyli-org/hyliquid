/// <reference types="vite/client" />

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
    }
}

export {};
