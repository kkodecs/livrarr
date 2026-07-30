use std::collections::HashSet;

use chrono::{DateTime, Duration, Utc};
use livrarr_db::{
    AuthorDb, AuthorLinkClaim, AuthorLinkDb, DbError, GuardedRouteWrite, RenameAuthorDbRequest,
    WorkDb,
};
use livrarr_domain::identity_matching::{
    author_verdict, parse_title, title_verdict, AuthorVerdict,
};
use livrarr_domain::seed::dominant_language;
use livrarr_domain::services::{
    AuthorLinkService, AuthorLinkWorkflow, AuthorProviderGateway, AuthorServiceError,
};
use livrarr_domain::{
    guard_author_route, AgreedAuthorRouteEvidence, Author, AuthorCandidateAlternateNameEvidence,
    AuthorCandidateCatalogState, AuthorCompatibilityProjection, AuthorEvidenceFingerprint,
    AuthorId, AuthorKeyAttempt, AuthorKeyAttemptOutcome, AuthorLinkCandidate,
    AuthorLinkCandidateReason, AuthorLinkCandidateStatus, AuthorLinkCursor, AuthorLinkError,
    AuthorLinkProgress, AuthorLinkProgressState, AuthorLinkProgressUpdate, AuthorLinkReview,
    AuthorLinkState, AuthorLinkTrigger, AuthorNameSource, AuthorNameVariant, AuthorProvider,
    AuthorProviderError, AuthorRoadInput, AuthorRoute, AuthorRouteEvidenceSource,
    AuthorRouteGuardResult, AuthorRouteKey, AuthorSweepProgress, AuthorSweepTickSummary,
    OpenLibraryAuthorCandidate, OpenLibraryAuthorKey, OpenLibraryNameRole,
    RejectedAuthorRouteEvidence, RequestPriority, RouteWriteOutcome, SettledAuthorWork,
    SettledWorkProviderKey, UserId, WorkId,
};
use livrarr_external_data::language::{provider_priority, ProviderPriority};
use tokio_util::sync::CancellationToken;

/// How long a claimed author stays leased to one sweep worker. A cancelled or
/// crashed tick releases its unprocessed claims by letting this expire, which is
/// what keeps an interrupted batch resumable without ever replaying a sibling
/// the previous tick already finished.
const SWEEP_LEASE_MINUTES: i64 = 5;

/// When a parked author is looked at again. Parking means "the evidence on hand
/// cannot answer this", so the useful trigger is new evidence (the migration-078
/// wake-up) and this is only the backstop.
const PARK_RECHECK_HOURS: i64 = 24;

/// When a fully linked author is looked at again.
const LINKED_RECHECK_HOURS: i64 = 24 * 7;

/// First retry delay for a provider key, doubled per prior attempt.
const RETRY_BASE_SECONDS: i64 = 5 * 60;

/// Ceiling on the doubling. Attempt count feeds the delay and nothing else — no
/// number of attempts turns a retryable failure into a terminal one.
const RETRY_CAP_SECONDS: i64 = 6 * 60 * 60;

/// Width of the deterministic per-key retry spread, so a library that failed
/// together does not retry together.
const RETRY_JITTER_SECONDS: i64 = 5 * 60;

/// How many OpenLibrary name-search candidates Tier 2 parks for review.
const TIER2_SEARCH_LIMIT: u32 = 10;

/// Catalog pages read per Tier-2 candidate before its evidence is reported as
/// partial. Corroboration is a review aid, not a full catalogue crawl.
const TIER2_CATALOG_PAGE_BUDGET: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorNameRankModel {
    EnglishOrUndetermined,
    ForeignDominant,
}

const ENGLISH_OR_UNDETERMINED_NAME_RANK: [AuthorNameSource; 8] = [
    AuthorNameSource::User,
    AuthorNameSource::Goodreads,
    AuthorNameSource::Hardcover,
    AuthorNameSource::GoogleBooks,
    AuthorNameSource::OpenLibrary,
    AuthorNameSource::Readarr,
    AuthorNameSource::Import,
    AuthorNameSource::Legacy,
];

const FOREIGN_DOMINANT_NAME_RANK: [AuthorNameSource; 8] = [
    AuthorNameSource::User,
    AuthorNameSource::GoogleBooks,
    AuthorNameSource::Hardcover,
    AuthorNameSource::Goodreads,
    AuthorNameSource::OpenLibrary,
    AuthorNameSource::Readarr,
    AuthorNameSource::Import,
    AuthorNameSource::Legacy,
];

/// Name-source priority for an author's display name, most preferred first.
///
/// A user entry outranks every provider in both models. Readarr, import, and
/// legacy observations sit below provider evidence: a Readarr name is a
/// same-record assertion rather than an independent provider fetch.
pub fn author_name_rank_table(model: AuthorNameRankModel) -> &'static [AuthorNameSource] {
    match model {
        AuthorNameRankModel::EnglishOrUndetermined => &ENGLISH_OR_UNDETERMINED_NAME_RANK,
        AuthorNameRankModel::ForeignDominant => &FOREIGN_DOMINANT_NAME_RANK,
    }
}

/// The display name for an author, chosen from their retained name variants.
///
/// An explicit user selection wins outright. Otherwise the highest-ranked source
/// present under the dominant-language model wins, with OpenLibrary primaries
/// ahead of aliases, and earliest observation then lowest id as stable
/// tie-breakers so equal-rank observations arriving in different batches do not
/// make the display name oscillate. An author with no nonempty variant has no
/// display name.
pub fn choose_author_display_name<'a>(
    variants: &[AuthorNameVariant],
    work_languages: impl Iterator<Item = Option<&'a str>>,
) -> Option<AuthorNameVariant> {
    if let Some(selected) = usable_variants(variants)
        .filter(|variant| variant.user_selected_at.is_some())
        .max_by_key(|variant| (variant.user_selected_at, std::cmp::Reverse(variant.id)))
    {
        return Some(selected.clone());
    }

    let model = match dominant_language(work_languages) {
        Some(language)
            if matches!(
                provider_priority(Some(&language)),
                ProviderPriority::Foreign
            ) =>
        {
            AuthorNameRankModel::ForeignDominant
        }
        _ => AuthorNameRankModel::EnglishOrUndetermined,
    };

    author_name_rank_table(model).iter().find_map(|source| {
        usable_variants(variants)
            .filter(|variant| variant.source == *source)
            .min_by_key(|variant| {
                (
                    open_library_role_rank(variant),
                    variant.observed_at,
                    variant.id,
                )
            })
            .cloned()
    })
}

/// A blank name is not a display name.
fn usable_variants(variants: &[AuthorNameVariant]) -> impl Iterator<Item = &AuthorNameVariant> {
    variants
        .iter()
        .filter(|variant| !variant.name.trim().is_empty())
}

/// OpenLibrary search marks a name as the author's primary or as an alias. A
/// variant carrying no OL role orders after both.
fn open_library_role_rank(variant: &AuthorNameVariant) -> u8 {
    match variant.open_library_role {
        Some(OpenLibraryNameRole::Primary) => 0,
        Some(OpenLibraryNameRole::Alias) => 1,
        None => 2,
    }
}

pub struct AuthorLinkingServiceImpl<D, G>
where
    D: AuthorLinkDb + AuthorDb + WorkDb,
    G: AuthorProviderGateway,
{
    pub db: D,
    pub gateway: G,
}

/// What one `run_author` pass observed, accumulated as it commits each effect.
#[derive(Default)]
struct RoadTally {
    /// Contributor refs any provider returned for this author's settled works.
    /// Zero is what makes Tier 2 eligible: the keys we hold said nothing.
    contributors_seen: u32,
    /// Routes this pass left active (attached, already active, or upgraded).
    active_routes: u32,
    /// An OpenLibrary route is now active, so Tier 2 has nothing to add.
    open_library_route: bool,
    /// Review evidence this pass persisted.
    pending_candidates: bool,
    /// An OpenLibrary key is still owed a retry — the one failure class that
    /// defers Tier 2, because searching by name would answer a question the
    /// key was about to answer properly.
    open_library_retry: bool,
    /// The earliest time any failed key or tier may be tried again.
    earliest_retry: Option<DateTime<Utc>>,
    /// Where an interrupted pass should resume.
    cursor: Option<AuthorLinkCursor>,
    /// The last diagnostic worth showing an operator.
    last_error: Option<String>,
    /// Names that parked instead of linking — the only input the retired 0.90
    /// comparator is still allowed to see.
    parked_names: Vec<String>,
    /// How many times this author's own progress row has already been attempted,
    /// which is the backoff input for a tier-level (rather than key-level)
    /// failure.
    author_attempt_count: u32,
}

impl RoadTally {
    /// Fold one guarded-writer outcome into the tally.
    fn record_route_write(&mut self, outcome: &RouteWriteOutcome, provider: AuthorProvider) {
        match outcome {
            RouteWriteOutcome::Attached(_)
            | RouteWriteOutcome::AlreadyActive(_)
            | RouteWriteOutcome::LegacyProvenanceUpgraded(_) => {
                self.active_routes += 1;
                self.open_library_route |= provider == AuthorProvider::OpenLibrary;
            }
            RouteWriteOutcome::ParkedTombstoned(candidate)
            | RouteWriteOutcome::ParkedLegacyContradiction(candidate)
            | RouteWriteOutcome::ParkedOwnershipCollision(candidate)
            | RouteWriteOutcome::RejectedByNameGuard(candidate) => {
                self.pending_candidates = true;
                self.parked_names.push(candidate.candidate_name.clone());
            }
        }
    }

    /// Remember the soonest retry across every key and tier that asked for one.
    fn schedule_retry(&mut self, at: DateTime<Utc>) {
        self.earliest_retry = Some(match self.earliest_retry {
            Some(existing) if existing <= at => existing,
            _ => at,
        });
    }
}

impl<D, G> AuthorLinkingServiceImpl<D, G>
where
    D: AuthorLinkDb + AuthorDb + WorkDb,
    G: AuthorProviderGateway,
{
    /// Run the one author-linking road for a claimed author.
    ///
    /// Order is the contract: local display-name work first, then the settled
    /// evidence fingerprint decides whether any provider is worth calling, then
    /// Tier 1 walks every provider key on every settled work and attaches every
    /// contributor the name guard agrees with, then Tier 2 parks name-search
    /// candidates for review without ever writing a route.
    pub async fn run_author(
        &self,
        claim: AuthorLinkClaim,
    ) -> Result<AuthorLinkProgressUpdate, AuthorLinkError> {
        self.run_author_tracked(claim)
            .await
            .map(|(update, _)| update)
    }

    /// [`Self::run_author`] plus whether this pass was an unchanged-evidence
    /// skip: the settled evidence is the same as last time and nothing was
    /// runnable, so no provider was called and only scheduling state moved.
    async fn run_author_tracked(
        &self,
        claim: AuthorLinkClaim,
    ) -> Result<(AuthorLinkProgressUpdate, bool), AuthorLinkError> {
        let input = self
            .db
            .load_road_input(claim.clone())
            .await
            .map_err(link_error)?;
        let progress = self
            .db
            .load_progress(claim.user_id, claim.author_id)
            .await
            .map_err(link_error)?;
        let now = Utc::now();

        // Local convergence runs before any network work: a display name is
        // computed from what is already stored, so making the author wait behind
        // a provider queue would strand it for no reason.
        let mut display_name_dirty = input.display_name_dirty;
        if display_name_dirty {
            self.converge_display_name(&claim, &input).await?;
            display_name_dirty = false;
        }

        let live_fingerprint = input.live_fingerprint.clone();
        let evidence_changed = !input
            .evaluated_fingerprint
            .as_ref()
            .is_some_and(|evaluated| same_fingerprint(evaluated, &live_fingerprint));
        let generation = if evidence_changed {
            progress.evidence_generation + 1
        } else {
            progress.evidence_generation
        };
        if evidence_changed {
            // Persisted before the first route or candidate write so every
            // effect of this pass lands in one reviewable generation.
            self.db
                .begin_evidence_generation(claim.clone(), generation)
                .await
                .map_err(link_error)?;
        }

        if input.settled_works.is_empty() {
            // Tier 3. No Confirmed or Provisional work means no key to inherit
            // from and no name worth searching on, so nothing is asked of any
            // provider and the author stays due for a later re-entry.
            let update = AuthorLinkProgressUpdate {
                state: AuthorLinkProgressState::ParkedNoSettledWork,
                tier: Some(3),
                cursor: None,
                evaluated_fingerprint: live_fingerprint,
                evidence_generation: generation,
                next_attempt_at: now + Duration::hours(PARK_RECHECK_HOURS),
                last_error: None,
                display_name_generation: claim.display_name_generation,
                display_name_dirty,
                would_have_linked_at_090: false,
            };
            self.db
                .advance_progress(claim, update.clone())
                .await
                .map_err(link_error)?;
            return Ok((update, !evidence_changed));
        }

        let display_names = associated_names(&input);
        let attempts = self
            .db
            .prepare_key_attempts(
                claim.clone(),
                generation,
                settled_provider_keys(&input.settled_works),
            )
            .await
            .map_err(link_error)?;

        let mut tally = RoadTally {
            author_attempt_count: progress.attempt_count,
            ..RoadTally::default()
        };
        for attempt in &attempts {
            self.run_key_attempt(&claim, attempt, generation, &display_names, now, &mut tally)
                .await?;
        }

        let mut tier = (!attempts.is_empty()).then_some(1u8);
        let has_open_library_route = tally.open_library_route
            || input
                .active_routes
                .iter()
                .any(|route| route.key.provider() == AuthorProvider::OpenLibrary);

        // Tier 2 exists for authors whose keys said nothing. A key that answered
        // — with a contributor to attach, to review, or to leave tombstoned — has
        // already produced the better evidence, and an outstanding OpenLibrary
        // retry is about to. Only a genuinely silent key set falls through here.
        let run_tier2 = !attempts.is_empty()
            && tally.contributors_seen == 0
            && !has_open_library_route
            && !tally.open_library_retry;
        if run_tier2 {
            tier = Some(2);
            self.run_name_search(&claim, &input, generation, &display_names, now, &mut tally)
                .await?;
        }

        let state = if tally.earliest_retry.is_some() {
            AuthorLinkProgressState::RetryableFailure
        } else if tally.pending_candidates {
            AuthorLinkProgressState::NeedsReview
        } else if tally.active_routes > 0 {
            AuthorLinkProgressState::Linked
        } else {
            AuthorLinkProgressState::ParkedNoEvidence
        };

        // The retired 0.90 comparator survives only here: a read-only counter,
        // taken after the real road has already parked, holding no route or
        // candidate handle it could act on.
        let would_have_linked_at_090 =
            matches!(
                state,
                AuthorLinkProgressState::NeedsReview | AuthorLinkProgressState::ParkedNoEvidence
            ) && legacy_guess_metric(&input.author.name, &tally.parked_names);

        let next_attempt_at = match (state, tally.earliest_retry) {
            (AuthorLinkProgressState::RetryableFailure, Some(at)) => at,
            (AuthorLinkProgressState::Linked, _) => now + Duration::hours(LINKED_RECHECK_HOURS),
            _ => now + Duration::hours(PARK_RECHECK_HOURS),
        };

        let update = AuthorLinkProgressUpdate {
            state,
            tier,
            cursor: tally.cursor.clone(),
            evaluated_fingerprint: live_fingerprint,
            evidence_generation: generation,
            next_attempt_at,
            last_error: tally.last_error.clone(),
            display_name_generation: claim.display_name_generation,
            display_name_dirty,
            would_have_linked_at_090,
        };
        self.db
            .advance_progress(claim, update.clone())
            .await
            .map_err(link_error)?;
        Ok((update, !attempts.is_empty()))
    }

    /// One provider key on one settled work: fetch its contributors, guard each
    /// one independently, and persist this key's own outcome.
    ///
    /// Every effect commits before the key is completed and before the next
    /// sibling starts, so a later failure can neither roll back what this key
    /// proved nor make the sweep redo it.
    async fn run_key_attempt(
        &self,
        claim: &AuthorLinkClaim,
        attempt: &AuthorKeyAttempt,
        generation: i64,
        display_names: &[String],
        now: DateTime<Utc>,
        tally: &mut RoadTally,
    ) -> Result<(), AuthorLinkError> {
        let fetched = self
            .gateway
            .fetch_work_authors(
                attempt.provider,
                attempt.work_route.clone(),
                RequestPriority::Low,
            )
            .await;

        let refs = match fetched {
            Ok(refs) => refs,
            Err(error) => {
                let outcome = self.classify_key_failure(attempt, error, now, tally);
                self.db
                    .complete_key_attempt(claim.clone(), attempt.id, outcome)
                    .await
                    .map_err(link_error)?;
                return Ok(());
            }
        };

        let mut rejected = Vec::new();
        for provider_ref in refs {
            tally.contributors_seen += 1;
            let provider = provider_ref.key.provider();
            match guard_author_route(
                display_names,
                provider_ref,
                Some(attempt.work_id),
                AuthorRouteEvidenceSource::Tier1SettledWork,
            ) {
                AuthorRouteGuardResult::Agreed(evidence) => {
                    let written = self
                        .db
                        .apply_guarded_route(GuardedRouteWrite {
                            claim_token: Some(claim.claim_token),
                            author_id: claim.author_id,
                            evidence,
                        })
                        .await
                        .map_err(link_error)?;
                    tally.record_route_write(&written, provider);
                }
                AuthorRouteGuardResult::Rejected(rejection) => {
                    let evidence = rejection.evidence();
                    tally.parked_names.push(evidence.observed_name.clone());
                    rejected.push(AuthorLinkCandidate {
                        id: 0,
                        author_id: claim.author_id,
                        key: evidence.key.clone(),
                        candidate_name: evidence.observed_name.clone(),
                        reason: AuthorLinkCandidateReason::NameGuardFailed,
                        name_verdict: rejection.verdict(),
                        primary_name_verdict: rejection.verdict(),
                        alternate_name_evidence: vec![],
                        top_work_preview: None,
                        // A keyed rejection ran no catalog read, so its catalog
                        // fields stay truthful not-run values rather than
                        // claiming a read that returned nothing.
                        catalog_evidence_state: AuthorCandidateCatalogState::Pending,
                        corroborated_title_count: 0,
                        settled_work_count: 0,
                        previously_removed: false,
                        status: AuthorLinkCandidateStatus::Pending,
                        evidence_generation: generation,
                        observed_at: now,
                    });
                }
            }
        }

        if !rejected.is_empty() {
            self.db
                .record_candidates(claim.clone(), rejected)
                .await
                .map_err(link_error)?;
            tally.pending_candidates = true;
        }

        self.db
            .complete_key_attempt(
                claim.clone(),
                attempt.id,
                AuthorKeyAttemptOutcome::Succeeded,
            )
            .await
            .map_err(link_error)?;
        Ok(())
    }

    /// Turn one provider failure into this key's durable outcome.
    ///
    /// Every class is key-local: a provider this install cannot use, a shape
    /// that moved, or a refusal about this one record never stops a sibling key
    /// and never erases what a sibling already proved.
    fn classify_key_failure(
        &self,
        attempt: &AuthorKeyAttempt,
        error: AuthorProviderError,
        now: DateTime<Utc>,
        tally: &mut RoadTally,
    ) -> AuthorKeyAttemptOutcome {
        match error {
            AuthorProviderError::NotConfigured => {
                tally.last_error = Some(format!("{:?} is not configured", attempt.provider));
                AuthorKeyAttemptOutcome::SkippedNotConfigured
            }
            AuthorProviderError::UnsupportedProvider => {
                let error = format!(
                    "{:?} has no keyed author lookup for work route {}",
                    attempt.provider, attempt.work_route
                );
                tally.last_error = Some(error.clone());
                AuthorKeyAttemptOutcome::SkippedPermanent { error }
            }
            AuthorProviderError::Permanent(error) => {
                tally.last_error = Some(error.clone());
                AuthorKeyAttemptOutcome::SkippedPermanent { error }
            }
            AuthorProviderError::LayoutDrift(error) => {
                tally.last_error = Some(error.clone());
                AuthorKeyAttemptOutcome::ParkedLayoutDrift { error }
            }
            AuthorProviderError::Retryable {
                error,
                retry_not_before,
            } => {
                let next_attempt_at = retry_at(
                    now,
                    attempt.attempt_count,
                    retry_not_before,
                    retry_seed(attempt),
                );
                tally.last_error = Some(error.clone());
                tally.schedule_retry(next_attempt_at);
                tally.cursor = Some(AuthorLinkCursor::Tier1 {
                    key_attempt_id: attempt.id,
                });
                tally.open_library_retry |= attempt.provider == AuthorProvider::OpenLibrary;
                AuthorKeyAttemptOutcome::Retryable {
                    error,
                    next_attempt_at,
                }
            }
        }
    }

    /// Tier 2: OpenLibrary name search, which always parks.
    ///
    /// No candidate here is provider proof that this person wrote these books —
    /// it is a name that looks right. So this never mints the Agree capability
    /// and never reaches the route writer; every result becomes review evidence
    /// with the verdicts and corroboration a person needs to decide.
    async fn run_name_search(
        &self,
        claim: &AuthorLinkClaim,
        input: &AuthorRoadInput,
        generation: i64,
        display_names: &[String],
        now: DateTime<Utc>,
        tally: &mut RoadTally,
    ) -> Result<(), AuthorLinkError> {
        let query = input.author.name.trim().to_string();
        if query.is_empty() {
            return Ok(());
        }

        let found = self
            .gateway
            .search_open_library_authors(query.clone(), TIER2_SEARCH_LIMIT, RequestPriority::Low)
            .await;
        let found = match found {
            Ok(found) => found,
            Err(AuthorProviderError::Retryable {
                error,
                retry_not_before,
            }) => {
                let at = retry_at(
                    now,
                    tally.author_attempt_count,
                    retry_not_before,
                    author_retry_seed(claim, generation),
                );
                tally.last_error = Some(error);
                tally.schedule_retry(at);
                tally.cursor = Some(AuthorLinkCursor::Tier2Search);
                return Ok(());
            }
            Err(other) => {
                tally.last_error = Some(format!("OpenLibrary author search failed: {other:?}"));
                return Ok(());
            }
        };

        let mut summaries: Vec<Tier2Candidate> = found
            .into_iter()
            .map(|candidate| {
                Tier2Candidate::summarize(candidate, display_names, &input.settled_works)
            })
            .collect();
        // top_work is a fetch-priority hint, never corroboration: a candidate
        // whose headline book we already own is worth reading first, and a
        // stable secondary order keeps a resumed pass deterministic.
        summaries.sort_by(|a, b| {
            b.top_work_matches
                .cmp(&a.top_work_matches)
                .then(verdict_rank(a.name_verdict).cmp(&verdict_rank(b.name_verdict)))
                .then(a.route_key.as_str().cmp(b.route_key.as_str()))
        });

        let mut candidates = Vec::with_capacity(summaries.len());
        for summary in summaries {
            let corroboration = self
                .corroborate_catalog(claim, &summary, input, now, generation, tally)
                .await?;
            candidates.push(summary.into_candidate(
                claim.author_id,
                generation,
                now,
                corroboration,
                input.settled_works.len() as u32,
            ));
        }

        if !candidates.is_empty() {
            self.db
                .record_candidates(claim.clone(), candidates)
                .await
                .map_err(link_error)?;
            tally.pending_candidates = true;
        }
        Ok(())
    }

    /// Read a Tier-2 candidate's catalog and count how many of the author's own
    /// settled works it actually contains.
    ///
    /// Only a `Same` title verdict counts, each settled work counts once, and a
    /// failed read is reported as retrying or unavailable — never as a completed
    /// read that found nothing, which would read as evidence against the
    /// candidate rather than an absence of evidence.
    async fn corroborate_catalog(
        &self,
        claim: &AuthorLinkClaim,
        summary: &Tier2Candidate,
        input: &AuthorRoadInput,
        now: DateTime<Utc>,
        generation: i64,
        tally: &mut RoadTally,
    ) -> Result<CatalogCorroboration, AuthorLinkError> {
        let settled_count = input.settled_works.len();
        let mut matched: HashSet<WorkId> = HashSet::new();
        let mut cursor: Option<String> = None;
        let mut pages = 0usize;

        let state = loop {
            let page = self
                .gateway
                .fetch_open_library_catalog_page(
                    summary.route_key.clone(),
                    cursor.clone(),
                    RequestPriority::Low,
                )
                .await;
            let page = match page {
                Ok(page) => page,
                Err(AuthorProviderError::Retryable {
                    error,
                    retry_not_before,
                }) => {
                    let at = retry_at(
                        now,
                        tally.author_attempt_count,
                        retry_not_before,
                        author_retry_seed(claim, generation),
                    );
                    tally.last_error = Some(error);
                    tally.schedule_retry(at);
                    tally.cursor = Some(AuthorLinkCursor::Tier2Catalog {
                        candidate: summary.route_key.clone(),
                        page: cursor,
                    });
                    break AuthorCandidateCatalogState::Retrying;
                }
                Err(other) => {
                    tally.last_error = Some(format!("OpenLibrary catalog page failed: {other:?}"));
                    break AuthorCandidateCatalogState::Unavailable;
                }
            };
            pages += 1;

            for title in &page.titles {
                let parsed = parse_title(title);
                for work in &input.settled_works {
                    if matched.contains(&work.work_id) {
                        continue;
                    }
                    if matches!(
                        title_verdict(&parse_title(&work.title), &parsed),
                        livrarr_domain::identity_matching::TitleVerdict::Same
                    ) {
                        matched.insert(work.work_id);
                    }
                }
            }

            if matched.len() == settled_count {
                break AuthorCandidateCatalogState::Complete;
            }
            match page.next_cursor {
                None => break AuthorCandidateCatalogState::Complete,
                Some(next) if pages < TIER2_CATALOG_PAGE_BUDGET => cursor = Some(next),
                Some(next) => {
                    // Out of page budget with the feed still going: what was
                    // observed stands, and the cursor says where a later pass
                    // would continue.
                    tally.cursor = Some(AuthorLinkCursor::Tier2Catalog {
                        candidate: summary.route_key.clone(),
                        page: Some(next),
                    });
                    break AuthorCandidateCatalogState::Partial;
                }
            }
        };

        Ok(CatalogCorroboration {
            state,
            matched: matched.len() as u32,
        })
    }

    /// Recompute the author's display name from stored variants and commit it
    /// through the one shared rename cascade.
    ///
    /// Provider-free and idempotent: when the ranked choice is already the
    /// stored name, nothing is written.
    async fn converge_display_name(
        &self,
        claim: &AuthorLinkClaim,
        input: &AuthorRoadInput,
    ) -> Result<(), AuthorLinkError> {
        let works = self
            .db
            .list_works_by_author(claim.user_id, claim.author_id)
            .await
            .map_err(link_error)?;
        let Some(chosen) = choose_author_display_name(
            &input.name_variants,
            works.iter().map(|work| work.language.as_deref()),
        ) else {
            return Ok(());
        };
        if chosen.name.trim() == input.author.name.trim() {
            return Ok(());
        }
        self.db
            .rename_author_and_cascade(RenameAuthorDbRequest {
                user_id: claim.user_id,
                author_id: claim.author_id,
                display_name: chosen.name.clone(),
                variant_id: chosen.id,
            })
            .await
            .map_err(link_error)?;
        Ok(())
    }
}

/// One OpenLibrary name-search result with its name evidence summarized against
/// the author's current associated names.
struct Tier2Candidate {
    route_key: OpenLibraryAuthorKey,
    candidate_name: String,
    primary_name_verdict: AuthorVerdict,
    name_verdict: AuthorVerdict,
    alternate_name_evidence: Vec<AuthorCandidateAlternateNameEvidence>,
    top_work_preview: Option<String>,
    top_work_matches: bool,
}

/// What a candidate's catalog read established.
struct CatalogCorroboration {
    state: AuthorCandidateCatalogState,
    matched: u32,
}

impl Tier2Candidate {
    /// Judge the primary name and every canonical-distinct alias through the one
    /// name authority, then summarize by the strongest verdict any of them
    /// reached — an alias agreeing is the same person agreeing.
    fn summarize(
        candidate: OpenLibraryAuthorCandidate,
        display_names: &[String],
        settled_works: &[SettledAuthorWork],
    ) -> Self {
        let primary_name_verdict =
            author_verdict(std::slice::from_ref(&candidate.name), display_names);
        let mut seen: HashSet<String> = HashSet::new();
        seen.insert(livrarr_domain::identity_matching::canonical_author_key(
            &candidate.name,
        ));
        let mut alternate_name_evidence = Vec::new();
        for alternate in &candidate.alternate_names {
            let canonical = livrarr_domain::identity_matching::canonical_author_key(alternate);
            if canonical.is_empty() || !seen.insert(canonical) {
                continue;
            }
            alternate_name_evidence.push(AuthorCandidateAlternateNameEvidence {
                verdict: author_verdict(std::slice::from_ref(alternate), display_names),
                name: alternate.clone(),
            });
        }

        let name_verdict = alternate_name_evidence
            .iter()
            .map(|evidence| evidence.verdict)
            .chain(std::iter::once(primary_name_verdict))
            .min_by_key(|verdict| verdict_rank(*verdict))
            .unwrap_or(primary_name_verdict);

        let top_work_matches = candidate.top_work.as_deref().is_some_and(|top| {
            let parsed = parse_title(top);
            settled_works.iter().any(|work| {
                matches!(
                    title_verdict(&parse_title(&work.title), &parsed),
                    livrarr_domain::identity_matching::TitleVerdict::Same
                )
            })
        });

        Self {
            route_key: candidate.route_key,
            candidate_name: candidate.name,
            primary_name_verdict,
            name_verdict,
            alternate_name_evidence,
            top_work_preview: candidate.top_work,
            top_work_matches,
        }
    }

    /// The parked review row. `corroborated_title_count` is what a catalog read
    /// actually observed, so a retrying or unavailable state reports what is
    /// known so far rather than a computed zero.
    fn into_candidate(
        self,
        author_id: AuthorId,
        generation: i64,
        now: DateTime<Utc>,
        corroboration: CatalogCorroboration,
        settled_work_count: u32,
    ) -> AuthorLinkCandidate {
        AuthorLinkCandidate {
            id: 0,
            author_id,
            key: AuthorRouteKey::OpenLibrary(self.route_key),
            candidate_name: self.candidate_name,
            reason: AuthorLinkCandidateReason::Tier2NameSearch,
            name_verdict: self.name_verdict,
            primary_name_verdict: self.primary_name_verdict,
            alternate_name_evidence: self.alternate_name_evidence,
            top_work_preview: self.top_work_preview,
            catalog_evidence_state: corroboration.state,
            corroborated_title_count: corroboration.matched,
            settled_work_count,
            previously_removed: false,
            status: AuthorLinkCandidateStatus::Pending,
            evidence_generation: generation,
            observed_at: now,
        }
    }
}

/// Verdict strength, strongest first: an abstention is weaker than grey but
/// still more than an outright disagreement.
fn verdict_rank(verdict: AuthorVerdict) -> u8 {
    match verdict {
        AuthorVerdict::Agree => 0,
        AuthorVerdict::Grey => 1,
        AuthorVerdict::Abstain => 2,
        AuthorVerdict::Disagree => 3,
    }
}

/// Two fingerprints describe the same settled evidence.
fn same_fingerprint(a: &AuthorEvidenceFingerprint, b: &AuthorEvidenceFingerprint) -> bool {
    a.settled_work_count == b.settled_work_count
        && a.settled_provider_key_count == b.settled_provider_key_count
        && a.content_hash == b.content_hash
}

/// Every name currently associated with this author — the snapshot the name
/// guard compares a provider's contributor against.
fn associated_names(input: &AuthorRoadInput) -> Vec<String> {
    let mut names = Vec::with_capacity(input.name_variants.len() + 1);
    let mut seen = HashSet::new();
    for name in std::iter::once(input.author.name.clone()).chain(
        input
            .name_variants
            .iter()
            .map(|variant| variant.name.clone()),
    ) {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            continue;
        }
        if seen.insert(trimmed.to_lowercase()) {
            names.push(trimmed.to_string());
        }
    }
    names
}

/// Every canonical provider key on every settled work — the full Tier-1 work
/// list, never the first one that looks usable.
fn settled_provider_keys(works: &[SettledAuthorWork]) -> Vec<SettledWorkProviderKey> {
    let mut keys = Vec::new();
    for work in works {
        for (provider, value) in [
            (AuthorProvider::OpenLibrary, work.ol_key.as_deref()),
            (AuthorProvider::Goodreads, work.gr_key.as_deref()),
            (AuthorProvider::Hardcover, work.hc_key.as_deref()),
        ] {
            if let Some(route) = value.map(str::trim).filter(|value| !value.is_empty()) {
                keys.push(SettledWorkProviderKey {
                    work_id: work.work_id,
                    provider,
                    work_route: route.to_string(),
                });
            }
        }
    }
    keys
}

/// When a failed provider call may be repeated.
///
/// The provider's own `Retry-After` is a floor, never a ceiling: the computed
/// backoff still applies on top of it, and a deterministic per-key offset keeps
/// a library that failed together from retrying in lockstep.
fn retry_at(
    now: DateTime<Utc>,
    attempt_count: u32,
    retry_not_before: Option<DateTime<Utc>>,
    seed: u64,
) -> DateTime<Utc> {
    let backoff = RETRY_BASE_SECONDS
        .saturating_mul(1i64 << attempt_count.min(16))
        .min(RETRY_CAP_SECONDS);
    let earliest = now + Duration::seconds(backoff);
    let base = match retry_not_before {
        Some(floor) if floor > earliest => floor,
        _ => earliest,
    };
    base + Duration::seconds((seed % RETRY_JITTER_SECONDS as u64) as i64)
}

/// A stable offset for one key: the same key always lands in the same place in
/// the retry window, so a restart schedules it identically.
fn retry_seed(attempt: &AuthorKeyAttempt) -> u64 {
    let mut buffer = Vec::new();
    for field in [
        attempt.user_id.to_string(),
        attempt.author_id.to_string(),
        attempt.evidence_generation.to_string(),
        attempt.work_id.to_string(),
        format!("{:?}", attempt.provider),
        attempt.work_route.clone(),
    ] {
        buffer.extend_from_slice(&(field.len() as u64).to_be_bytes());
        buffer.extend_from_slice(field.as_bytes());
    }
    fnv1a_64(&buffer)
}

/// The same stable offset for an author-level tier failure.
fn author_retry_seed(claim: &AuthorLinkClaim, generation: i64) -> u64 {
    let mut buffer = Vec::new();
    for field in [
        claim.user_id.to_string(),
        claim.author_id.to_string(),
        generation.to_string(),
    ] {
        buffer.extend_from_slice(&(field.len() as u64).to_be_bytes());
        buffer.extend_from_slice(field.as_bytes());
    }
    fnv1a_64(&buffer)
}

const FNV_OFFSET_64: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME_64: u64 = 0x0000_0100_0000_01b3;

fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET_64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME_64);
    }
    hash
}

/// The retired `author_similarity >= 0.90` predicate, kept as a shadow counter
/// only.
///
/// It runs after the road has already parked, sees nothing but names, and
/// returns a boolean: there is no route, candidate, or write it could reach. The
/// number it produces answers "how often would the old guesser have linked
/// this?" and nothing else.
fn legacy_guess_metric(author_name: &str, parked_names: &[String]) -> bool {
    parked_names
        .iter()
        .any(|name| livrarr_matching::author_similarity(author_name, name) >= 0.90)
}

/// Map a repository failure onto the domain error the doors and sweep expect.
///
/// A lost claim stays a lost claim: it means another writer owns this author
/// now, so it is surfaced rather than retried into.
fn link_error(error: DbError) -> AuthorLinkError {
    match error {
        DbError::ClaimLost => AuthorLinkError::ClaimLost,
        DbError::NotFound { .. } => AuthorLinkError::NotFound,
        DbError::Constraint { message } => match route_owner_from_message(&message) {
            Some(author_id) => AuthorLinkError::RouteOwnedByOtherAuthor(author_id),
            None => AuthorLinkError::InvalidRoute(message),
        },
        other => AuthorLinkError::Database(other.to_string()),
    }
}

/// The author id in the repository's ownership-collision message, which is the
/// one place that identifies the holder of a contested route.
fn route_owner_from_message(message: &str) -> Option<AuthorId> {
    message
        .strip_prefix("author route is already held by author ")?
        .trim()
        .parse()
        .ok()
}

impl<D, G> AuthorLinkService for AuthorLinkingServiceImpl<D, G>
where
    D: AuthorLinkDb + AuthorDb + WorkDb + Send + Sync,
    G: AuthorProviderGateway + Send + Sync,
{
    async fn list_review(&self, user_id: UserId) -> Result<Vec<AuthorLinkReview>, AuthorLinkError> {
        self.db.list_review(user_id).await.map_err(link_error)
    }

    async fn pick_candidate(
        &self,
        user_id: UserId,
        candidate_id: i64,
    ) -> Result<AuthorRoute, AuthorLinkError> {
        self.db
            .pick_candidate_as_user(user_id, candidate_id)
            .await
            .map_err(link_error)
    }

    /// Attach a route the user explicitly chose.
    ///
    /// A selection is the user's own answer, so it needs no provider proof and
    /// carries none: it is recorded as `UserPicked`, never as evidence a
    /// provider supplied.
    async fn attach_selected_route(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        key: AuthorRouteKey,
    ) -> Result<AuthorRoute, AuthorLinkError> {
        self.db
            .attach_route_as_user(user_id, author_id, key)
            .await
            .map_err(link_error)
    }

    async fn dismiss_candidate(
        &self,
        user_id: UserId,
        candidate_id: i64,
    ) -> Result<(), AuthorLinkError> {
        self.db
            .dismiss_candidate_as_user(user_id, candidate_id)
            .await
            .map_err(link_error)
    }

    /// Remove a route and re-arm the author.
    ///
    /// The removed row stays as the permanent tombstone: automation may see this
    /// key again and must park it rather than put back what the user took away.
    async fn remove_route(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        route_id: i64,
    ) -> Result<(), AuthorLinkError> {
        self.db
            .remove_route_as_user(user_id, author_id, route_id)
            .await
            .map_err(link_error)?;
        self.db
            .ensure_enqueued(user_id, author_id, AuthorLinkTrigger::UserReResolve)
            .await
            .map_err(link_error)
    }

    /// Ask for the author to be looked at again. Returns as soon as the durable
    /// task is due — no provider is awaited on a user's request.
    async fn re_resolve(
        &self,
        user_id: UserId,
        author_id: AuthorId,
    ) -> Result<AuthorLinkProgress, AuthorLinkError> {
        self.db
            .ensure_enqueued(user_id, author_id, AuthorLinkTrigger::UserReResolve)
            .await
            .map_err(link_error)?;
        self.db
            .load_progress(user_id, author_id)
            .await
            .map_err(link_error)
    }

    async fn progress(&self, user_id: UserId) -> Result<AuthorSweepProgress, AuthorLinkError> {
        self.db.sweep_progress(user_id).await.map_err(link_error)
    }
}

impl<D, G> AuthorLinkWorkflow for AuthorLinkingServiceImpl<D, G>
where
    D: AuthorLinkDb + AuthorDb + WorkDb + Send + Sync,
    G: AuthorProviderGateway + Send + Sync,
{
    async fn enqueue(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        trigger: AuthorLinkTrigger,
    ) -> Result<(), AuthorLinkError> {
        self.db
            .ensure_enqueued(user_id, author_id, trigger)
            .await
            .map_err(link_error)
    }

    /// Route evidence a guard already agreed with.
    ///
    /// The opaque capability is the whole argument: holding one is proof the
    /// canonical name authority returned Agree, and this seam cannot be reached
    /// with anything else.
    async fn submit_evidence(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        evidence: AgreedAuthorRouteEvidence,
    ) -> Result<RouteWriteOutcome, AuthorLinkError> {
        // Ownership first: the guarded writer resolves the author's real owner
        // itself, so a caller passing someone else's author id would otherwise
        // write for that owner. This user-scoped read is the check.
        self.db
            .list_active_routes(user_id, author_id, None)
            .await
            .map_err(link_error)?;
        self.db
            .apply_guarded_route(GuardedRouteWrite {
                claim_token: None,
                author_id,
                evidence,
            })
            .await
            .map_err(link_error)
    }

    async fn record_readarr_rejection(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        rejected: RejectedAuthorRouteEvidence,
    ) -> Result<AuthorLinkCandidate, AuthorLinkError> {
        self.db
            .record_readarr_rejection(user_id, author_id, rejected)
            .await
            .map_err(link_error)
    }

    /// One bounded sweep tick.
    ///
    /// Claims are taken in one short transaction and every provider call happens
    /// outside it. One author's failure is counted and the batch continues; a
    /// cancelled tick stops claiming new work and lets its remaining leases
    /// expire, so nothing already committed is replayed or rolled back.
    async fn run_due(
        &self,
        batch_size: u32,
        cancel: CancellationToken,
    ) -> Result<AuthorSweepTickSummary, AuthorLinkError> {
        self.db
            .ensure_missing_progress_rows(batch_size)
            .await
            .map_err(link_error)?;

        let now = Utc::now();
        let claims = self
            .db
            .claim_due(
                now,
                now + Duration::minutes(SWEEP_LEASE_MINUTES),
                batch_size,
            )
            .await
            .map_err(link_error)?;

        let mut summary = AuthorSweepTickSummary {
            claimed: claims.len() as u32,
            evaluated: 0,
            unchanged_fingerprint: 0,
            failed: 0,
        };
        for claim in claims {
            if cancel.is_cancelled() {
                break;
            }
            let author_id = claim.author_id;
            match self.run_author_tracked(claim).await {
                Ok((_, provider_work)) => {
                    summary.evaluated += 1;
                    if !provider_work {
                        summary.unchanged_fingerprint += 1;
                    }
                }
                Err(error) => {
                    tracing::warn!(author_id, ?error, "author-link sweep: author failed");
                    summary.failed += 1;
                }
            }
        }
        Ok(summary)
    }
}

pub struct AuthorResponseAssembler;

impl AuthorResponseAssembler {
    pub async fn route_view(
        &self,
        user_id: UserId,
        author: &Author,
    ) -> Result<
        (
            Vec<AuthorRoute>,
            AuthorLinkState,
            bool,
            AuthorCompatibilityProjection,
        ),
        AuthorServiceError,
    > {
        todo!()
    }
}

#[cfg(test)]
mod production_gateway_composition {
    use super::AuthorLinkingServiceImpl;
    use livrarr_db::test_helpers::create_test_db;
    use livrarr_external_data::live_config::LiveMetadataConfig;
    use livrarr_external_data::{
        AuthorProviderGatewayImpl, GoodreadsClient, HardcoverClient, OpenLibraryClient,
    };
    use livrarr_http::fetcher::HttpFetcherImpl;

    fn metadata_config() -> livrarr_domain::settings::MetadataConfig {
        livrarr_domain::settings::MetadataConfig {
            hardcover_enabled: false,
            hardcover_api_token: None,
            llm_enabled: false,
            llm_provider: None,
            llm_endpoint: None,
            llm_api_key: None,
            llm_model: None,
            audnexus_url: "https://api.audnex.us".to_string(),
            languages: vec!["en".to_string()],
            google_books_api_key: None,
        }
    }

    /// The road runs on the production gateway.
    ///
    /// Server wiring is a later unit, so nothing constructs this pair yet. This
    /// builds it from the real concrete transport the server will hand it, which
    /// is what proves the type actually satisfies the road's bounds rather than
    /// only resembling them.
    #[tokio::test]
    async fn road_is_constructible_with_the_production_gateway() {
        let db = create_test_db().await;
        let fetcher = HttpFetcherImpl::new().expect("production fetcher");
        let http = livrarr_http::HttpClient::builder()
            .user_agent("livrarr-gateway-composition-test")
            .build()
            .expect("goodreads llm client");
        let live_config = LiveMetadataConfig::new(metadata_config());

        let gateway = AuthorProviderGatewayImpl::new(
            OpenLibraryClient::new(fetcher.clone()),
            GoodreadsClient::new(fetcher.clone(), http, "https://www.goodreads.com"),
            HardcoverClient::new(fetcher, live_config),
        );

        let _road: AuthorLinkingServiceImpl<_, AuthorProviderGatewayImpl<HttpFetcherImpl>> =
            AuthorLinkingServiceImpl { db, gateway };
    }
}
