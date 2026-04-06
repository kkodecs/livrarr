//! Auth crypto contracts and real implementation.
//!
//! Satisfies: RUNTIME-AUTH-CRYPTO-001 through RUNTIME-AUTH-CRYPTO-004

use argon2::password_hash::rand_core::OsRng as Argon2OsRng;
use argon2::password_hash::SaltString;
use argon2::{Argon2, Params, PasswordHash, PasswordHasher, PasswordVerifier};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

/// Auth crypto service contract.
#[trait_variant::make(Send)]
pub trait AuthCryptoService: Send + Sync {
    /// Hash a password using argon2id. Returns PHC-format string.
    async fn hash_password(&self, password: &str) -> Result<String, AuthCryptoError>;

    /// Verify a password against a PHC-format hash.
    async fn verify_password(&self, password: &str, hash: &str) -> Result<bool, AuthCryptoError>;

    /// Generate a 32-byte cryptographically random token, hex-encoded (64 chars).
    async fn generate_token(&self) -> Result<String, AuthCryptoError>;

    /// Hash a token using SHA-256. Returns 64-char lowercase hex string.
    async fn hash_token(&self, token: &str) -> Result<String, AuthCryptoError>;

    /// Constant-time byte comparison.
    async fn constant_time_eq(&self, a: &[u8], b: &[u8]) -> Result<bool, AuthCryptoError>;
}

#[derive(Debug, thiserror::Error)]
pub enum AuthCryptoError {
    #[error("hashing failed: {0}")]
    HashFailed(String),
    #[error("invalid hash format: {0}")]
    InvalidHash(String),
}

/// Real auth crypto implementation using argon2id, OsRng, SHA-256, subtle.
pub struct RealAuthCrypto;

impl AuthCryptoService for RealAuthCrypto {
    async fn hash_password(&self, password: &str) -> Result<String, AuthCryptoError> {
        let password = password.to_string();
        tokio::task::spawn_blocking(move || {
            let params = Params::new(19456, 2, 1, Some(32))
                .map_err(|e| AuthCryptoError::HashFailed(e.to_string()))?;
            let argon2 = Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
            let salt = SaltString::generate(&mut Argon2OsRng);
            let hash = argon2
                .hash_password(password.as_bytes(), &salt)
                .map_err(|e| AuthCryptoError::HashFailed(e.to_string()))?;
            Ok(hash.to_string())
        })
        .await
        .map_err(|e| AuthCryptoError::HashFailed(e.to_string()))?
    }

    async fn verify_password(&self, password: &str, hash: &str) -> Result<bool, AuthCryptoError> {
        let password = password.to_string();
        let hash = hash.to_string();
        tokio::task::spawn_blocking(move || {
            let parsed = PasswordHash::new(&hash)
                .map_err(|e| AuthCryptoError::InvalidHash(e.to_string()))?;
            let params = Params::new(19456, 2, 1, Some(32))
                .map_err(|e| AuthCryptoError::HashFailed(e.to_string()))?;
            let argon2 = Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
            Ok(argon2.verify_password(password.as_bytes(), &parsed).is_ok())
        })
        .await
        .map_err(|e| AuthCryptoError::HashFailed(e.to_string()))?
    }

    async fn generate_token(&self) -> Result<String, AuthCryptoError> {
        let mut bytes = [0u8; 32];
        getrandom::getrandom(&mut bytes).map_err(|e| AuthCryptoError::HashFailed(e.to_string()))?;
        Ok(hex::encode(bytes))
    }

    async fn hash_token(&self, token: &str) -> Result<String, AuthCryptoError> {
        let mut hasher = Sha256::new();
        hasher.update(token.as_bytes());
        Ok(hex::encode(hasher.finalize()))
    }

    async fn constant_time_eq(&self, a: &[u8], b: &[u8]) -> Result<bool, AuthCryptoError> {
        if a.len() != b.len() {
            return Ok(false);
        }
        Ok(a.ct_eq(b).into())
    }
}
