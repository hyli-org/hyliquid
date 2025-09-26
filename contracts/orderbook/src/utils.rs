use k256::{
    ecdsa::{Signature, VerifyingKey},
    EncodedPoint,
};
use sha3::{Digest, Sha3_256};

use crate::smt_values::UserInfo;

/// Verifies that the signature provided in private_input was made with the private key
/// of the specified user by validating:
/// 1. That the public key exists for this user
/// 2. That the signature is valid for the order_id with this public key
pub fn verify_user_signature_authorization(
    user_info: &UserInfo,
    pubkey: &Vec<u8>,
    msg: &str,
    signature: &Vec<u8>,
) -> Result<(), String> {
    // Verify that the public key exists for this user
    if !user_info.session_keys.contains(pubkey) {
        return Err(format!("Public key not found for user {}", user_info.user));
    }

    // Verify the signature of the order_id with the public key
    if !verify_signature(signature, msg, pubkey) {
        return Err("Invalid signature for order_id".to_string());
    }

    Ok(())
}

/// Verifies a signature for a given message with a public key
/// Uses ECDSA with secp256k1 curve and SHA3_256 hashing
pub fn verify_signature(signature: &Vec<u8>, msg: &str, public_key: &Vec<u8>) -> bool {
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

    // Hash the message with SHA3_256
    let mut hasher = Sha3_256::new();
    hasher.update(msg.as_bytes());

    // Verify the signature
    use k256::ecdsa::signature::DigestVerifier;
    verifying_key.verify_digest(hasher, &signature).is_ok()
}
