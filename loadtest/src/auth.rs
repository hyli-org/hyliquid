use anyhow::{Context, Result};
use k256::{
    ecdsa::{signature::DigestSigner, Signature, SigningKey},
    SecretKey,
};
use sha3::{Digest, Sha3_256};

/// User authentication context containing identity and cryptographic keys
#[derive(Clone)]
pub struct UserAuth {
    pub identity: String,
    pub signing_key: SigningKey,
    pub public_key_hex: String,
}

impl UserAuth {
    /// Create a new UserAuth from an identity string
    /// This replicates the logic from tx_sender.rs
    pub fn new(identity: &str) -> Result<Self> {
        // Generate keypair from identity (deterministic)
        let mut hasher = Sha3_256::new();
        hasher.update(identity.as_bytes());
        let derived_key = hasher.finalize();
        let private_key_bytes = derived_key.to_vec();

        let secret_key = SecretKey::from_slice(&private_key_bytes)
            .context("Invalid private key derived from identity")?;
        let signing_key = SigningKey::from(secret_key);

        let public_key = signing_key.verifying_key();
        let public_key_bytes = public_key.to_encoded_point(false).as_bytes().to_vec();
        let public_key_hex = hex::encode(public_key_bytes);

        Ok(UserAuth {
            identity: identity.to_string(),
            signing_key,
            public_key_hex,
        })
    }

    /// Create a signature for the given data
    /// Format matches tx_sender: SHA3-256 hash of the data, then ECDSA sign
    pub fn sign(&self, data: &str) -> Result<String> {
        let mut hasher = Sha3_256::new();
        hasher.update(data.as_bytes());

        let signature: Signature = self.signing_key.sign_digest(hasher);
        Ok(hex::encode(signature.to_bytes()))
    }

    /// Create signature for create_order action
    /// Format: {identity}:{nonce}:create_order:{order_id}
    pub fn sign_create_order(&self, nonce: u32, order_id: &str) -> Result<String> {
        let data = format!("{}:{}:create_order:{}", self.identity, nonce, order_id);
        self.sign(&data)
    }

    /// Create signature for cancel action
    /// Format: {identity}:{nonce}:cancel:{order_id}
    pub fn sign_cancel(&self, nonce: u32, order_id: &str) -> Result<String> {
        let data = format!("{}:{}:cancel:{}", self.identity, nonce, order_id);
        self.sign(&data)
    }

    #[allow(dead_code)]
    /// Create signature for withdraw action
    /// Format: {identity}:{nonce}:withdraw:{token}:{amount}
    pub fn sign_withdraw(&self, nonce: u32, token: &str, amount: u64) -> Result<String> {
        let data = format!("{}:{}:withdraw:{}:{}", self.identity, nonce, token, amount);
        self.sign(&data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_auth_creation() {
        let auth = UserAuth::new("test_user").unwrap();
        assert_eq!(auth.identity, "test_user");
        assert!(!auth.public_key_hex.is_empty());
    }

    #[test]
    fn test_user_auth_deterministic() {
        let auth1 = UserAuth::new("test_user").unwrap();
        let auth2 = UserAuth::new("test_user").unwrap();
        assert_eq!(auth1.public_key_hex, auth2.public_key_hex);
    }

    #[test]
    fn test_signature_creation() {
        let auth = UserAuth::new("test_user").unwrap();
        let sig = auth.sign("test_data").unwrap();
        assert!(!sig.is_empty());
        assert_eq!(sig.len(), 128); // 64 bytes in hex = 128 chars
    }

    #[test]
    fn test_create_order_signature() {
        let auth = UserAuth::new("test_user").unwrap();
        let sig = auth.sign_create_order(0, "order_123").unwrap();
        assert!(!sig.is_empty());
    }
}
