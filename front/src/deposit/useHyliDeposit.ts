import { ref } from "vue";
import { assetsState } from "../trade/trade";
import { useWallet } from "hyli-wallet-vue";
import { BACKEND_API_URL } from "../config";

interface DepositResult {
    success: boolean;
    error?: string;
}

const toScaledAmount = (amount: number, scale: number): number => {
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

export function useHyliDeposit() {
    const isSubmitting = ref(false);
    const errorMessage = ref<string | null>(null);
    const successMessage = ref<string | null>(null);

    const submitDeposit = async (symbol: string, amount: number): Promise<DepositResult> => {
        const { wallet } = useWallet();
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
            const response = await fetch(`${BACKEND_API_URL.value}/deposit`, {
                method: "POST",
                headers: {
                    "Content-Type": "application/json",
                    "x-identity": address,
                },
                body: JSON.stringify({
                    symbol,
                    amount: scaledAmount,
                }),
            });

            if (!response.ok) {
                const message = `Deposit failed (${response.status})`;
                errorMessage.value = message;
                return { success: false, error: message };
            }

            successMessage.value = `Deposit submitted for ${amount} ${symbol}`;
            return { success: true };
        } catch (error) {
            const message = error instanceof Error ? error.message : "Deposit request failed";
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
        submitDeposit,
        clearStatus,
    };
}
