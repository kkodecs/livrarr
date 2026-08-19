//! Identity-layer-rewrite (F2) additive materialize-crate vocabulary. IR v1
//! `livrarr-materialize` module (ir-v1-identity-layer-rewrite.yaml:1312-1330).
//!
//! `MaterializeService::materialize(MaterializeRequest) ->
//! Result<MaterializeOutcome, MaterializeError>` already exists verbatim in
//! `livrarr_domain::services::materialize` and is unchanged (not a stub
//! target). `MaterializeIdentityRequest` is the one genuinely new type.

use livrarr_domain::identity_layer::WorkCoverSelection;
use livrarr_domain::services::{CoverSlotState, MaterializeRequest, MaterializeTags};
use livrarr_domain::WorkId;

#[derive(Debug, Clone)]
pub struct MaterializeIdentityRequest {
    pub work_id: WorkId,
    pub primary_author_display_name: String,
    pub selected_covers: WorkCoverSelection,
    pub tags: MaterializeTags,
}

impl MaterializeIdentityRequest {
    /// Map caller-resolved F2 identity selections into the established DB-free
    /// materialization request. Operational file paths/change gates remain in
    /// the template because they are not identity decisions.
    pub fn into_materialize_request(self, mut template: MaterializeRequest) -> MaterializeRequest {
        template.work_id = self.work_id;
        template.tags = self.tags;
        template.tags.author = self.primary_author_display_name;
        template.ebook_cover =
            self.selected_covers
                .ebook
                .as_ref()
                .map_or_else(CoverSlotState::default, |candidate| CoverSlotState {
                    chosen_new_url: Some(candidate.proxy_url.clone()),
                    ..CoverSlotState::default()
                });
        template.audiobook_cover = self
            .selected_covers
            .audiobook
            .as_ref()
            .or_else(|| {
                self.selected_covers
                    .audiobook_is_ebook_fallback
                    .then_some(self.selected_covers.ebook.as_ref())
                    .flatten()
            })
            .map_or_else(CoverSlotState::default, |candidate| CoverSlotState {
                chosen_new_url: Some(candidate.proxy_url.clone()),
                ..CoverSlotState::default()
            });
        template
    }
}
