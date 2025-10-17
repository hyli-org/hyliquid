import { computed, ref } from "vue";
import { useWallet } from "hyli-wallet-vue";
import {
    BACKEND_API_URL,
    COLLATERAL_NETWORKS,
    type CollateralNetworkConfig,
    WALLET_SERVER_BASE_URL,
} from "../config";

interface BackendSessionKey {
    key: string;
    expiration_date: number;
    nonce: number;
    laneId?: string;
}

interface AccountInfo {
    account: string;
    auth_method:
        | {
            Password: {
                hash: string;
            };
        }
        | {
              Jwt: {
                  hash: number[];
              };
        }
        | {
            Ethereum: {
                address: string;
            };
        };
    session_keys: BackendSessionKey[];
    nonce: number;
    salt: string;
}

const associationMessage = (username: string) => `i am ${username}: ${Date.now()}`;

const amountRegex = /^\d+(\.\d+)?$/;
const coerceToDecimalString = (value: unknown): string => {
    if (typeof value === "string") return value;
    if (typeof value === "number") {
        if (!Number.isFinite(value)) {
            return "";
        }
        return value.toString();
    }
    if (typeof value === "bigint") {
        return value.toString();
    }
    if (value == null) return "";
    return String(value);
};
const normalizeAmount = (amount: unknown, decimals: number): bigint => {
    const sanitized = coerceToDecimalString(amount).trim();
    if (!amountRegex.test(sanitized)) {
        throw new Error("Invalid amount format");
    }

    const [integerPart, fractionalPart = ""] = sanitized.split(".");

    if (fractionalPart.length > decimals && fractionalPart.slice(decimals).replace(/0+$/, "") !== "") {
        throw new Error(`Amount exceeds ${decimals} decimal places`);
    }

    const normalizedFraction = fractionalPart.padEnd(decimals, "0").slice(0, decimals);
    const joined = `${integerPart}${normalizedFraction}`;
    if (!joined) {
        return 0n;
    }
    return BigInt(joined);
};
const hexFromBigInt = (value: bigint) => `0x${value.toString(16)}`;

const fetchAccountInfo = async (username: string): Promise<AccountInfo | null> => {
    try {
        const response = await fetch(
            `${WALLET_SERVER_BASE_URL}/v1/indexer/contract/wallet/account/${encodeURIComponent(username)}`,
        );
        if (!response.ok) {
            return null;
        }
        return (await response.json()) as AccountInfo;
    } catch {
        return null;
    }
};

const availableNetworks: CollateralNetworkConfig[] = COLLATERAL_NETWORKS.filter(
    (network) => Boolean(network?.id && network?.chainId),
);

export function useEthereumBridge() {
    const { wallet } = useWallet();

    const loadingAssociation = ref(false);
    const associationError = ref<string | null>(null);
    const accountInfo = ref<AccountInfo | null>(null);
    const manualAssociation = ref<{ address: string; signature: string } | null>(null);
    const submittingAssociation = ref(false);
    const submitError = ref<string | null>(null);
    const depositError = ref<string | null>(null);
    const txHash = ref<string | null>(null);
    const isSendingTransaction = ref(false);
    const networkError = ref<string | null>(null);
    const isSwitchingNetwork = ref(false);
    const tokenDecimalsCache = ref<Record<string, number>>({});

    const selectedNetworkId = ref<string | null>(
        availableNetworks.find(network => network.id === "local-anvil")?.id ?? availableNetworks[0]?.id ?? null
    );
    const selectedNetwork = computed<CollateralNetworkConfig | null>(() => {
        if (!selectedNetworkId.value) return null;
        return availableNetworks.find((network) => network.id === selectedNetworkId.value) ?? null;
    });

    const setSelectedNetwork = (networkId: string) => {
        if (selectedNetworkId.value === networkId) {
            return;
        }

        if (availableNetworks.some((network) => network.id === networkId)) {
            selectedNetworkId.value = networkId;
            networkError.value = null;
            depositError.value = null;
            txHash.value = null;
        }
    };

    const normalizedWalletAddress = computed(() => {
        const info = accountInfo.value;
        if (!info || !("Ethereum" in info.auth_method)) {
            return null;
        }

        const { address } = info.auth_method.Ethereum;
        return address ? address.toLowerCase() : null;
    });

    const associatedAddress = computed(() => {
        return manualAssociation.value?.address ?? normalizedWalletAddress.value ?? null;
    });

    const needsManualAssociation = computed(
        () => normalizedWalletAddress.value === null && manualAssociation.value === null,
    );

    const providerAvailable = computed(() => typeof window !== "undefined" && Boolean(window.ethereum));

    const getProvider = (): EthereumProvider => {
        if (!providerAvailable.value || !window.ethereum) {
            throw new Error("Ethereum provider not detected");
        }
        return window.ethereum as EthereumProvider;
    };

    const refreshAssociation = async () => {
        const username = wallet.value?.username;
        if (!username) {
            accountInfo.value = null;
            return;
        }

        loadingAssociation.value = true;
        associationError.value = null;

        try {
            const info = await fetchAccountInfo(username);
            accountInfo.value = info;
            if (!info) {
                associationError.value = "Unable to fetch wallet state from indexer";
            } else if ("Ethereum" in info.auth_method) {
                manualAssociation.value = null;
            }
        } catch (error) {
            const message = error instanceof Error ? error.message : "Failed to fetch wallet state";
            associationError.value = message;
            accountInfo.value = null;
        } finally {
            loadingAssociation.value = false;
        }
    };

    const requestManualAssociation = async () => {
        const username = wallet.value?.username;
        if (!username) {
            throw new Error("Wallet username unavailable");
        }

        submitError.value = null;
        submittingAssociation.value = true;

        try {
            const provider = getProvider();
            const accounts = await provider.request<string[]>({ method: "eth_requestAccounts" });

            if (!accounts || accounts.length === 0) {
                throw new Error("No Ethereum account available");
            }

            const account = accounts[0]!;
            const message = associationMessage(username);
            const signature = await provider.request<string>({
                method: "personal_sign",
                params: [message, account],
            });

            try {
                const response = await fetch(`${BACKEND_API_URL.value}/bridge/associate`, {
                    method: "POST",
                    headers: {
                        "Content-Type": "application/json",
                    },
                    body: JSON.stringify({
                        username,
                        ethAddress: account,
                        signature,
                        message,
                    }),
                });
                if (!response.ok) {
                    throw new Error(`Backend association request failed (${response.status})`);
                }
            } catch (error) {
                const message =
                    error instanceof Error ? error.message : "Failed to notify backend about association";
                submitError.value = message;
                throw error;
            }

            manualAssociation.value = {
                address: account.toLowerCase(),
                signature,
            };
            await refreshAssociation();
        } catch (error) {
            if (!submitError.value) {
                submitError.value = error instanceof Error ? error.message : "Failed to establish association";
            }
            throw error;
        } finally {
            submittingAssociation.value = false;
        }
    };

    const ensureTokenDecimals = async (
        provider: EthereumProvider,
        token: string,
        cacheKey: string,
    ): Promise<number> => {
        const cached = tokenDecimalsCache.value[cacheKey];
        if (cached !== undefined) {
            return cached;
        }

        try {
            const response = await provider.request<string>({
                method: "eth_call",
                params: [
                    {
                        to: token,
                        data: "0x313ce567",
                    },
                    "latest",
                ],
            });
            if (response) {
                const parsed = Number.parseInt(response, 16);
                if (!Number.isNaN(parsed)) {
                    tokenDecimalsCache.value = {
                        ...tokenDecimalsCache.value,
                        [cacheKey]: parsed,
                    };
                    return parsed;
                }
            }
        } catch (error) {
            console.warn("Failed to fetch token decimals, defaulting to 18", error);
        }

        tokenDecimalsCache.value = {
            ...tokenDecimalsCache.value,
            [cacheKey]: 18,
        };
        return 18;
    };

    const encodeErc20Transfer = (recipient: string, amount: bigint): string => {
        const selector = "0xa9059cbb";
        const encodeAddress = (address: string) => address.replace(/^0x/, "").padStart(64, "0");
        const encodeUint256 = (value: bigint) => value.toString(16).padStart(64, "0");

        const toField = encodeAddress(recipient.toLowerCase());
        const amountField = encodeUint256(amount);
        return `${selector}${toField}${amountField}`;
    };

    const ensureProviderNetwork = async (provider: EthereumProvider, network: CollateralNetworkConfig) => {
        const currentChainId = await provider.request<string>({ method: "eth_chainId" });
        if (currentChainId?.toLowerCase() === network.chainId.toLowerCase()) {
            return;
        }

        try {
            await provider.request({
                method: "wallet_switchEthereumChain",
                params: [{ chainId: network.chainId }],
            });
        } catch (switchError: any) {
            if (switchError?.code === 4902 || switchError?.data === 4902) {
                if (!network.rpcUrl) {
                    throw new Error(
                        "Selected network is not available in the wallet and no RPC URL is configured to add it",
                    );
                }

                await provider.request({
                    method: "wallet_addEthereumChain",
                    params: [
                        {
                            chainId: network.chainId,
                            chainName: network.name,
                            rpcUrls: [network.rpcUrl],
                            nativeCurrency: {
                                name: "ETH",
                                symbol: "ETH",
                                decimals: 18,
                            },
                            blockExplorerUrls: network.blockExplorerUrl ? [network.blockExplorerUrl] : undefined,
                        },
                    ],
                });

                await provider.request({
                    method: "wallet_switchEthereumChain",
                    params: [{ chainId: network.chainId }],
                });
            } else {
                const message =
                    switchError instanceof Error ? switchError.message : "Failed to switch wallet network";
                throw new Error(message);
            }
        }
    };

    const sendDepositTransaction = async (amountTokens: number) => {
        depositError.value = null;
        txHash.value = null;

        const network = selectedNetwork.value;
        if (!network) {
            depositError.value = "Please select a network";
            return;
        }

        const address = associatedAddress.value;
        if (!address) {
            depositError.value = "No associated Ethereum address found";
            return;
        }

        const tokenAddress = network.tokenAddress?.trim() ?? "";
        if (!tokenAddress) {
            depositError.value = "Collateral token address is not configured";
            return;
        }

        const vaultAddress = network.vaultAddress?.trim() ?? "";
        if (!/^0x[a-fA-F0-9]{40}$/.test(vaultAddress)) {
            depositError.value = "Vault address is invalid";
            return;
        }

        if (!/^0x[a-fA-F0-9]{40}$/.test(tokenAddress)) {
            depositError.value = "Collateral token address is invalid";
            return;
        }

        isSendingTransaction.value = true;

        try {
            const provider = getProvider();
            isSwitchingNetwork.value = true;
            networkError.value = null;
            try {
                await ensureProviderNetwork(provider, network);
            } catch (switchError) {
                const message =
                    switchError instanceof Error ? switchError.message : "Failed to switch wallet network";
                networkError.value = message;
                depositError.value = message;
                return;
            } finally {
                isSwitchingNetwork.value = false;
            }

            const cacheKey = `${network.chainId.toLowerCase()}:${tokenAddress.toLowerCase()}`;
            const decimals = await ensureTokenDecimals(provider, tokenAddress, cacheKey);
            
            if (!Number.isFinite(amountTokens) || amountTokens <= 0) {
                throw new Error("Amount must be greater than zero");
            }
            
            const amountInput = amountTokens.toString();
            const amountBigInt = normalizeAmount(amountInput, decimals);
            if (amountBigInt <= 0n) {
                throw new Error("Amount must be greater than zero");
            }

            const data = encodeErc20Transfer(vaultAddress, amountBigInt);
            const valueHex = hexFromBigInt(0n);

            const txParams: Record<string, string> = {
                from: address,
                to: tokenAddress,
                value: valueHex,
                data,
            };

            const hash = await provider.request<string>({
                method: "eth_sendTransaction",
                params: [txParams],
            });

            txHash.value = hash;


            const username = wallet.value?.username;
            if (username) {
                try {
                    await fetch(`${BACKEND_API_URL.value}/bridge/claim`, {
                        method: "POST",
                        headers: {
                            "Content-Type": "application/json",
                        },
                        body: JSON.stringify({
                            username,
                            ethAddress: address,
                            txHash: hash,
                            amount: amountInput,
                            tokenAddress,
                            vaultAddress,
                            networkId: network.id,
                            chainId: network.chainId,
                        }),
                    });
                } catch (error) {
                    console.warn("Failed to notify backend about Ethereum deposit", error);
                }
            }
        } catch (error) {
            depositError.value = error instanceof Error ? error.message : "Ethereum transaction failed";
        } finally {
            isSendingTransaction.value = false;
        }
    };

    return {
        loadingAssociation,
        associationError,
        needsManualAssociation,
        associatedAddress,
        submittingAssociation,
        submitError,
        providerAvailable,
        txHash,
        depositError,
        isSendingTransaction,
        networkError,
        isSwitchingNetwork,
        manualAssociation,
        availableNetworks,
        selectedNetwork,
        selectedNetworkId,
        setSelectedNetwork,
        refreshAssociation,
        requestManualAssociation,
        sendDepositTransaction,
    };
}
