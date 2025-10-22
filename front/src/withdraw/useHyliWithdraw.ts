import { ref } from "vue";
import { assetsState } from "../trade/trade";
import { useWallet } from "hyli-wallet-vue";
import { BACKEND_API_URL } from "../config";
import { encodeToHex } from "../utils";

interface WithdrawResult {
    success: boolean;
    error?: string;
}

export interface WithdrawDestination {
    network: string;
    address: string;
}

export const toScaledAmount = (amount: number, scale: number): number => {
    if (!Number.isFinite(amount) || amount <= 0) {
        throw new Error("Invalid amount format");
    }

    const amountStr = amount.toString();
    const [integerPart, fractionalPart = ""] = amountStr.split(".");

    if (fractionalPart.length > scale && fractionalPart.slice(scale).replace(/0+$/, "") !== "") {
        throw new Error(`Too many decimal places (max ${scale})`);
    }

    const normalizedFraction = fractionalPart.padEnd(scale, "0").slice(0, scale);
    const normalized = `${integerPart}${normalizedFraction}`;

    const scaled = Number(normalized);
    if (!Number.isFinite(scaled)) {
        throw new Error("Amount is too large");
    }

    return scaled;
};

export function useHyliWithdraw() {
    const isSubmitting = ref(false);
    const errorMessage = ref<string | null>(null);
    const successMessage = ref<string | null>(null);

    const submitWithdraw = async (symbol: string, amount: number, destination: WithdrawDestination): Promise<WithdrawResult> => {
        const { wallet, getOrReuseSessionKey, signMessageWithSessionKey } = useWallet();
        const address = wallet.value?.address;

        if (!address) {
            errorMessage.value = "Wallet address unavailable";
            return { success: false, error: errorMessage.value };
        }

        const asset = assetsState.list.find((item) => item.symbol === symbol);
        if (!asset) {
            errorMessage.value = `Unknown asset ${symbol}`;
            return { success: false, error: errorMessage.value };
        }

        let scaledAmount: number;
        try {
            scaledAmount = toScaledAmount(amount, asset.scale);
        } catch (error) {
            const message = error instanceof Error ? error.message : "Failed to parse amount";
            errorMessage.value = message;
            return { success: false, error: message };
        }

        isSubmitting.value = true;
        errorMessage.value = null;
        successMessage.value = null;

        try {
            const nonceResponse = await fetch(`${BACKEND_API_URL.value}/nonce`, {
                method: "GET",
                headers: {
                    "x-identity": address,
                },
            });

            if (!nonceResponse.ok) {
                const message = `Failed to fetch nonce (${nonceResponse.status})`;
                errorMessage.value = message;
                return { success: false, error: message };
            }

            const nonce = await nonceResponse.text();

            const sessionKey = await getOrReuseSessionKey();
            if (!sessionKey?.publicKey) {
                throw new Error("No session key available");
            }

            const signed = signMessageWithSessionKey(`${address}:${nonce}:withdraw:${symbol}:${scaledAmount}`);

            const body: Record<string, unknown> = {
                symbol,
                amount: scaledAmount,
                destination
            };

            const response = await fetch(`${BACKEND_API_URL.value}/withdraw`, {
                method: "POST",
                headers: {
                    "Content-Type": "application/json",
                    "x-identity": address,
                    "x-public-key": sessionKey.publicKey,
                    "x-signature": encodeToHex(signed.signature),
                },
                body: JSON.stringify(body),
            });

            if (!response.ok) {
                const message = `Withdraw failed (${response.status})`;
                errorMessage.value = message;
                return { success: false, error: message };
            }

            successMessage.value = `Withdraw submitted for ${amount} ${symbol}`;
            return { success: true };
        } catch (error) {
            const message = error instanceof Error ? error.message : "Withdraw request failed";
            errorMessage.value = message;
            return { success: false, error: message };
        } finally {
            isSubmitting.value = false;
        }
    };

    const clearStatus = () => {
        errorMessage.value = null;
        successMessage.value = null;
    };

    return {
        isSubmitting,
        errorMessage,
        successMessage,
        submitWithdraw,
        clearStatus,
    };
}
