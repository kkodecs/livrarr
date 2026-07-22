//! Scoped, user-bound, expiring stream token (Unit C — alpha hardening).
//!
//! The `<audio src>` element cannot send an `Authorization` header, so the
//! stream endpoint has always taken its credential via a URL query
//! parameter. It used to be the raw session token — a leak of that URL
//! (browser history, proxy/access logs, a shared link) handed over the
//! whole account, not just playback of one book, and it never expired.
//!
//! This module mints a short-lived (24h) HMAC-signed token carrying
//! exactly `{user_id, item_id, purpose, exp}`, so a leak is bounded to one
//! user, one item, streaming only, for at most a day. It reuses the same
//! HMAC-SHA256 mechanism as the cover-proxy signer (`cover_service.rs`'s
//! `sign_url`/`verify_hmac_signature`, `coverproxy.rs`'s
//! `verify_proxy_sig`) via the shared `HasHmacKey` capability — no new
//! crypto dependency, same pattern already used twice in this crate.
//!
//! Domain separation: the cover proxy always signs the raw external image
//! URL string as the HMAC message (always starts with `https://`). This
//! module's signed message always starts with [`DOMAIN_TAG`]
//! (`livrarr.stream.v1|...`), a shape no `https://` URL can ever take.
//! The two signed-message spaces are disjoint, so a valid signature under
//! one scheme can never be replayed as valid under the other — see the
//! `cover_proxy_signature_never_validates_as_a_stream_token` test below.

use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use sha2::Sha256;

use livrarr_domain::{LibraryItemId, UserId};

type HmacSha256 = Hmac<Sha256>;

/// Domain-separation tag — see module docs.
const DOMAIN_TAG: &str = "livrarr.stream.v1";
const PURPOSE_STREAM: &str = "stream";
/// Blast radius of a leak is one user + one item + streaming only, and it
/// expires — a small ceiling; 24h minimizes background re-mints across a
/// long listen.
const TTL_HOURS: i64 = 24;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum StreamTokenError {
    #[error("malformed token")]
    Malformed,
    #[error("invalid signature")]
    InvalidSignature,
    #[error("wrong purpose")]
    WrongPurpose,
    #[error("token expired")]
    Expired,
    #[error("token does not authorize this item")]
    ItemMismatch,
}

fn claims_message(user_id: UserId, item_id: LibraryItemId, purpose: &str, exp: i64) -> String {
    format!("{DOMAIN_TAG}|{user_id}|{item_id}|{purpose}|{exp}")
}

fn sign(message: &str, key: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(message.as_bytes());
    data_encoding::HEXLOWER.encode(&mac.finalize().into_bytes())
}

fn verify_sig(message: &str, sig: &str, key: &[u8]) -> bool {
    let expected = sign(message, key);
    subtle::ConstantTimeEq::ct_eq(expected.as_bytes(), sig.as_bytes()).into()
}

/// Mint a stream token for `(user_id, item_id)`, valid for [`TTL_HOURS`]
/// from `now`. Returns the opaque wire-format token plus its unix-epoch
/// expiry (seconds) — the mint HTTP handler hands both to the frontend so
/// it can schedule a proactive refresh before the token actually expires.
pub fn mint_stream_token(
    key: &[u8],
    user_id: UserId,
    item_id: LibraryItemId,
    now: DateTime<Utc>,
) -> (String, i64) {
    mint_with_purpose(key, user_id, item_id, PURPOSE_STREAM, now)
}

fn mint_with_purpose(
    key: &[u8],
    user_id: UserId,
    item_id: LibraryItemId,
    purpose: &str,
    now: DateTime<Utc>,
) -> (String, i64) {
    let exp = (now + chrono::Duration::hours(TTL_HOURS)).timestamp();
    let message = claims_message(user_id, item_id, purpose, exp);
    let sig = sign(&message, key);
    let token = format!(
        "{}.{}",
        data_encoding::BASE64URL_NOPAD.encode(message.as_bytes()),
        sig
    );
    (token, exp)
}

/// Verify `token`: signature, expiry, `purpose == "stream"`, and that its
/// embedded `item_id` matches `expected_item_id`. On success, returns the
/// `user_id` embedded in the token. The caller MUST still authorize via
/// `FileService::resolve_path(user_id, item_id)` — ownership is
/// re-checked against the database on every stream request, never trusted
/// from the token's claims alone.
pub fn verify_stream_token(
    key: &[u8],
    token: &str,
    expected_item_id: LibraryItemId,
    now: DateTime<Utc>,
) -> Result<UserId, StreamTokenError> {
    let (encoded, sig) = token.split_once('.').ok_or(StreamTokenError::Malformed)?;
    let message_bytes = data_encoding::BASE64URL_NOPAD
        .decode(encoded.as_bytes())
        .map_err(|_| StreamTokenError::Malformed)?;
    let message = String::from_utf8(message_bytes).map_err(|_| StreamTokenError::Malformed)?;

    // Authenticity first: nothing parsed from an unauthenticated message is
    // trustworthy, including the fields used below.
    if !verify_sig(&message, sig, key) {
        return Err(StreamTokenError::InvalidSignature);
    }

    let mut parts = message.split('|');
    let domain = parts.next().ok_or(StreamTokenError::Malformed)?;
    let user_id: UserId = parts
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or(StreamTokenError::Malformed)?;
    let item_id: LibraryItemId = parts
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or(StreamTokenError::Malformed)?;
    let purpose = parts.next().ok_or(StreamTokenError::Malformed)?;
    let exp: i64 = parts
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or(StreamTokenError::Malformed)?;
    if parts.next().is_some() {
        return Err(StreamTokenError::Malformed);
    }

    if domain != DOMAIN_TAG {
        return Err(StreamTokenError::Malformed);
    }
    if purpose != PURPOSE_STREAM {
        return Err(StreamTokenError::WrongPurpose);
    }
    if item_id != expected_item_id {
        return Err(StreamTokenError::ItemMismatch);
    }
    if now.timestamp() >= exp {
        return Err(StreamTokenError::Expired);
    }

    Ok(user_id)
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamTokenResponse {
    pub token: String,
    /// Unix-epoch seconds. The frontend uses this to schedule a proactive
    /// refresh before the token actually expires.
    pub exp: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &[u8] = b"test-hmac-key-32-bytes-long!!!!!";

    fn now() -> DateTime<Utc> {
        Utc::now()
    }

    #[test]
    fn mint_verify_roundtrip_succeeds() {
        let (token, exp) = mint_stream_token(KEY, 7, 42, now());
        assert!(exp > now().timestamp());
        let user_id = verify_stream_token(KEY, &token, 42, now()).unwrap();
        assert_eq!(user_id, 7);
    }

    #[test]
    fn valid_token_replay_within_ttl_succeeds() {
        // A stream token is a stateless bearer credential reused across
        // many HTTP range requests during one playback session — replay
        // within the TTL window must succeed, not just once.
        let (token, _exp) = mint_stream_token(KEY, 1, 2, now());
        assert!(verify_stream_token(KEY, &token, 2, now()).is_ok());
        assert!(verify_stream_token(KEY, &token, 2, now()).is_ok());
        assert!(verify_stream_token(KEY, &token, 2, now()).is_ok());
    }

    #[test]
    fn wrong_item_rejected() {
        let (token, _exp) = mint_stream_token(KEY, 1, 42, now());
        let err = verify_stream_token(KEY, &token, 43, now()).unwrap_err();
        assert_eq!(err, StreamTokenError::ItemMismatch);
    }

    #[test]
    fn wrong_purpose_rejected() {
        let (token, _exp) = mint_with_purpose(KEY, 1, 42, "cover", now());
        let err = verify_stream_token(KEY, &token, 42, now()).unwrap_err();
        assert_eq!(err, StreamTokenError::WrongPurpose);
    }

    #[test]
    fn tampered_token_rejected() {
        let (token, _exp) = mint_stream_token(KEY, 1, 42, now());
        let (claims_part, sig_part) = token.split_once('.').unwrap();
        let flipped_last = if sig_part.ends_with('f') { '0' } else { 'f' };
        let tampered_sig = format!("{}{flipped_last}", &sig_part[..sig_part.len() - 1]);
        let tampered = format!("{claims_part}.{tampered_sig}");
        let err = verify_stream_token(KEY, &tampered, 42, now()).unwrap_err();
        assert_eq!(err, StreamTokenError::InvalidSignature);
    }

    #[test]
    fn expired_token_rejected() {
        let past = now() - chrono::Duration::hours(TTL_HOURS + 1);
        let (token, exp) = mint_stream_token(KEY, 1, 42, past);
        assert!(exp <= now().timestamp());
        let err = verify_stream_token(KEY, &token, 42, now()).unwrap_err();
        assert_eq!(err, StreamTokenError::Expired);
    }

    #[test]
    fn wrong_key_rejected() {
        let (token, _exp) = mint_stream_token(KEY, 1, 42, now());
        let other_key = b"different-hmac-key-32-bytes-lon!";
        let err = verify_stream_token(other_key, &token, 42, now()).unwrap_err();
        assert_eq!(err, StreamTokenError::InvalidSignature);
    }

    #[test]
    fn malformed_token_rejected() {
        assert_eq!(
            verify_stream_token(KEY, "not-a-token", 42, now()).unwrap_err(),
            StreamTokenError::Malformed
        );
        assert_eq!(
            verify_stream_token(KEY, "abc.def", 42, now()).unwrap_err(),
            StreamTokenError::Malformed
        );
    }

    #[test]
    fn cover_proxy_signature_never_validates_as_a_stream_token() {
        // Domain separation: the cover proxy signs the raw external URL
        // string directly (`cover_service::sign_url` /
        // `coverproxy::verify_proxy_sig`). That message can never equal
        // this module's `livrarr.stream.v1|...`-prefixed claims string, so
        // a cover-proxy signature can never be replayed here.
        let cover_style_message = "https://covers.openlibrary.org/b/id/12345-L.jpg";
        let mut mac = HmacSha256::new_from_slice(KEY).unwrap();
        mac.update(cover_style_message.as_bytes());
        let cover_sig = data_encoding::HEXLOWER.encode(&mac.finalize().into_bytes());
        let fake_token = format!(
            "{}.{}",
            data_encoding::BASE64URL_NOPAD.encode(cover_style_message.as_bytes()),
            cover_sig
        );
        let err = verify_stream_token(KEY, &fake_token, 42, now()).unwrap_err();
        assert_eq!(err, StreamTokenError::Malformed);
    }
}
