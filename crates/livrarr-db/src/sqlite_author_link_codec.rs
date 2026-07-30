//! Column codecs for the author-provider-linking tables.
//!
//! Every enum here is stored as the exact snake_case token migration 078's
//! CHECK constraints allow, and every timestamp is written in the same
//! `%Y-%m-%dT%H:%M:%S%.3fZ` shape the migration-078 trigger writes, so column
//! comparisons (`MIN(next_attempt_at, ?)`, lease expiry) stay lexicographic.

use chrono::{DateTime, Utc};
use livrarr_domain::identity_matching::AuthorVerdict;
use livrarr_domain::{
    AuthorCandidateCatalogState, AuthorEvidenceFingerprint, AuthorKeyAttemptState,
    AuthorLinkCandidateReason, AuthorLinkCandidateStatus, AuthorLinkCursor,
    AuthorLinkProgressState, AuthorLinkTrigger, AuthorNameSource, AuthorProvider, AuthorRouteKey,
    AuthorRouteProvenance, AuthorRouteState, OpenLibraryNameRole,
};

use crate::DbError;

/// The stored timestamp shape, identical to migration 078's
/// `strftime('%Y-%m-%dT%H:%M:%fZ', 'now')`.
pub(crate) fn ts(dt: DateTime<Utc>) -> String {
    dt.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

pub(crate) fn now_ts() -> String {
    ts(Utc::now())
}

fn bad(column: &'static str, value: &str) -> DbError {
    DbError::IncompatibleData {
        detail: format!("unknown author-link {column} value: {value:?}"),
    }
}

pub(crate) fn provider_str(provider: AuthorProvider) -> &'static str {
    match provider {
        AuthorProvider::OpenLibrary => "open_library",
        AuthorProvider::Goodreads => "goodreads",
        AuthorProvider::Hardcover => "hardcover",
    }
}

pub(crate) fn parse_provider(raw: &str) -> Result<AuthorProvider, DbError> {
    match raw {
        "open_library" => Ok(AuthorProvider::OpenLibrary),
        "goodreads" => Ok(AuthorProvider::Goodreads),
        "hardcover" => Ok(AuthorProvider::Hardcover),
        other => Err(bad("provider", other)),
    }
}

/// Re-parse a stored `(provider, route_value)` pair through the canonical
/// parser, so an alias that predates canonicalization can never masquerade as
/// a canonical route on the way out.
pub(crate) fn parse_route_key(provider: &str, value: &str) -> Result<AuthorRouteKey, DbError> {
    let provider = parse_provider(provider)?;
    AuthorRouteKey::parse(provider, value).map_err(|_| bad("route_value", value))
}

pub(crate) fn parse_route_state(raw: &str) -> Result<AuthorRouteState, DbError> {
    match raw {
        "active" => Ok(AuthorRouteState::Active),
        "removed" => Ok(AuthorRouteState::Removed),
        other => Err(bad("route state", other)),
    }
}

pub(crate) fn provenance_str(provenance: AuthorRouteProvenance) -> &'static str {
    match provenance {
        AuthorRouteProvenance::LegacyUnguarded => "legacy_unguarded",
        AuthorRouteProvenance::Tier1Inherited => "tier1_inherited",
        AuthorRouteProvenance::ReadarrGuarded => "readarr_guarded",
        AuthorRouteProvenance::UserPicked => "user_picked",
        AuthorRouteProvenance::MergeCoalesced => "merge_coalesced",
    }
}

pub(crate) fn parse_provenance(raw: &str) -> Result<AuthorRouteProvenance, DbError> {
    match raw {
        "legacy_unguarded" => Ok(AuthorRouteProvenance::LegacyUnguarded),
        "tier1_inherited" => Ok(AuthorRouteProvenance::Tier1Inherited),
        "readarr_guarded" => Ok(AuthorRouteProvenance::ReadarrGuarded),
        "user_picked" => Ok(AuthorRouteProvenance::UserPicked),
        "merge_coalesced" => Ok(AuthorRouteProvenance::MergeCoalesced),
        other => Err(bad("provenance", other)),
    }
}

/// Stable consumer ordering: an explicit user pick outranks guarded evidence,
/// which outranks a merge fold, which outranks an un-reverified legacy value.
pub(crate) fn provenance_rank(provenance: AuthorRouteProvenance) -> i64 {
    match provenance {
        AuthorRouteProvenance::UserPicked => 0,
        AuthorRouteProvenance::Tier1Inherited | AuthorRouteProvenance::ReadarrGuarded => 1,
        AuthorRouteProvenance::MergeCoalesced => 2,
        AuthorRouteProvenance::LegacyUnguarded => 3,
    }
}

pub(crate) fn verdict_str(verdict: AuthorVerdict) -> &'static str {
    match verdict {
        AuthorVerdict::Agree => "agree",
        AuthorVerdict::Grey => "grey",
        AuthorVerdict::Disagree => "disagree",
        AuthorVerdict::Abstain => "abstain",
    }
}

pub(crate) fn parse_verdict(raw: &str) -> Result<AuthorVerdict, DbError> {
    match raw {
        "agree" => Ok(AuthorVerdict::Agree),
        "grey" => Ok(AuthorVerdict::Grey),
        "disagree" => Ok(AuthorVerdict::Disagree),
        "abstain" => Ok(AuthorVerdict::Abstain),
        other => Err(bad("verdict", other)),
    }
}

/// Review ordering at equal observed evidence: settled evidence first,
/// unfinished evidence after it, and a failed read last — never as zero.
pub(crate) fn catalog_state_rank(state: AuthorCandidateCatalogState) -> i64 {
    match state {
        AuthorCandidateCatalogState::Complete => 0,
        AuthorCandidateCatalogState::Partial => 1,
        AuthorCandidateCatalogState::Retrying => 2,
        AuthorCandidateCatalogState::Pending => 3,
        AuthorCandidateCatalogState::Unavailable => 4,
    }
}

pub(crate) fn catalog_state_str(state: AuthorCandidateCatalogState) -> &'static str {
    match state {
        AuthorCandidateCatalogState::Pending => "pending",
        AuthorCandidateCatalogState::Partial => "partial",
        AuthorCandidateCatalogState::Retrying => "retrying",
        AuthorCandidateCatalogState::Complete => "complete",
        AuthorCandidateCatalogState::Unavailable => "unavailable",
    }
}

pub(crate) fn parse_catalog_state(raw: &str) -> Result<AuthorCandidateCatalogState, DbError> {
    match raw {
        "pending" => Ok(AuthorCandidateCatalogState::Pending),
        "partial" => Ok(AuthorCandidateCatalogState::Partial),
        "retrying" => Ok(AuthorCandidateCatalogState::Retrying),
        "complete" => Ok(AuthorCandidateCatalogState::Complete),
        "unavailable" => Ok(AuthorCandidateCatalogState::Unavailable),
        other => Err(bad("catalog evidence state", other)),
    }
}

pub(crate) fn candidate_reason_str(reason: AuthorLinkCandidateReason) -> &'static str {
    match reason {
        AuthorLinkCandidateReason::Tier2NameSearch => "tier2_name_search",
        AuthorLinkCandidateReason::NameGuardFailed => "name_guard_failed",
        AuthorLinkCandidateReason::ReadarrNameGuardFailed => "readarr_name_guard_failed",
        AuthorLinkCandidateReason::Tombstoned => "tombstoned",
        AuthorLinkCandidateReason::LegacyContradiction => "legacy_contradiction",
        AuthorLinkCandidateReason::OwnershipCollision => "ownership_collision",
        AuthorLinkCandidateReason::InvalidLegacyRoute => "invalid_legacy_route",
    }
}

pub(crate) fn parse_candidate_reason(raw: &str) -> Result<AuthorLinkCandidateReason, DbError> {
    match raw {
        "tier2_name_search" => Ok(AuthorLinkCandidateReason::Tier2NameSearch),
        "name_guard_failed" => Ok(AuthorLinkCandidateReason::NameGuardFailed),
        "readarr_name_guard_failed" => Ok(AuthorLinkCandidateReason::ReadarrNameGuardFailed),
        "tombstoned" => Ok(AuthorLinkCandidateReason::Tombstoned),
        "legacy_contradiction" => Ok(AuthorLinkCandidateReason::LegacyContradiction),
        "ownership_collision" => Ok(AuthorLinkCandidateReason::OwnershipCollision),
        "invalid_legacy_route" => Ok(AuthorLinkCandidateReason::InvalidLegacyRoute),
        other => Err(bad("candidate reason", other)),
    }
}

pub(crate) fn parse_candidate_status(raw: &str) -> Result<AuthorLinkCandidateStatus, DbError> {
    match raw {
        "pending" => Ok(AuthorLinkCandidateStatus::Pending),
        "dismissed" => Ok(AuthorLinkCandidateStatus::Dismissed),
        "picked" => Ok(AuthorLinkCandidateStatus::Picked),
        "superseded" => Ok(AuthorLinkCandidateStatus::Superseded),
        other => Err(bad("candidate status", other)),
    }
}

pub(crate) fn name_source_str(source: AuthorNameSource) -> &'static str {
    match source {
        AuthorNameSource::User => "user",
        AuthorNameSource::Goodreads => "goodreads",
        AuthorNameSource::Hardcover => "hardcover",
        AuthorNameSource::GoogleBooks => "google_books",
        AuthorNameSource::OpenLibrary => "open_library",
        AuthorNameSource::Readarr => "readarr",
        AuthorNameSource::Import => "import",
        AuthorNameSource::Legacy => "legacy",
    }
}

pub(crate) fn parse_name_source(raw: &str) -> Result<AuthorNameSource, DbError> {
    match raw {
        "user" => Ok(AuthorNameSource::User),
        "goodreads" => Ok(AuthorNameSource::Goodreads),
        "hardcover" => Ok(AuthorNameSource::Hardcover),
        "google_books" => Ok(AuthorNameSource::GoogleBooks),
        "open_library" => Ok(AuthorNameSource::OpenLibrary),
        "readarr" => Ok(AuthorNameSource::Readarr),
        "import" => Ok(AuthorNameSource::Import),
        "legacy" => Ok(AuthorNameSource::Legacy),
        other => Err(bad("name source", other)),
    }
}

/// The name source a provider route's own observations are stored under.
pub(crate) fn name_source_for_provider(provider: AuthorProvider) -> AuthorNameSource {
    match provider {
        AuthorProvider::OpenLibrary => AuthorNameSource::OpenLibrary,
        AuthorProvider::Goodreads => AuthorNameSource::Goodreads,
        AuthorProvider::Hardcover => AuthorNameSource::Hardcover,
    }
}

pub(crate) fn ol_role_str(role: OpenLibraryNameRole) -> &'static str {
    match role {
        OpenLibraryNameRole::Primary => "primary",
        OpenLibraryNameRole::Alias => "alias",
    }
}

pub(crate) fn parse_ol_role(raw: Option<&str>) -> Result<Option<OpenLibraryNameRole>, DbError> {
    match raw {
        None => Ok(None),
        Some("primary") => Ok(Some(OpenLibraryNameRole::Primary)),
        Some("alias") => Ok(Some(OpenLibraryNameRole::Alias)),
        Some(other) => Err(bad("open_library_role", other)),
    }
}

/// Primary outranks Alias outranks "no role asserted" for the same canonical
/// OpenLibrary name — the merge retention order.
pub(crate) fn ol_role_rank(role: Option<OpenLibraryNameRole>) -> i64 {
    match role {
        Some(OpenLibraryNameRole::Primary) => 2,
        Some(OpenLibraryNameRole::Alias) => 1,
        None => 0,
    }
}

pub(crate) fn progress_state_str(state: AuthorLinkProgressState) -> &'static str {
    match state {
        AuthorLinkProgressState::Queued => "queued",
        AuthorLinkProgressState::Running => "running",
        AuthorLinkProgressState::ParkedNoSettledWork => "parked_no_settled_work",
        AuthorLinkProgressState::ParkedNoEvidence => "parked_no_evidence",
        AuthorLinkProgressState::NeedsReview => "needs_review",
        AuthorLinkProgressState::Linked => "linked",
        AuthorLinkProgressState::RetryableFailure => "retryable_failure",
    }
}

pub(crate) fn attempt_state_str(state: AuthorKeyAttemptState) -> &'static str {
    match state {
        AuthorKeyAttemptState::Pending => "pending",
        AuthorKeyAttemptState::Running => "running",
        AuthorKeyAttemptState::Succeeded => "succeeded",
        AuthorKeyAttemptState::Retryable => "retryable",
        AuthorKeyAttemptState::SkippedNotConfigured => "skipped_not_configured",
        AuthorKeyAttemptState::SkippedPermanent => "skipped_permanent",
        AuthorKeyAttemptState::ParkedLayoutDrift => "parked_layout_drift",
    }
}

pub(crate) fn parse_attempt_state(raw: &str) -> Result<AuthorKeyAttemptState, DbError> {
    match raw {
        "pending" => Ok(AuthorKeyAttemptState::Pending),
        "running" => Ok(AuthorKeyAttemptState::Running),
        "succeeded" => Ok(AuthorKeyAttemptState::Succeeded),
        "retryable" => Ok(AuthorKeyAttemptState::Retryable),
        "skipped_not_configured" => Ok(AuthorKeyAttemptState::SkippedNotConfigured),
        "skipped_permanent" => Ok(AuthorKeyAttemptState::SkippedPermanent),
        "parked_layout_drift" => Ok(AuthorKeyAttemptState::ParkedLayoutDrift),
        other => Err(bad("key attempt state", other)),
    }
}

pub(crate) fn trigger_str(trigger: AuthorLinkTrigger) -> &'static str {
    match trigger {
        AuthorLinkTrigger::LegacyBackfill => "legacy_backfill",
        AuthorLinkTrigger::AuthorCreated => "author_created",
        AuthorLinkTrigger::AuthorAdopted => "author_adopted",
        AuthorLinkTrigger::UserReResolve => "user_re_resolve",
        AuthorLinkTrigger::EvidenceFingerprintChanged => "evidence_fingerprint_changed",
        AuthorLinkTrigger::DisplayNameDirty => "display_name_dirty",
        AuthorLinkTrigger::RetryDue => "retry_due",
    }
}

/// `tier1:<key_attempt_id>` / `tier2_search` / `tier2_catalog:<OL key>[:<page>]`.
/// The candidate key never contains `:`, so the page tail is unambiguous.
pub(crate) fn cursor_to_string(cursor: &AuthorLinkCursor) -> String {
    match cursor {
        AuthorLinkCursor::Tier1 { key_attempt_id } => format!("tier1:{key_attempt_id}"),
        AuthorLinkCursor::Tier2Search => "tier2_search".to_string(),
        AuthorLinkCursor::Tier2Catalog { candidate, page } => match page {
            Some(page) => format!("tier2_catalog:{}:{page}", candidate.as_str()),
            None => format!("tier2_catalog:{}", candidate.as_str()),
        },
    }
}

/// A cursor that no longer decodes is reported as "no cursor" rather than a
/// guessed position: the road then restarts the tier instead of resuming from
/// a place it cannot prove.
pub(crate) fn cursor_from_string(raw: &str) -> Option<AuthorLinkCursor> {
    if raw == "tier2_search" {
        return Some(AuthorLinkCursor::Tier2Search);
    }
    if let Some(id) = raw.strip_prefix("tier1:") {
        return id
            .parse()
            .ok()
            .map(|key_attempt_id| AuthorLinkCursor::Tier1 { key_attempt_id });
    }
    let rest = raw.strip_prefix("tier2_catalog:")?;
    let (candidate, page) = match rest.split_once(':') {
        Some((candidate, page)) => (candidate, Some(page.to_string())),
        None => (rest, None),
    };
    match AuthorRouteKey::parse(AuthorProvider::OpenLibrary, candidate) {
        Ok(AuthorRouteKey::OpenLibrary(candidate)) => {
            Some(AuthorLinkCursor::Tier2Catalog { candidate, page })
        }
        _ => None,
    }
}

/// `<settled works>:<settled provider keys>:<content hash>`.
pub(crate) fn fingerprint_to_string(fingerprint: &AuthorEvidenceFingerprint) -> String {
    format!(
        "{}:{}:{}",
        fingerprint.settled_work_count,
        fingerprint.settled_provider_key_count,
        fingerprint.content_hash
    )
}

/// An undecodable stored fingerprint reads as "never evaluated", so the road
/// re-evaluates instead of trusting a value it cannot interpret.
pub(crate) fn fingerprint_from_string(raw: &str) -> Option<AuthorEvidenceFingerprint> {
    let mut parts = raw.splitn(3, ':');
    let settled_work_count = parts.next()?.parse().ok()?;
    let settled_provider_key_count = parts.next()?.parse().ok()?;
    let content_hash = parts.next()?.to_string();
    Some(AuthorEvidenceFingerprint {
        settled_work_count,
        settled_provider_key_count,
        content_hash,
    })
}
