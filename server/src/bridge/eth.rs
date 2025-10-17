use alloy::{
    contract::{ContractInstance, Interface},
    dyn_abi::DynSolValue,
    json_abi::JsonAbi,
    primitives::{keccak256, Address, TxHash, U256},
    providers::{DynProvider, Provider, ProviderBuilder, WsConnect},
    rpc::types::{Filter, Log},
    signers::local::PrivateKeySigner,
};
use anyhow::{Context, Result};
use futures::{Stream, StreamExt};
use reqwest::Url;
use serde_json::json;
use std::str::FromStr;
use std::sync::Arc;

/// Result information about a submitted Ethereum ERC20 transfer.
pub struct EthSendResult {
    pub tx_hash: TxHash,
    pub block_number: Option<u64>,
}

#[derive(Clone)]
pub struct EthClient {
    contract: ContractInstance<DynProvider>,
}

impl EthClient {
    /// Creates a new Ethereum client capable of signing ERC20 transfers.
    pub async fn new(http_url: &str, private_key: &str, contract_address: Address) -> Result<Self> {
        let url = Url::parse(http_url)
            .with_context(|| format!("parsing Ethereum HTTP provider url: {http_url}"))?;

        let signer = PrivateKeySigner::from_str(private_key.trim_start_matches("0x"))
            .context("parsing Ethereum private key")?;

        let provider = ProviderBuilder::new()
            .wallet(signer)
            .connect_http(url)
            .erased();

        let abi: JsonAbi = serde_json::from_value(json!([
            {
                "type": "function",
                "name": "transfer",
                "inputs": [
                    { "name": "to", "type": "address" },
                    { "name": "amount", "type": "uint256" }
                ],
                "outputs": [ { "type": "bool" } ],
                "stateMutability": "nonpayable"
            }
        ]))
        .context("constructing ERC20 transfer ABI")?;

        let interface = Interface::new(abi);
        let contract = ContractInstance::new(contract_address, provider.clone(), interface);

        Ok(Self { contract })
    }

    /// Sends an ERC20 transfer and waits for the receipt.
    pub async fn transfer(&self, to: Address, amount: U256) -> Result<EthSendResult> {
        let call = self
            .contract
            .function(
                "transfer",
                &[DynSolValue::Address(to), DynSolValue::Uint(amount, 256)],
            )
            .context("building ERC20 transfer call")?;

        let pending = call.send().await.context("sending ERC20 transfer")?;
        let tx_hash = *pending.tx_hash();
        let receipt = pending
            .get_receipt()
            .await
            .context("waiting for ERC20 transfer receipt")?;

        Ok(EthSendResult {
            tx_hash,
            block_number: receipt.block_number,
        })
    }
}

/// Petit wrapper pour écouter un contrat Ethereum avec Alloy.
///
/// # Exemples d'utilisation
///
/// ## Écouter tous les événements d'un contrat
/// ```rust
/// use alloy::primitives::Address;
/// use std::str::FromStr;
///
/// # async fn example() -> anyhow::Result<()> {
/// let contract_address = Address::from_str("0x...")?;
/// let listener = EthListener::connect("wss://mainnet.infura.io/ws/v3/YOUR_KEY", contract_address, 18000000).await?;
///
/// let mut stream = listener.event_stream().await?;
/// while let Some(result) = stream.next().await {
///     match result {
///         Ok(log) => println!("Nouvel événement: {:?}", log),
///         Err(e) => eprintln!("Erreur: {}", e),
///     }
/// }
/// # Ok(())
/// # }
/// ```
///
/// ## Écouter des événements spécifiques
/// ```rust
/// # async fn example_specific_events() -> anyhow::Result<()> {
/// let contract_address = Address::from_str("0x...")?;
/// let listener = EthListener::connect("wss://mainnet.infura.io/ws/v3/YOUR_KEY", contract_address, 18000000).await?;
///
/// let mut stream = listener.event_stream_by_signatures(vec![
///     "Transfer(address,address,uint256)",
///     "Approval(address,address,uint256)"
/// ]).await?;
///
/// while let Some(result) = stream.next().await {
///     match result {
///         Ok(log) => println!("Transfer ou Approval: {:?}", log),
///         Err(e) => eprintln!("Erreur: {}", e),
///     }
/// }
/// # Ok(())
/// # }
/// ```
pub struct EthListener {
    provider: Arc<dyn Provider>,
    contract: Address,
}

impl EthListener {
    const RANGE_BATCH_SIZE: u64 = 1000;
    /// Connecte au RPC (ws/http) et crée le listener.
    pub async fn connect(rpc_url: &str, contract: Address) -> Result<Self> {
        let connect = WsConnect::new(rpc_url);
        let provider = ProviderBuilder::new().connect_ws(connect).await?;
        Ok(Self {
            provider: Arc::new(provider),
            contract,
        })
    }

    /// Retourne un stream d'événements concernant le contrat avec filtrage par topics.
    ///
    /// Le stream produit des `Result<Log, Error>` où chaque `Log` représente un événement
    /// émis par le contrat. Les erreurs de connexion sont gérées et propagées dans le stream.
    ///
    /// # Arguments
    /// * `topics` - Optionnel, vecteur de topics pour filtrer les événements spécifiques.
    ///   Chaque topic est un `Vec<Option<[u8; 32]>>` représentant les signatures d'événements.
    ///
    /// Exemple d'usage:
    /// ```rust
    /// // Écouter tous les événements
    /// let mut stream = listener.event_stream_with_topics(None).await?;
    ///
    /// // Écouter seulement un événement spécifique (ex: Transfer)
    /// let transfer_topic = keccak256("Transfer(address,address,uint256)");
    /// let topics = vec![Some(transfer_topic)];
    /// let mut stream = listener.event_stream_with_topics(Some(topics)).await?;
    /// ```
    pub async fn event_stream_with_topics(
        &self,
        topics: Option<Vec<Option<[u8; 32]>>>,
    ) -> Result<impl Stream<Item = Result<Log, Box<dyn std::error::Error + Send + Sync>>>> {
        let mut filter = Filter::new().address(self.contract);

        if let Some(topics) = topics {
            // Appliquer les topics au filtre
            for (i, topic) in topics.into_iter().enumerate() {
                match i {
                    0 => {
                        if let Some(topic) = topic {
                            filter = filter.event_signature(topic);
                        }
                    }
                    1 => {
                        if let Some(topic) = topic {
                            filter = filter.topic1(topic);
                        }
                    }
                    2 => {
                        if let Some(topic) = topic {
                            filter = filter.topic2(topic);
                        }
                    }
                    3 => {
                        if let Some(topic) = topic {
                            filter = filter.topic3(topic);
                        }
                    }
                    _ => break, // Alloy ne supporte que 4 topics maximum
                }
            }
        }

        let sub = self.provider.subscribe_logs(&filter).await?;
        let stream = sub.into_stream();

        // Le stream d'Alloy retourne déjà des Log directement
        // On les convertit en Result<Log, Error> pour une gestion d'erreur cohérente
        let error_stream =
            stream.map(|log| Ok(log) as Result<Log, Box<dyn std::error::Error + Send + Sync>>);

        Ok(error_stream)
    }

    /// Crée un topic d'événement à partir de sa signature.
    ///
    /// # Arguments
    /// * `signature` - La signature de l'événement (ex: "Transfer(address,address,uint256)")
    ///
    /// # Exemple
    /// ```rust
    /// let transfer_topic = EthListener::create_event_topic("Transfer(address,address,uint256)");
    /// ```
    pub fn create_event_topic(signature: &str) -> [u8; 32] {
        keccak256(signature.as_bytes()).into()
    }

    /// Convertit une adresse en topic (32 bytes avec padding)
    ///
    /// # Arguments
    /// * `address` - L'adresse à convertir
    ///
    /// # Exemple
    /// ```rust
    /// let address = Address::from_str("0x15d34AAf54267DB7D7c367839AAf71A00a2C6A65")?;
    /// let topic = EthListener::address_to_topic(address);
    /// ```
    pub fn address_to_topic(address: Address) -> [u8; 32] {
        let mut topic = [0u8; 32];
        topic[12..].copy_from_slice(address.as_slice());
        topic
    }

    pub fn parse_log_data(log: &Log) -> (Address, Address, u64) {
        let from = Address::from_slice(&log.topics()[1][12..]);
        let to = Address::from_slice(&log.topics()[2][12..]);
        let value = u64::from_be_bytes(log.inner.data.data[24..32].try_into().unwrap());
        (from, to, value)
    }

    pub async fn latest_block_number(&self) -> Result<u64> {
        Ok(self.provider.get_block_number().await?)
    }

    /// Écoute les transferts vers une adresse spécifique
    ///
    /// # Arguments
    /// * `target_address` - L'adresse de destination à filtrer
    ///
    /// # Exemple
    /// ```rust
    /// let vault_address = Address::from_str("0x15d34AAf54267DB7D7c367839AAf71A00a2C6A65")?;
    /// let mut stream = listener.stream_transfers_to(vault_address).await?;
    /// ```
    pub async fn stream_transfers_to(
        &self,
        target_address: Address,
    ) -> Result<impl Stream<Item = Result<Log, Box<dyn std::error::Error + Send + Sync>>>> {
        let transfer_topic = Self::create_event_topic("Transfer(address,address,uint256)");
        let target_topic = Self::address_to_topic(target_address);

        let topics = vec![
            Some(transfer_topic), // topic0: signature de l'événement
            None,                 // topic1: from (n'importe qui)
            Some(target_topic),   // topic2: to (adresse cible)
            None,                 // topic3: value (n'importe quelle valeur)
        ];

        self.event_stream_with_topics(Some(topics)).await
    }

    /// Écoute les transferts depuis une adresse spécifique
    pub async fn stream_transfers_from(
        &self,
        source_address: Address,
    ) -> Result<impl Stream<Item = Result<Log, Box<dyn std::error::Error + Send + Sync>>>> {
        let transfer_topic = Self::create_event_topic("Transfer(address,address,uint256)");
        let source_topic = Self::address_to_topic(source_address);

        let topics = vec![
            Some(transfer_topic), // topic0: signature de l'événement
            Some(source_topic),   // topic1: from (adresse source)
            None,                 // topic2: to (n'importe qui)
            None,                 // topic3: value (n'importe quelle valeur)
        ];

        self.event_stream_with_topics(Some(topics)).await
    }

    pub async fn fetch_transfers_to_range(
        &self,
        target_address: Address,
        from_block: u64,
        to_block: u64,
    ) -> Result<Vec<Log>> {
        self.fetch_transfers_with_topics(
            from_block,
            to_block,
            None,
            Some(Self::address_to_topic(target_address)),
        )
        .await
    }

    pub async fn fetch_transfers_from_range(
        &self,
        source_address: Address,
        from_block: u64,
        to_block: u64,
    ) -> Result<Vec<Log>> {
        self.fetch_transfers_with_topics(
            from_block,
            to_block,
            Some(Self::address_to_topic(source_address)),
            None,
        )
        .await
    }

    async fn fetch_transfers_with_topics(
        &self,
        from_block: u64,
        to_block: u64,
        topic1: Option<[u8; 32]>,
        topic2: Option<[u8; 32]>,
    ) -> Result<Vec<Log>> {
        if from_block > to_block {
            return Ok(Vec::new());
        }

        let transfer_topic = Self::create_event_topic("Transfer(address,address,uint256)");
        let mut out = Vec::new();
        let mut current = from_block;

        while current <= to_block {
            let end = (current + Self::RANGE_BATCH_SIZE - 1).min(to_block);
            let mut filter = Filter::new()
                .address(self.contract)
                .from_block(alloy::eips::BlockNumberOrTag::Number(current))
                .to_block(alloy::eips::BlockNumberOrTag::Number(end))
                .event_signature(transfer_topic);

            if let Some(topic) = topic1 {
                filter = filter.topic1(topic);
            }
            if let Some(topic) = topic2 {
                filter = filter.topic2(topic);
            }

            let batch = self.provider.get_logs(&filter).await?;
            out.extend(batch);
            current = end.saturating_add(1);
        }

        Ok(out)
    }
}
