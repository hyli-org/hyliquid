import { computed, ref } from "vue";
import { useWallet } from "hyli-wallet-vue";
import {
    BACKEND_API_URL,
    COLLATERAL_NETWORKS,
    type CollateralNetworkConfig,
    WALLET_SERVER_BASE_URL,
} from "../config";
import { Interface, parseUnits } from "ethers";

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

const isHexAddress = (value: string): boolean => /^0x[a-fA-F0-9]{40}$/.test(value.trim());

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
    const bridgeClaimed = ref(false);
    const claimStatusLoading = ref(false);
    const claimStatusError = ref<string | null>(null);
    const claimAddress = ref<string | null>(null);

    const setClaimState = ({ claimed, address }: { claimed: boolean; address?: string | null }) => {
        bridgeClaimed.value = claimed;
        claimAddress.value = address ? normalizeHexLike(address) : null;
    };

    const resetClaimState = () => {
        setClaimState({ claimed: false, address: null });
    };

    const requireIdentity = (): string => {
        const identity = userIdentity.value;
        if (!identity) {
            throw new Error("Wallet identity unavailable");
        }
        return identity;
    };

    const selectedNetworkId = ref<string | null>(
        // availableNetworks.find((network) => network.id === "local-anvil")?.id ??
        availableNetworks.find((network) => network.id === "ethereum-sepolia")?.id ??
            availableNetworks[0]?.id ??
            null,
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

    const requireSelectedNetwork = (
        errorMessage = "Please select a network",
    ): CollateralNetworkConfig => {
        const network = selectedNetwork.value;
        if (!network) {
            throw new Error(errorMessage);
        }
        return network;
    };

    const requireAssociatedAddress = (): string => {
        const address = associatedAddress.value;
        if (!address) {
            throw new Error("No associated Ethereum address found");
        }
        return address;
    };

    const requireBridgeClaimed = () => {
        if (!bridgeClaimed.value) {
            throw new Error("Please claim your Ethereum address before depositing");
        }
    };

    const requireHexAddress = (
        value: string | undefined | null,
        errorMessage: string,
        missingMessage?: string,
    ): string => {
        const trimmed = value?.trim() ?? "";
        if (!trimmed) {
            throw new Error(missingMessage ?? errorMessage);
        }
        if (!isHexAddress(trimmed)) {
            throw new Error(errorMessage);
        }
        return normalizeHexLike(trimmed);
    };

    const providerAvailable = computed(() => typeof window !== "undefined" && Boolean(window.ethereum));

    const getProvider = (): EthereumProvider => {
        if (!providerAvailable.value || !window.ethereum) {
            throw new Error("Ethereum provider not detected");
        }
        return window.ethereum as EthereumProvider;
    };

    const checkBridgeClaimStatus = async () => {
        claimStatusError.value = null;

        const identity = userIdentity.value;
        if (!identity) {
            resetClaimState();
            return;
        }

        claimStatusLoading.value = true;

        try {
            const response = await fetch(
                `${BACKEND_API_URL.value}/bridge/claim/${encodeURIComponent(identity)}`,
            );

            if (response.status === 404) {
                resetClaimState();
                return;
            }

            if (!response.ok) {
                throw new Error(`Bridge claim status failed (${response.status})`);
            }

            const data = (await response.json()) as { claimed: boolean; eth_address?: string };
            const claimed = Boolean(data.claimed);
            setClaimState({
                claimed,
                address: claimed && data.eth_address ? data.eth_address : null,
            });
        } catch (error) {
            claimStatusError.value =
                error instanceof Error ? error.message : "Failed to load bridge claim status";
            resetClaimState();
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
        const identity = requireIdentity();

        const normalizedChain = normalizeHexLike(chain);
        const normalizedAddress = normalizeHexLike(ethAddress);
        const message = buildClaimMessage(normalizedChain, normalizedAddress, identity);

        const accountForSignature = signerAccount ?? ethAddress;
        const signature = await provider.request<string>({
            method: "personal_sign",
            params: [message, accountForSignature],
        });

        await submitBridgeClaim(normalizedChain, normalizedAddress, identity, signature);
        claimStatusError.value = null;
        setClaimState({ claimed: true, address: normalizedAddress });

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
            claimStatusError.value = null;
            resetClaimState();
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
        requireIdentity();
        const network = requireSelectedNetwork("Please select a network before associating an address");

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

            const normalizedTarget = normalizeHexLike(targetAddress);
            const signature = await claimBridgeIdentity(
                provider,
                network.chainId,
                normalizedTarget,
                signerAccount,
            );

            manualAssociation.value = {
                address: normalizedTarget,
                signature,
            };
        } catch (error) {
            if (!submitError.value) {
                submitError.value = error instanceof Error ? error.message : "Failed to establish association";
            }
            throw error;
        } finally {
            submittingAssociation.value = false;
        }
    };


    const sendDepositTransaction = async (amountTokens: string) => {
        depositError.value = null;
        txHash.value = null;

        try {
            const network = requireSelectedNetwork();
            const address = requireAssociatedAddress();
            requireBridgeClaimed();

            const tokenAddress = requireHexAddress(
                network.tokenAddress,
                "Collateral token address is invalid",
                "Collateral token address is not configured",
            );
            const vaultAddress = requireHexAddress(network.vaultAddress, "Vault address is invalid");

            isSendingTransaction.value = true;

            const provider = getProvider();
            isSwitchingNetwork.value = true;
            networkError.value = null;

            let signerAccount: string;
            try {
                signerAccount = await resolveSignerAccount(provider, address);
            } catch (accountError) {
                const message =
                    accountError instanceof Error
                        ? accountError.message
                        : "Unable to resolve Ethereum signer account";
                throw new Error(message);
            }

            // FIXME: Do not hardcode this
            const decimals = "wei";
            const erc20 = new Interface(['function transfer(address to, uint256 amount) returns (bool)']);
            
            const amount = parseUnits(amountTokens, decimals);
            const data = erc20.encodeFunctionData('transfer', [vaultAddress, amount]);

            const hash = await provider.request<string>({
                method: "eth_sendTransaction",
                params: [
                    {
                        from: signerAccount,
                        to: tokenAddress,
                        data,
                        value: "0x0",
                    },
                ],
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
