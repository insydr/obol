use hmac::{Hmac, Mac};
use sha2::Sha256;

/// HMAC-SHA256 type alias.
type HmacSha256 = Hmac<Sha256>;

/// Verify the HMAC-SHA256 signature of a signal payload.
///
/// If the signal server provides a signature header, this function can
/// verify that the payload was not tampered with during transit.  If the
/// server does not provide signatures, this check should be skipped and
/// the risk documented.
///
/// # Arguments
/// * `payload` - The raw JSON payload string
/// * `signature` - The hex-encoded HMAC signature from the server
/// * `secret` - The shared secret key
///
/// # Returns
/// `true` if the signature is valid, `false` otherwise.
pub fn verify_signal_signature(payload: &str, signature: &str, secret: &[u8]) -> bool {
    let mut mac = match HmacSha256::new_from_slice(secret) {
        Ok(m) => m,
        Err(_) => return false,
    };
    mac.update(payload.as_bytes());

    // Decode the hex-encoded signature.
    let sig_bytes = match hex::decode(signature) {
        Ok(b) => b,
        Err(_) => return false,
    };

    mac.verify_slice(&sig_bytes).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verify_valid_signature() {
        let payload = r#"{"mint":"So1111...","source":"fee_claim"}"#;
        let secret = b"test-secret-key";

        let mut mac = HmacSha256::new_from_slice(secret).unwrap();
        mac.update(payload.as_bytes());
        let result = mac.finalize();
        let sig_bytes = result.into_bytes();
        let signature = hex::encode(sig_bytes);

        assert!(verify_signal_signature(payload, &signature, secret));
    }

    #[test]
    fn test_verify_invalid_signature() {
        let payload = r#"{"mint":"So1111...","source":"fee_claim"}"#;
        let secret = b"test-secret-key";
        let wrong_secret = b"wrong-secret";

        let mut mac = HmacSha256::new_from_slice(wrong_secret).unwrap();
        mac.update(payload.as_bytes());
        let result = mac.finalize();
        let sig_bytes = result.into_bytes();
        let signature = hex::encode(sig_bytes);

        assert!(!verify_signal_signature(payload, &signature, secret));
    }

    #[test]
    fn test_verify_tampered_payload() {
        let payload = r#"{"mint":"So1111...","source":"fee_claim"}"#;
        let tampered = r#"{"mint":"Hacked...","source":"fee_claim"}"#;
        let secret = b"test-secret-key";

        let mut mac = HmacSha256::new_from_slice(secret).unwrap();
        mac.update(payload.as_bytes());
        let result = mac.finalize();
        let sig_bytes = result.into_bytes();
        let signature = hex::encode(sig_bytes);

        assert!(!verify_signal_signature(tampered, &signature, secret));
    }
}
