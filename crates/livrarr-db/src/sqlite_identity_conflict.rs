use livrarr_domain::identity::*;
use livrarr_domain::{IdentityStatus, WorkId};
use sqlx::SqliteConnection;

use crate::sqlite::SqliteDb;

impl SqliteDb {}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Derive the correct `IdentityStatus` badge from open conflicts and the
/// validated ledger∪column projection (identity-edit r4): open conflict →
/// Conflict; confirmed-ledger OR valid-column work key → Confirmed; bridge →
/// Provisional; else Pending. Column-only legacy values are real identity
/// (ground truth 6b) — the backfill cannot eliminate every one (intentional
/// duplicate losers stay column-only) — while quarantined-invalid columns
/// earn no badge.
pub(crate) async fn derive_badge_in_tx(
    tx: &mut SqliteConnection,
    work_id: WorkId,
) -> Result<IdentityStatus, sqlx::Error> {
    // If any open conflict exists for this work, keep the Conflict badge.
    let open_conflicts: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM work_identity_conflicts
         WHERE existing_work_id = ?1 AND status = 'open'",
    )
    .bind(work_id)
    .fetch_one(&mut *tx)
    .await?;

    if open_conflicts > 0 {
        return Ok(IdentityStatus::Conflict);
    }

    let confirmed_types: Vec<String> = sqlx::query_scalar(
        "SELECT anchor_type FROM work_identity_anchors
         WHERE work_id = ?1 AND confidence = 'confirmed'",
    )
    .bind(work_id)
    .fetch_all(&mut *tx)
    .await?;

    type SlotColumns = (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let columns: Option<SlotColumns> =
        sqlx::query_as("SELECT ol_key, gr_key, hc_key, isbn_13, asin FROM works WHERE id = ?1")
            .bind(work_id)
            .fetch_optional(&mut *tx)
            .await?;
    let (ol, gr, hc, isbn, asin) = columns.unwrap_or_default();

    let valid_column = |anchor_type: &str, value: &Option<String>| -> bool {
        value
            .as_deref()
            .is_some_and(|v| crate::sqlite_work_identity::column_value_valid(anchor_type, v))
    };
    let slot_effective = |anchor_type: &str, value: &Option<String>| -> bool {
        confirmed_types.iter().any(|t| t == anchor_type) || valid_column(anchor_type, value)
    };

    if slot_effective(AnchorType::OL_WORK, &ol)
        || slot_effective(AnchorType::GR_WORK, &gr)
        || slot_effective(AnchorType::HC_WORK, &hc)
    {
        return Ok(IdentityStatus::Confirmed);
    }
    if slot_effective(AnchorType::ISBN_13, &isbn) || slot_effective(AnchorType::ASIN, &asin) {
        return Ok(IdentityStatus::Provisional);
    }

    Ok(IdentityStatus::Pending)
}

/// Serialize an `IdentityStatus` to the snake_case string stored in the DB.
pub(crate) fn identity_status_str(s: IdentityStatus) -> &'static str {
    match s {
        IdentityStatus::Pending => "pending",
        IdentityStatus::Confirmed => "confirmed",
        IdentityStatus::Provisional => "provisional",
        IdentityStatus::Conflict => "conflict",
        IdentityStatus::NeedsReview => "needs_review",
        IdentityStatus::NotFound => "not_found",
    }
}
