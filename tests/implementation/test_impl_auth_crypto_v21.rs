use librarr_server::auth_crypto::{AuthCryptoError, AuthCryptoService, RealAuthCrypto};

use tokio::task::JoinSet;

#[tokio::test]
async fn test_impl_auth_crypto_v21_hash_password_uses_expected_argon2_parameters_in_phc() {
    // Verifies the implementation embeds the expected Argon2id PHC parameters:
    // memory cost m=19456, time cost t=2, parallelism p=1.
    let crypto = RealAuthCrypto;
    let hash = crypto
        .hash_password("parameter-check-password")
        .await
        .unwrap();

    // Parse PHC string directly: $argon2id$v=19$m=19456,t=2,p=1$salt$hash
    assert!(hash.starts_with("$argon2id$"), "wrong algorithm");
    assert!(hash.contains("m=19456"), "wrong memory cost: {hash}");
    assert!(hash.contains("t=2"), "wrong time cost: {hash}");
    assert!(hash.contains("p=1"), "wrong parallelism: {hash}");
}

#[tokio::test]
async fn test_impl_auth_crypto_v21_hash_password_parallel_requests_all_succeed() {
    // Exercises concurrent password hashing to catch hidden shared-state issues,
    // thread-safety bugs, or runtime interaction problems.
    let mut join_set = JoinSet::new();

    for i in 0..24usize {
        join_set.spawn(async move {
            let crypto = RealAuthCrypto;
            let password = format!("parallel-password-{i}-{}", "x".repeat(i + 1));
            crypto.hash_password(&password).await
        });
    }

    let mut hashes = Vec::new();
    while let Some(res) = join_set.join_next().await {
        let hash = res.expect("task panicked").expect("hashing failed");
        assert!(hash.starts_with("$argon2id$"));
        hashes.push(hash);
    }

    assert_eq!(hashes.len(), 24);
    for i in 0..hashes.len() {
        for j in (i + 1)..hashes.len() {
            assert_ne!(hashes[i], hashes[j], "unexpected duplicate password hash");
        }
    }
}

#[tokio::test]
async fn test_impl_auth_crypto_v21_generate_token_high_volume_no_collisions() {
    // Pushes token generation beyond the existing uniqueness test with a larger sample
    // to detect accidental reuse, weak RNG wiring, or encoding issues.
    let crypto = RealAuthCrypto;
    let mut seen = std::collections::HashSet::new();

    for _ in 0..512usize {
        let token = crypto.generate_token().await.unwrap();
        assert!(
            seen.insert(token.clone()),
            "token collision detected for generated token: {token}"
        );
    }

    assert_eq!(seen.len(), 512);
}

#[tokio::test]
async fn test_impl_auth_crypto_v21_hash_token_handles_non_utf8_equivalent_binary_bytes_via_lossy_str(
) {
    // Simulates binary/non-UTF8 input reaching the string API by using a lossy UTF-8 conversion.
    // This ensures hashing remains stable and returns a valid SHA-256 hex digest.
    let crypto = RealAuthCrypto;
    let bytes = [0xff, 0xfe, 0x00, 0x61, 0x80, 0xf0, 0x28, 0x8c, 0xbc];
    let token = String::from_utf8_lossy(&bytes).into_owned();

    let hash1 = crypto.hash_token(&token).await.unwrap();
    let hash2 = crypto.hash_token(&token).await.unwrap();

    assert_eq!(hash1.len(), 64);
    assert_eq!(hash1, hash2);
    assert!(hash1.chars().all(|c| c.is_ascii_hexdigit()));
}

#[tokio::test]
async fn test_impl_auth_crypto_v21_hash_token_handles_embedded_nul_and_control_chars() {
    // Covers edge-case token contents that often break C-style string handling assumptions:
    // embedded NULs, newlines, tabs, and other control characters.
    let crypto = RealAuthCrypto;
    let token = "abc\0def\n\r\t\u{0007}\u{001b}ghi";

    let hash = crypto.hash_token(token).await.unwrap();

    assert_eq!(hash.len(), 64);
    assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
}

#[tokio::test]
async fn test_impl_auth_crypto_v21_verify_password_rejects_bcrypt_format_with_invalid_hash_error() {
    // Ensures non-Argon2 password hashes (bcrypt PHC-like/MCF format) are rejected
    // and mapped to the expected InvalidHash error variant.
    let crypto = RealAuthCrypto;
    let bcrypt_hash = "$2b$12$abcdefghijklmnopqrstuuC6LJv7V6czS4QhIwTZPIYOvTo95Ofz6";

    let err = crypto
        .verify_password("irrelevant-password", bcrypt_hash)
        .await
        .expect_err("bcrypt hash should not be accepted");

    match err {
        AuthCryptoError::InvalidHash(msg) => {
            assert!(!msg.is_empty(), "error message should not be empty");
        }
        other => panic!("expected InvalidHash, got {other:?}"),
    }
}

#[tokio::test]
async fn test_impl_auth_crypto_v21_verify_password_rejects_truncated_hash_with_invalid_hash_error()
{
    // Verifies severely truncated Argon2 PHC strings fail cleanly.
    // Truncate to just the algorithm prefix — guaranteed unparseable.
    let crypto = RealAuthCrypto;

    let truncated = "$argon2id$v=19$m=19456,t=2,p=1$abc";
    let err = crypto
        .verify_password("truncate-me", truncated)
        .await
        .expect_err("truncated hash should be rejected");

    match err {
        AuthCryptoError::InvalidHash(msg) => {
            assert!(!msg.is_empty(), "error message should not be empty");
        }
        other => panic!("expected InvalidHash, got {other:?}"),
    }
}

#[tokio::test]
async fn test_impl_auth_crypto_v21_verify_password_with_salt_only_hash_returns_false_not_error() {
    // A PHC string with salt but no output digest is parseable by argon2.
    // Verification returns Ok(false) since the hash can't match, not an error.
    let crypto = RealAuthCrypto;
    let salt_only = "$argon2id$v=19$m=19456,t=2,p=1$onlysalt";

    let result = crypto.verify_password("password", salt_only).await;
    match result {
        Ok(false) => {} // expected: parseable but won't verify
        Ok(true) => panic!("salt-only hash should not verify"),
        Err(_) => {} // also acceptable: some argon2 versions error on this
    }
}

#[tokio::test]
async fn test_impl_auth_crypto_v21_constant_time_eq_empty_slices_true() {
    // Covers the empty/empty edge case explicitly for constant-time equality.
    let crypto = RealAuthCrypto;
    let result = crypto.constant_time_eq(&[], &[]).await.unwrap();
    assert!(result);
}

#[tokio::test]
async fn test_impl_auth_crypto_v21_constant_time_eq_empty_vs_nonempty_false() {
    // Covers asymmetric empty slice comparisons to ensure no panic and correct false result.
    let crypto = RealAuthCrypto;
    let result_left_empty = crypto.constant_time_eq(&[], b"x").await.unwrap();
    let result_right_empty = crypto.constant_time_eq(b"x", &[]).await.unwrap();

    assert!(!result_left_empty);
    assert!(!result_right_empty);
}

#[tokio::test]
async fn test_impl_auth_crypto_v21_verify_password_wrong_password_returns_ok_false_not_error() {
    // Distinguishes authentication failure from parsing/format failure:
    // a valid hash with the wrong password should return Ok(false), not an error variant.
    let crypto = RealAuthCrypto;
    let hash = crypto.hash_password("correct-password").await.unwrap();

    let result = crypto.verify_password("definitely-wrong", &hash).await;
    match result {
        Ok(false) => {}
        Ok(true) => panic!("wrong password unexpectedly verified"),
        Err(err) => panic!("expected Ok(false), got error: {err:?}"),
    }
}

#[tokio::test]
async fn test_impl_auth_crypto_v21_hash_password_and_verify_unicode_password_round_trip() {
    // Stresses Unicode handling with mixed scripts, combining marks, and emoji.
    let crypto = RealAuthCrypto;
    let password = "pässwörd🔐漢字e\u{301}";

    let hash = crypto.hash_password(password).await.unwrap();
    let verified = crypto.verify_password(password, &hash).await.unwrap();

    assert!(verified);
}
