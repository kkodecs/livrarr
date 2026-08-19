use livrarr_domain::identity_layer::{
    EvidenceProvenance, IdentityTitleTuple, WorkIdentityEvidence,
};
use livrarr_matching::identity_layer::WorkMatchAuthorityInputs;

/// Adapt legacy display-only consumer records to the F2 matching boundary.
/// These consumers do not have resolved Author ids yet, so the normalized
/// credited name is converted to a stable process-independent surrogate ref.
pub(crate) fn authority_inputs(
    left_title: &str,
    left_author: &str,
    right_title: &str,
    right_author: &str,
) -> WorkMatchAuthorityInputs {
    WorkMatchAuthorityInputs {
        left: evidence(left_title, left_author),
        right: evidence(right_title, right_author),
    }
}

fn evidence(title: &str, author: &str) -> WorkIdentityEvidence {
    let parsed = livrarr_domain::identity_matching::parse_title(title);
    let normalized_author = author
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    WorkIdentityEvidence {
        title: IdentityTitleTuple {
            main: title.to_string(),
            subtitle: parsed.subtitle.clone(),
            volume: parsed
                .series_markers
                .first()
                .map(|marker| marker.number.to_string()),
            normalized_main: parsed.main,
            normalized_subtitle: parsed.subtitle.unwrap_or_default(),
            normalized_volume: parsed
                .series_markers
                .first()
                .map(|marker| marker.number.to_string())
                .unwrap_or_default(),
            provenance: EvidenceProvenance::Migrated,
        },
        primary_author_id: stable_author_ref(&normalized_author),
        routes: vec![],
    }
}

fn stable_author_ref(value: &str) -> i64 {
    let hash = value.bytes().fold(0xcbf29ce484222325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    });
    (hash & i64::MAX as u64) as i64
}
