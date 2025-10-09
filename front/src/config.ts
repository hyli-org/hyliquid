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
