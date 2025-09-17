use k256::{
    ecdsa::{Signature, VerifyingKey},
    EncodedPoint,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// Verifies that the signature provided in private_input was made with the private key
/// of the specified user by validating:
/// 1. That the public key exists for this user
/// 2. That the signature is valid for the order_id with this public key
pub fn verify_user_signature_authorization(
    user: &str,
    pubkey: &Vec<u8>,
    signature: &Vec<u8>,
    order_id: &str,
    user_session_keys: &BTreeMap<String, Vec<Vec<u8>>>,
) -> Result<(), String> {
    // Verify that the public key exists for this user
    let user_keys = user_session_keys
        .get(user)
        .ok_or_else(|| format!("No session keys found for user {user}"))?;

    if !user_keys.contains(pubkey) {
        return Err(format!("Public key not found for user {user}"));
    }

    // Verify the signature of the order_id with the public key
    if !verify_order_creation_signature(signature, order_id, pubkey) {
        return Err("Invalid signature for order_id".to_string());
    }

    Ok(())
}

/// Verifies a signature for a given message with a public key
/// Uses ECDSA with secp256k1 curve and SHA256 hashing
pub fn verify_order_creation_signature(
    signature: &Vec<u8>,
    order_id: &str,
    public_key: &Vec<u8>,
) -> bool {
    // Parse the signature
    let signature = match Signature::try_from(signature.as_slice()) {
        Ok(sig) => sig,
        Err(_) => return false,
    };

    // Parse the public key - try both compressed and uncompressed formats
    let encoded_point = match EncodedPoint::from_bytes(public_key) {
        Ok(point) => point,
        Err(_) => return false,
    };

    let verifying_key = match VerifyingKey::from_encoded_point(&encoded_point) {
        Ok(key) => key,
        Err(_) => return false,
    };

    // Hash the message (order_id) with SHA256
    let mut hasher = Sha256::new();
    hasher.update(order_id.as_bytes());
    let message_hash = hasher.finalize();

    // Verify the signature
    use k256::ecdsa::signature::Verifier;
    verifying_key.verify(&message_hash, &signature).is_ok()
}
