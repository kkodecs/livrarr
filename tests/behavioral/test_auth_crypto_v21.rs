use librarr_server::auth_crypto::{AuthCryptoService, RealAuthCrypto};

async fn new_test_auth_crypto() -> impl AuthCryptoService {
    RealAuthCrypto
}

// REQ-ID: RUNTIME-AUTH-CRYPTO-001
// IR contract: argon2id hashing returns PHC-format string starting with "$argon2id$"
#[tokio::test]
async fn test_auth_crypto_v21_hash_password_returns_phc_format_string() {
    let auth = new_test_auth_crypto().await;

    let hash = auth
        .hash_password("correct horse battery staple")
        .await
        .expect("hash_password should succeed for a valid password");

    assert!(
        hash.starts_with("$argon2id$"),
        "expected PHC-format argon2id hash, got: {hash}"
    );
}

// REQ-ID: RUNTIME-AUTH-CRYPTO-002
// IR contract: token generation — 32 bytes OsRng, hex-encoded (64 chars)
#[tokio::test]
async fn test_auth_crypto_v21_generate_token_returns_64_char_hex_string() {
    let auth = new_test_auth_crypto().await;

    let token = auth
        .generate_token()
        .await
        .expect("generate_token should succeed");

    assert_eq!(token.len(), 64, "token must be 64 hex chars");
    assert!(
        token.chars().all(|c| c.is_ascii_hexdigit()),
        "token must contain only hex characters, got: {token}"
    );
}

// REQ-ID: RUNTIME-AUTH-CRYPTO-002
// IR contract: SHA-256 token hash — known-answer test vector
#[tokio::test]
async fn test_auth_crypto_v21_hash_token_matches_sha256_known_answer_for_test_string() {
    let auth = new_test_auth_crypto().await;

    let token_hash = auth
        .hash_token("test")
        .await
        .expect("hash_token should succeed for valid UTF-8 input");

    assert_eq!(
        token_hash, "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08",
        "hash_token must match the standard SHA-256 known-answer test vector for 'test'"
    );
}

// REQ-ID: RUNTIME-AUTH-CRYPTO-002
// IR contract: SHA-256 digest is 64-char hex string
#[tokio::test]
async fn test_auth_crypto_v21_hash_token_returns_sha256_hash_as_64_char_hex_string() {
    let auth = new_test_auth_crypto().await;

    let token = auth
        .generate_token()
        .await
        .expect("generate_token should succeed");
    let token_hash = auth
        .hash_token(&token)
        .await
        .expect("hash_token should succeed for generated token");

    assert_eq!(token_hash.len(), 64, "SHA-256 hash must be 64 hex chars");
    assert!(
        token_hash.chars().all(|c| c.is_ascii_hexdigit()),
        "hash must contain only hex characters, got: {token_hash}"
    );
}

// REQ-ID: RUNTIME-AUTH-CRYPTO-001, RUNTIME-AUTH-CRYPTO-004
// IR contract: verify_password succeeds for correct password against own hash
#[tokio::test]
async fn test_auth_crypto_v21_verify_password_succeeds_for_correct_password() {
    let auth = new_test_auth_crypto().await;

    let password = "Tr0ub4dor&3";
    let hash = auth
        .hash_password(password)
        .await
        .expect("hash_password should succeed for valid password");
    let verified = auth
        .verify_password(password, &hash)
        .await
        .expect("verify_password should return Ok for a valid PHC hash");

    assert!(
        verified,
        "verify_password should succeed for the original password"
    );
}

// REQ-ID: RUNTIME-AUTH-CRYPTO-001, RUNTIME-AUTH-CRYPTO-004
// IR contract: verify_password rejects incorrect password
#[tokio::test]
async fn test_auth_crypto_v21_verify_password_fails_for_wrong_password() {
    let auth = new_test_auth_crypto().await;

    let hash = auth
        .hash_password("Tr0ub4dor&3")
        .await
        .expect("hash_password should succeed for valid password");
    let verified = auth.verify_password("wrong-password", &hash).await.expect(
        "verify_password should return Ok for a valid PHC hash even when password is incorrect",
    );

    assert!(
        !verified,
        "verify_password should fail for an incorrect password"
    );
}

// REQ-ID: RUNTIME-AUTH-CRYPTO-004
// IR contract: verify_password returns error for malformed hash (no panic)
#[tokio::test]
async fn test_auth_crypto_v21_verify_password_returns_error_for_malformed_hash() {
    let auth = new_test_auth_crypto().await;

    let result = auth
        .verify_password("password", "not-a-valid-phc-string")
        .await;

    assert!(
        result.is_err(),
        "verify_password must reject malformed hash input with an error"
    );
}

// REQ-ID: RUNTIME-AUTH-CRYPTO-002
// IR contract: generated tokens are unique (OsRng randomness)
#[tokio::test]
async fn test_auth_crypto_v21_generated_tokens_are_unique() {
    let auth = new_test_auth_crypto().await;

    let token_a = auth
        .generate_token()
        .await
        .expect("first generate_token should succeed");
    let token_b = auth
        .generate_token()
        .await
        .expect("second generate_token should succeed");

    assert_ne!(
        token_a, token_b,
        "two generated tokens should differ to demonstrate randomness"
    );
}

// REQ-ID: RUNTIME-AUTH-CRYPTO-001
// IR contract: password hash salt is random (same password, different hashes)
#[tokio::test]
async fn test_auth_crypto_v21_password_hash_salt_is_random_for_same_password() {
    let auth = new_test_auth_crypto().await;

    let password = "same-password";
    let hash_a = auth
        .hash_password(password)
        .await
        .expect("first hash_password should succeed");
    let hash_b = auth
        .hash_password(password)
        .await
        .expect("second hash_password should succeed");

    assert_ne!(
        hash_a, hash_b,
        "hashing the same password twice should produce different PHC strings due to random salt"
    );
    assert!(hash_a.starts_with("$argon2id$"));
    assert!(hash_b.starts_with("$argon2id$"));
}

// REQ-ID: RUNTIME-AUTH-CRYPTO-003
// IR contract: constant_time_eq returns true for identical byte slices
#[tokio::test]
async fn test_auth_crypto_v21_constant_time_eq_returns_true_for_matching_bytes() {
    let auth = new_test_auth_crypto().await;

    let a = b"0123456789abcdef0123456789abcdef";
    let b = b"0123456789abcdef0123456789abcdef";

    let eq = auth
        .constant_time_eq(a, b)
        .await
        .expect("constant_time_eq should succeed for same-length inputs");

    assert!(
        eq,
        "constant_time_eq should return true for identical byte slices"
    );
}

// REQ-ID: RUNTIME-AUTH-CRYPTO-003
// IR contract: constant_time_eq returns false for different byte slices
#[tokio::test]
async fn test_auth_crypto_v21_constant_time_eq_returns_false_for_non_matching_bytes() {
    let auth = new_test_auth_crypto().await;

    let a = b"0123456789abcdef0123456789abcdef";
    let b = b"fedcba9876543210fedcba9876543210";

    let eq = auth
        .constant_time_eq(a, b)
        .await
        .expect("constant_time_eq should succeed for same-length inputs");

    assert!(
        !eq,
        "constant_time_eq should return false for different byte slices"
    );
}

// REQ-ID: RUNTIME-AUTH-CRYPTO-003
// IR contract: constant_time_eq safely returns false for different-length inputs
#[tokio::test]
async fn test_auth_crypto_v21_constant_time_eq_returns_false_for_different_length_inputs() {
    let auth = new_test_auth_crypto().await;

    let a = b"short";
    let b = b"longer";

    let eq = auth
        .constant_time_eq(a, b)
        .await
        .expect("constant_time_eq should safely handle differing lengths");

    assert!(
        !eq,
        "constant_time_eq should return false for inputs of different lengths"
    );
}

// REQ-ID: RUNTIME-AUTH-CRYPTO-002
// IR contract: SHA-256 hash_token is deterministic for same input
#[tokio::test]
async fn test_auth_crypto_v21_hash_token_is_deterministic_for_same_token() {
    let auth = new_test_auth_crypto().await;

    let token = auth
        .generate_token()
        .await
        .expect("generate_token should succeed");
    let hash_a = auth
        .hash_token(&token)
        .await
        .expect("first hash_token should succeed");
    let hash_b = auth
        .hash_token(&token)
        .await
        .expect("second hash_token should succeed");

    assert_eq!(
        hash_a, hash_b,
        "hash_token should be deterministic for the same input token"
    );
}

// REQ-ID: RUNTIME-AUTH-CRYPTO-002
// IR contract: SHA-256 hashes differ for different tokens
#[tokio::test]
async fn test_auth_crypto_v21_hash_token_differs_for_different_tokens() {
    let auth = new_test_auth_crypto().await;

    let token_a = auth
        .generate_token()
        .await
        .expect("first generate_token should succeed");
    let token_b = auth
        .generate_token()
        .await
        .expect("second generate_token should succeed");

    let hash_a = auth
        .hash_token(&token_a)
        .await
        .expect("hash_token should succeed for first token");
    let hash_b = auth
        .hash_token(&token_b)
        .await
        .expect("hash_token should succeed for second token");

    assert_ne!(
        token_a, token_b,
        "precondition failed: generated tokens should differ"
    );
    assert_ne!(
        hash_a, hash_b,
        "SHA-256 hashes of different tokens should differ"
    );
}

// REQ-ID: RUNTIME-AUTH-CRYPTO-001
// IR contract: empty password hashes and verifies without panic
#[tokio::test]
async fn test_auth_crypto_v21_empty_password_round_trips_successfully() {
    let auth = new_test_auth_crypto().await;

    let hash = auth
        .hash_password("")
        .await
        .expect("hash_password should handle empty passwords safely");
    let verified = auth
        .verify_password("", &hash)
        .await
        .expect("verify_password should handle empty passwords against valid hash");

    assert!(hash.starts_with("$argon2id$"));
    assert!(
        verified,
        "empty password should verify successfully against its own hash"
    );
}

// REQ-ID: RUNTIME-AUTH-CRYPTO-002
// IR contract: empty string SHA-256 matches known-answer vector
#[tokio::test]
async fn test_auth_crypto_v21_hash_token_matches_sha256_known_answer_for_empty_string() {
    let auth = new_test_auth_crypto().await;

    let token_hash = auth
        .hash_token("")
        .await
        .expect("hash_token should support empty string input");

    assert_eq!(
        token_hash, "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        "hash_token must match the standard SHA-256 known-answer test vector for empty input"
    );
}

// REQ-ID: RUNTIME-AUTH-CRYPTO-001
// IR contract: excessively long password handled without panic
#[tokio::test]
async fn test_auth_crypto_v21_hash_password_handles_excessively_long_password_without_panic() {
    let auth = new_test_auth_crypto().await;

    let password = "A".repeat(1024 * 1024);

    let result = auth.hash_password(&password).await;

    assert!(
        result.is_ok() || result.is_err(),
        "hash_password must return a structured result for excessively long input rather than panicking"
    );
}
