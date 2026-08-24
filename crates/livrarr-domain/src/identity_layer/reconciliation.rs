//! Dependency-neutral complete-group evaluator, authority-certainty
//! predicate, Author exact/unambiguous predicates, and ManualImport defer
//! formatter. Ordinary road reconciliation and the ManualImport minimum
//! operation call this module as the one authority.

use super::matching::{
    DirectionalMatchVerdicts, LostMatchGuardSet, MainTitleGuard, WorkIdentityEvidence,
    WrongMergeGuardSet,
};
use super::status::CapturedIdentity;
use super::title::IdentityTitleTuple;
use super::{evaluate_match, DeferReason};
use crate::identity_matching::{
    author_verdict, parse_title, title_verdict, unambiguous_author_match, AuthorVerdict,
    TitleVerdict,
};
use crate::{AuthorId, WorkId};

/// One stored Work in a complete group, with the display strings used by
/// REQ-003 defer copy.
#[derive(Debug, Clone, PartialEq)]
pub struct CompleteGroupMember {
    pub work_id: WorkId,
    pub display_title: String,
    pub display_author: String,
    pub identity: CapturedIdentity,
}

/// Shared complete-group decision. Door policy maps `Review` and 2+-member
/// `AutoMerge` to a ManualImport defer; other doors keep their existing
/// absorption/card behaviour.
#[derive(Debug, Clone, PartialEq)]
pub enum CompleteGroupEvaluation {
    Create,
    AutoMerge { members: Vec<CompleteGroupMember> },
    Review { members: Vec<CompleteGroupMember> },
}

/// Candidate identity for the shared evaluator. The repository supplies the
/// live group rows; this type carries no persistence capability.
#[derive(Debug, Clone)]
pub struct CompleteGroupCandidate {
    pub identity_title: IdentityTitleTuple,
    pub primary_author_id: AuthorId,
    pub text_distinction: Option<String>,
}

/// The lost-match guards used by complete-group evaluation. Identical to the
/// road's historical `strict_lost_guards`.
pub fn strict_lost_guards() -> LostMatchGuardSet {
    LostMatchGuardSet {
        one_sided_subtitle_recovery: true,
        shared_edition_id_confirmation: true,
        translation_same_text_signals: Default::default(),
    }
}

/// The wrong-merge guards used by complete-group evaluation. Identical to the
/// road's historical `strict_wrong_merge_guards`.
pub fn strict_wrong_merge_guards() -> WrongMergeGuardSet {
    WrongMergeGuardSet {
        main_title_guard: MainTitleGuard(true),
        volume_conflict_guard: true,
        author_disagreement_guard: true,
        work_key_contradiction_guard: true,
        audited_different_text_guard: true,
    }
}

/// Authority-certainty predicate previously private to metadata.
pub fn authority_certain(verdicts: &DirectionalMatchVerdicts) -> bool {
    matches!(verdicts.title, crate::identity_matching::TitleVerdict::Same)
        && matches!(
            verdicts.author,
            crate::identity_matching::AuthorVerdict::Agree
        )
        && !matches!(
            verdicts.id,
            crate::identity_matching::IdVerdict::WorkKeyContradiction
        )
}

/// Exact-name Author discovery is a case-insensitive full-name match.
pub fn author_name_is_exact(stored_name: &str, requested_name: &str) -> bool {
    stored_name
        .trim()
        .eq_ignore_ascii_case(requested_name.trim())
}

/// Unambiguous Author match: the shared identity-matching picker, which
/// requires exactly one compatible stored name.
pub fn adopt_unambiguous_author(requested: &str, stored_names: &[String]) -> Option<usize> {
    unambiguous_author_match(requested, stored_names)
}

/// Handler/request-start exact-text hint: identity-absorb grade against the
/// stored MAIN title and stored author (ST-011).
pub fn exact_text_hint_holds(
    stored_title: &str,
    stored_author: &str,
    incoming_title: &str,
    incoming_author: &str,
) -> bool {
    let stored = parse_title(stored_title);
    let incoming = parse_title(incoming_title);
    if title_verdict(&stored, &incoming) == TitleVerdict::Same {
        matches!(
            author_verdict(&[stored_author.to_string()], &[incoming_author.to_string()]),
            AuthorVerdict::Agree | AuthorVerdict::Abstain
        )
    } else {
        false
    }
}

/// A member's stored display title: main, plus `: <subtitle>` when stored.
pub fn rendered_stored_title(identity: &CapturedIdentity) -> String {
    let main = identity.identity_title.main.as_str();
    match identity
        .identity_title
        .subtitle
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(subtitle) => format!("{main}: {subtitle}"),
        None => main.to_string(),
    }
}

pub fn captured_as_evidence(identity: &CapturedIdentity) -> WorkIdentityEvidence {
    WorkIdentityEvidence {
        title: identity.identity_title.clone(),
        primary_author_id: identity.primary_author_id,
        routes: identity.active_routes.clone(),
    }
}

pub fn complete_group_member(
    identity: CapturedIdentity,
    display_author: String,
) -> CompleteGroupMember {
    CompleteGroupMember {
        work_id: identity.own_work_id,
        display_title: rendered_stored_title(&identity),
        display_author,
        identity,
    }
}

/// Order stored display rows by rendered title, then author, then Work id.
/// REQ-003's "casefold" is Unicode `to_lowercase` — the project's one text
/// normalization, matching what settlement stores in `normalized_author`.
pub fn order_complete_group_members(members: &mut [CompleteGroupMember]) {
    members.sort_by(|left, right| {
        left.display_title
            .to_lowercase()
            .cmp(&right.display_title.to_lowercase())
            .then_with(|| {
                left.display_author
                    .to_lowercase()
                    .cmp(&right.display_author.to_lowercase())
            })
            .then_with(|| left.work_id.cmp(&right.work_id))
    });
}

/// Exact REQ-003 singular defer copy.
pub fn format_singular_defer(title: &str, author: &str) -> String {
    format!(
        "Not imported: you already have \"{title}\" by {author}. Choose Edit title and author for this row. To add this file to that book, enter exactly \"{title}\" and \"{author}\", then retry. To add it as a different book, enter a different main title (a different subtitle alone is not enough)."
    )
}

/// Exact REQ-003 plural defer copy. `members` must already be ordered.
pub fn format_plural_defer(members: &[CompleteGroupMember]) -> String {
    let book_list = members
        .iter()
        .enumerate()
        .map(|(index, member)| {
            format!(
                "({}) \"{}\" by {}",
                index + 1,
                member.display_title,
                member.display_author
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    format!(
        "Not imported: this title matches multiple existing books: {book_list}. Manual import cannot choose among them. Choose Edit title and author for this row and enter a different main title to create a different book (a different subtitle alone is not enough)."
    )
}

pub fn format_manual_import_defer(members: &[CompleteGroupMember]) -> DeferReason {
    let mut ordered = members.to_vec();
    order_complete_group_members(&mut ordered);
    let reason = match ordered.as_slice() {
        [member] => format_singular_defer(&member.display_title, &member.display_author),
        _ => format_plural_defer(&ordered),
    };
    DeferReason(reason)
}

/// Evaluate the live complete group using `evaluate_match` and the strict
/// guards. Empty groups and a candidate-side text distinction create. A fully
/// text-certain cohort AutoMerges. Any audited distinction or non-certain
/// pair is Review.
pub fn evaluate_captured_group(
    candidate: &CompleteGroupCandidate,
    members: Vec<CompleteGroupMember>,
) -> CompleteGroupEvaluation {
    let identities: Vec<CapturedIdentity> = members
        .iter()
        .map(|member| member.identity.clone())
        .collect();
    let candidate_evidence = WorkIdentityEvidence {
        title: candidate.identity_title.clone(),
        primary_author_id: candidate.primary_author_id,
        routes: Vec::new(),
    };
    let mut pairwise_certain = Vec::new();
    for current in &identities {
        let verdicts = evaluate_match(
            candidate_evidence.clone(),
            captured_as_evidence(current),
            strict_lost_guards(),
            strict_wrong_merge_guards(),
        );
        pairwise_certain.push(authority_certain(&verdicts));
    }
    for (index, left) in identities.iter().enumerate() {
        for right in identities.iter().skip(index + 1) {
            let verdicts = evaluate_match(
                captured_as_evidence(left),
                captured_as_evidence(right),
                strict_lost_guards(),
                strict_wrong_merge_guards(),
            );
            pairwise_certain.push(authority_certain(&verdicts));
        }
    }
    let cohort_has_audited_distinction = identities
        .iter()
        .any(|identity| identity.text_distinction != "common");
    if candidate.text_distinction.is_some() || pairwise_certain.is_empty() {
        CompleteGroupEvaluation::Create
    } else if cohort_has_audited_distinction || !pairwise_certain.iter().all(|certain| *certain) {
        CompleteGroupEvaluation::Review { members }
    } else {
        CompleteGroupEvaluation::AutoMerge { members }
    }
}
