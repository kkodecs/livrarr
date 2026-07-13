//! `.kash` link (cross-format resume) data access: `KashLinkDb` trait.

use crate::{DbError, KashLink, LibraryItemId, NewKashLink};

/// `.kash` link data access (cross-format resume). 1:1 per item — UNIQUE on
/// both `audio_item_id` and `ebook_item_id`.
#[trait_variant::make(Send)]
pub trait KashLinkDb: Send + Sync {
    /// Insert or update the link keyed by `audio_item_id` (one transaction).
    /// An identity change (different ebook, different `epub_hash`, or
    /// duration drift beyond tolerance) deletes the link's
    /// `cross_format_state` rows in the same transaction — a furthest mark
    /// recorded against a different alignment/timeline is never
    /// reinterpreted. Linking to an ebook already in another link surfaces
    /// as `DbError::Constraint` (first-link-wins, caller logs).
    async fn upsert_link(&self, link: NewKashLink) -> Result<KashLink, DbError>;

    /// The link containing this item on either side, if any.
    async fn link_for_item(
        &self,
        library_item_id: LibraryItemId,
    ) -> Result<Option<KashLink>, DbError>;

    /// Remove the link for an audio item (scan reconciliation: sidecar gone
    /// or duration-mismatched). Its `cross_format_state` rows cascade.
    /// Idempotent when no link exists.
    async fn delete_link_for_audio(&self, audio_item_id: LibraryItemId) -> Result<(), DbError>;
}
