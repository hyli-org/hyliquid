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

const normalizeHexLike = (value: string): string => {
    const trimmed = value.trim();
    if (!trimmed.startsWith("0x")) {
        return trimmed.toLowerCase();
    }
    return `0x${trimmed.slice(2).toLowerCase()}`;
};

const buildClaimMessage = (chain: string, ethAddress: string, userIdentity: string): string => {
    const normalizedChain = normalizeHexLike(chain);
    const normalizedAddress = normalizeHexLike(ethAddress);
    return `${normalizedChain}:${normalizedAddress}:${userIdentity}`;
};

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
    const userIdentity = computed(() => wallet.value?.address ?? null);

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
    const bridgeClaimed = ref(false);
    const claimStatusLoading = ref(false);
    const claimStatusError = ref<string | null>(null);
    const claimAddress = ref<string | null>(null);

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
        return address ? normalizeHexLike(address) : null;
    });

    const associatedAddress = computed(() => {
        return claimAddress.value ?? manualAssociation.value?.address ?? normalizedWalletAddress.value ?? null;
    });

    const needsManualAssociation = computed(() => {
        return (
            normalizedWalletAddress.value === null &&
            manualAssociation.value === null &&
            claimAddress.value === null
        );
    });

    const hasBridgeIdentity = computed(() => Boolean(userIdentity.value));
    const needsBridgeClaim = computed(() => hasBridgeIdentity.value && !bridgeClaimed.value);

    const providerAvailable = computed(() => typeof window !== "undefined" && Boolean(window.ethereum));

    const getProvider = (): EthereumProvider => {
        if (!providerAvailable.value || !window.ethereum) {
            throw new Error("Ethereum provider not detected");
        }
        return window.ethereum as EthereumProvider;
    };

    const checkBridgeClaimStatus = async () => {
        const identity = userIdentity.value;
        claimStatusError.value = null;

        if (!identity) {
            bridgeClaimed.value = false;
            claimAddress.value = null;
            return;
        }

        claimStatusLoading.value = true;

        try {
            const response = await fetch(
                `${BACKEND_API_URL.value}/bridge/claim/${encodeURIComponent(identity)}`,
            );

            if (response.status === 404) {
                bridgeClaimed.value = false;
                claimAddress.value = null;
                return;
            }

            if (!response.ok) {
                throw new Error(`Bridge claim status failed (${response.status})`);
            }

            const data = (await response.json()) as { claimed: boolean; eth_address?: string };
            bridgeClaimed.value = Boolean(data.claimed);

            if (bridgeClaimed.value && data.eth_address) {
                claimAddress.value = normalizeHexLike(data.eth_address);
            } else {
                claimAddress.value = null;
            }
        } catch (error) {
            claimStatusError.value =
                error instanceof Error ? error.message : "Failed to load bridge claim status";
            bridgeClaimed.value = false;
            claimAddress.value = null;
        } finally {
            claimStatusLoading.value = false;
        }
    };

    const submitBridgeClaim = async (
        chain: string,
        ethAddress: string,
        identity: string,
        signature: string,
    ): Promise<void> => {
        const normalizedChain = normalizeHexLike(chain);
        const normalizedAddress = normalizeHexLike(ethAddress);

        const response = await fetch(`${BACKEND_API_URL.value}/bridge/claim`, {
            method: "POST",
            headers: {
                "Content-Type": "application/json",
            },
            body: JSON.stringify({
                chain: normalizedChain,
                eth_address: normalizedAddress,
                user_identity: identity,
                signature,
            }),
        });

        if (!response.ok) {
            throw new Error(`Bridge claim failed (${response.status})`);
        }
    };

    const claimBridgeIdentity = async (
        provider: EthereumProvider,
        chain: string,
        ethAddress: string,
        signerAccount?: string,
    ): Promise<string> => {
        const identity = userIdentity.value;
        if (!identity) {
            throw new Error("Wallet identity unavailable");
        }

        const normalizedChain = normalizeHexLike(chain);
        const normalizedAddress = normalizeHexLike(ethAddress);
        const message = buildClaimMessage(normalizedChain, normalizedAddress, identity);

        const accountForSignature = signerAccount ?? ethAddress;
        const signature = await provider.request<string>({
            method: "personal_sign",
            params: [message, accountForSignature],
        });

        await submitBridgeClaim(normalizedChain, normalizedAddress, identity, signature);
        bridgeClaimed.value = true;
        claimStatusError.value = null;
        claimAddress.value = normalizedAddress;

        return signature;
    };

    const resolveSignerAccount = async (
        provider: EthereumProvider,
        targetAddress: string,
    ): Promise<string> => {
        const normalizedTarget = normalizeHexLike(targetAddress);

        try {
            const accounts = await provider.request<string[]>({ method: "eth_accounts" });
            if (accounts?.length) {
                const match = accounts.find(
                    (account) => normalizeHexLike(account) === normalizedTarget,
                );
                if (match) {
                    return match;
                }
            }
        } catch {
            // ignore and fallback to requesting accounts
        }

        const requested = await provider.request<string[]>({ method: "eth_requestAccounts" });
        if (!requested || requested.length === 0) {
            throw new Error("No Ethereum account available");
        }

        const match = requested.find((account) => normalizeHexLike(account) === normalizedTarget);
        if (!match) {
            throw new Error("Connected wallet does not control the associated Ethereum address");
        }

        return match;
    };

    const refreshAssociation = async () => {
        const username = wallet.value?.username;
        if (!username) {
            accountInfo.value = null;
            manualAssociation.value = null;
            bridgeClaimed.value = false;
            claimStatusError.value = null;
            claimAddress.value = null;
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

        await checkBridgeClaimStatus();
    };

    const requestManualAssociation = async () => {
        const identity = userIdentity.value;
        if (!identity) {
            throw new Error("Wallet identity unavailable");
        }

        const network = selectedNetwork.value;
        if (!network) {
            throw new Error("Please select a network before associating an address");
        }

        submitError.value = null;
        submittingAssociation.value = true;

        try {
            const provider = getProvider();
            let targetAddress = associatedAddress.value;
            let signerAccount: string;

            if (targetAddress) {
                signerAccount = await resolveSignerAccount(provider, targetAddress);
            } else {
                const accounts = await provider.request<string[]>({ method: "eth_requestAccounts" });
                if (!accounts || accounts.length === 0) {
                    throw new Error("No Ethereum account available");
                }
                signerAccount = accounts[0]!;
                targetAddress = signerAccount;
            }

            const signature = await claimBridgeIdentity(
                provider,
                network.chainId,
                targetAddress,
                signerAccount,
            );

            manualAssociation.value = {
                address: normalizeHexLike(targetAddress),
                signature,
            };
            bridgeClaimed.value = true;
            claimAddress.value = normalizeHexLike(targetAddress);
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

        if (!bridgeClaimed.value) {
            depositError.value = "Please claim your Ethereum address before depositing";
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

            let signerAccount: string;
            try {
                signerAccount = await resolveSignerAccount(provider, address);
            } catch (accountError) {
                const message =
                    accountError instanceof Error
                        ? accountError.message
                        : "Unable to resolve Ethereum signer account";
                depositError.value = message;
                return;
            }

            const cacheKey = `${network.chainId.toLowerCase()}:${tokenAddress.toLowerCase()}`;
            const decimals = await ensureTokenDecimals(provider, tokenAddress, cacheKey);
            
            if (!Number.isFinite(amountTokens) || amountTokens <= 0) {
                throw new Error("Amount must be greater than zero");
            }
            
            const amountBigInt = normalizeAmount(amountTokens.toString(), decimals);
            if (amountBigInt <= 0n) {
                throw new Error("Amount must be greater than zero");
            }

            const data = encodeErc20Transfer(vaultAddress, amountBigInt);
            const valueHex = hexFromBigInt(0n);

            const txParams: Record<string, string> = {
                from: signerAccount,
                to: tokenAddress,
                value: valueHex,
                data,
            };

            const hash = await provider.request<string>({
                method: "eth_sendTransaction",
                params: [txParams],
            });

            txHash.value = hash;
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
        needsBridgeClaim,
        hasBridgeIdentity,
        bridgeClaimed,
        claimStatusLoading,
        claimStatusError,
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
        checkBridgeClaimStatus,
        refreshAssociation,
        requestManualAssociation,
        sendDepositTransaction,
    };
}
