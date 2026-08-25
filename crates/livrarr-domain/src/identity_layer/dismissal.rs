//! Version-1 canonical dismissal key shared by pending reuse, user Dismiss,
//! historical adoption, suppression, and revocation (REQ-005).

use std::collections::BTreeSet;

use serde_json::{json, Value};

use super::matching::WorkIdentityEvidence;
use super::route::{IdentityProvider, RouteKind};
use super::services::SettlementReviewCard;
use super::shared::ReviewKind;
use crate::services::MergeFieldChoiceEntry;

/// One public representation of a version-1 review-dismissal key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewDismissalKeyV1 {
    GroupIdentity {
        user_id: crate::UserId,
        work_ids: Vec<crate::WorkId>,
        proposed: Option<GroupProposedKeyV1>,
        merge_choices: Vec<MergeFieldChoiceEntry>,
    },
    PendingRoute {
        user_id: crate::UserId,
        work_id: crate::WorkId,
        provider: IdentityProvider,
        kind: RouteKind,
        value: String,
    },
    EditionEvidence {
        user_id: crate::UserId,
        edition_id: super::shared::EditionId,
    },
}

/// Normalized GroupIdentity proposed tuple plus route identities. Presentation
/// title fields, provenance, and operational route fields are excluded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupProposedKeyV1 {
    pub normalized_main: String,
    pub normalized_subtitle: String,
    pub normalized_volume: String,
    pub primary_author_id: crate::AuthorId,
    pub routes: Vec<(IdentityProvider, RouteKind, String)>,
}

impl ReviewDismissalKeyV1 {
    pub const KEY_VERSION: i64 = 1;

    pub fn key_version(&self) -> i64 {
        Self::KEY_VERSION
    }

    pub fn kind(&self) -> ReviewKind {
        match self {
            Self::GroupIdentity { .. } => ReviewKind::GroupIdentity,
            Self::PendingRoute { .. } => ReviewKind::PendingRoute,
            Self::EditionEvidence { .. } => ReviewKind::EditionEvidence,
        }
    }

    /// Work ids stored on the membership projection. EditionEvidence has none.
    pub fn work_ids(&self) -> Vec<crate::WorkId> {
        match self {
            Self::GroupIdentity { work_ids, .. } => work_ids.clone(),
            Self::PendingRoute { work_id, .. } => vec![*work_id],
            Self::EditionEvidence { .. } => Vec::new(),
        }
    }

    pub fn from_card(
        user_id: crate::UserId,
        card: &SettlementReviewCard,
        fallback_work_id: Option<crate::WorkId>,
    ) -> Option<Self> {
        match card {
            SettlementReviewCard::GroupIdentity {
                work_ids,
                proposed_identity,
                merge_choices,
            } => {
                let mut work_ids = work_ids.clone();
                work_ids.sort_unstable();
                work_ids.dedup();
                let proposed = proposed_identity.as_ref().map(canonical_group_proposed);
                Some(Self::GroupIdentity {
                    user_id,
                    work_ids,
                    proposed,
                    merge_choices: canonical_merge_choices(merge_choices),
                })
            }
            SettlementReviewCard::PendingRoute { work_id, candidate } => {
                let durable = durable_work_id(*work_id, fallback_work_id)?;
                Some(Self::PendingRoute {
                    user_id,
                    work_id: durable,
                    provider: candidate.route.provider.clone(),
                    kind: candidate.route.kind.clone(),
                    value: candidate.route.value.trim().to_string(),
                })
            }
            SettlementReviewCard::EditionEvidence { edition_id, .. } => {
                Some(Self::EditionEvidence {
                    user_id,
                    edition_id: *edition_id,
                })
            }
            _ => None,
        }
    }

    pub fn canonical_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(&self.canonical_value()?)
    }

    fn canonical_value(&self) -> Result<Value, serde_json::Error> {
        Ok(match self {
            Self::GroupIdentity {
                user_id,
                work_ids,
                proposed,
                merge_choices,
            } => {
                let proposed = match proposed {
                    Some(proposed) => Some(canonical_proposed_value(proposed)?),
                    None => None,
                };
                let merge_choices = merge_choices
                    .iter()
                    .map(serde_json::to_value)
                    .collect::<Result<Vec<_>, _>>()?;
                json!({
                    "key_version": Self::KEY_VERSION,
                    "kind": "GroupIdentity",
                    "user_id": user_id,
                    "work_ids": work_ids,
                    "proposed": proposed,
                    "merge_choices": merge_choices,
                })
            }
            Self::PendingRoute {
                user_id,
                work_id,
                provider,
                kind,
                value,
            } => json!({
                "key_version": Self::KEY_VERSION,
                "kind": "PendingRoute",
                "user_id": user_id,
                "work_id": work_id,
                "provider": provider,
                "route_kind": kind,
                "value": value,
            }),
            Self::EditionEvidence {
                user_id,
                edition_id,
            } => json!({
                "key_version": Self::KEY_VERSION,
                "kind": "EditionEvidence",
                "user_id": user_id,
                "edition_id": edition_id,
            }),
        })
    }
}

fn durable_work_id(
    card_work_id: crate::WorkId,
    fallback_work_id: Option<crate::WorkId>,
) -> Option<crate::WorkId> {
    if card_work_id > 0 {
        Some(card_work_id)
    } else {
        fallback_work_id.filter(|id| *id > 0)
    }
}

fn canonical_group_proposed(identity: &WorkIdentityEvidence) -> GroupProposedKeyV1 {
    let mut route_keys = BTreeSet::new();
    for route in &identity.routes {
        if let Ok(encoded) = serde_json::to_string(&(
            &route.provider,
            &route.kind,
            route.provider_scoped_id.as_str(),
        )) {
            route_keys.insert(encoded);
        }
    }
    let routes = route_keys
        .into_iter()
        .filter_map(|encoded| {
            serde_json::from_str::<(IdentityProvider, RouteKind, String)>(&encoded).ok()
        })
        .collect();
    GroupProposedKeyV1 {
        normalized_main: identity.title.normalized_main.clone(),
        normalized_subtitle: identity.title.normalized_subtitle.clone(),
        normalized_volume: identity.title.normalized_volume.clone(),
        primary_author_id: identity.primary_author_id,
        routes,
    }
}

fn canonical_proposed_value(proposed: &GroupProposedKeyV1) -> Result<Value, serde_json::Error> {
    let routes = proposed
        .routes
        .iter()
        .map(|(provider, kind, value)| serde_json::to_value((provider, kind, value)))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(json!({
        "normalized_main": proposed.normalized_main,
        "normalized_subtitle": proposed.normalized_subtitle,
        "normalized_volume": proposed.normalized_volume,
        "primary_author_id": proposed.primary_author_id,
        "routes": routes,
    }))
}

fn canonical_merge_choices(choices: &[MergeFieldChoiceEntry]) -> Vec<MergeFieldChoiceEntry> {
    let mut unique = BTreeSet::new();
    for choice in choices {
        if let Ok(encoded) = serde_json::to_string(choice) {
            unique.insert(encoded);
        }
    }
    unique
        .into_iter()
        .filter_map(|encoded| serde_json::from_str(&encoded).ok())
        .collect()
}
