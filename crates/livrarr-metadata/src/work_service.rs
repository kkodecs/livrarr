use livrarr_db::{
    AuthorDb, ConfigDb, CreateWorkDbRequest, EnrichmentRetryDb, GrabDb, LibraryItemDb,
    MergeWorksDbRequest, ProvenanceDb, SetFieldProvenanceRequest, UpdateWorkEnrichmentDbRequest,
    UpdateWorkUserFieldsDbRequest, WorkDb, WorkDbCreate,
};
use livrarr_domain::keyed_mutex::KeyedMutex;
use livrarr_domain::services::*;
use livrarr_domain::*;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

pub struct WorkServiceImpl<D, E, H> {
    pub(crate) db: D,
    enrichment: E,
    http: H,
    data_dir: PathBuf,
    refresh_locks: Arc<KeyedMutex<(UserId, WorkId)>>,
    bulk_refresh_users: Arc<std::sync::Mutex<std::collections::HashSet<i64>>>,
    /// Optional multi-provider identity resolver used by the add-time and
    /// mid-enrichment identity leg (`settle_identity`, REQ-010). `None` skips
    /// that leg (back-compat until the resolver is composed in the server).
    pub(crate) resolver: Option<Arc<crate::english_identity_resolver::LiveEnglishIdentityResolver>>,
    /// REQ-005 (responsiveness): in-memory signal read by `is_enriching` —
    /// true exactly while a `complete_add`/background enrichment run
    /// executes for (user, work). Never persisted: empty after a restart
    /// (D-001), same shape as `refresh_locks` but a signal, not a lock.
    enriching: Arc<std::sync::Mutex<std::collections::HashSet<(UserId, WorkId)>>>,
    /// Identity-edit preview snapshots (single-use intent tokens, r4
    /// §Preview 6): bounded per-user/global, TTL'd, process-local by design —
    /// restart → redo preview. The durable `identity_generation` each entry
    /// carries is the commit staleness authority; this map only frees
    /// capacity. The lock is never held across a provider await.
    preview_snapshots: Arc<std::sync::Mutex<PreviewSnapshotStore>>,
}

/// RAII membership in the `enriching` registry (REQ-005, design §2.5): insert
/// on entry, remove on every exit including a panic unwind — same pattern
/// family as `BulkRefreshGuard`, but a signal (multiple runs may be members
/// at once) rather than mutual exclusion.
struct EnrichingGuard {
    registry: Arc<std::sync::Mutex<std::collections::HashSet<(UserId, WorkId)>>>,
    key: (UserId, WorkId),
}

impl EnrichingGuard {
    fn enter(
        registry: Arc<std::sync::Mutex<std::collections::HashSet<(UserId, WorkId)>>>,
        key: (UserId, WorkId),
    ) -> Self {
        {
            let mut set = registry
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            set.insert(key);
        }
        Self { registry, key }
    }
}

impl Drop for EnrichingGuard {
    fn drop(&mut self) {
        // A panicked peer must not wedge release: take the lock through poison.
        let mut set = self
            .registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        set.remove(&self.key);
    }
}

impl<D, E, H> WorkServiceImpl<D, E, H> {
    pub fn new(db: D, enrichment: E, http: H, data_dir: PathBuf) -> Self {
        let refresh_locks = Arc::new(KeyedMutex::new());
        spawn_refresh_locks_sweeper(&refresh_locks);
        Self {
            db,
            enrichment,
            http,
            data_dir,
            refresh_locks,
            bulk_refresh_users: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
            resolver: None,
            enriching: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
            preview_snapshots: Arc::new(std::sync::Mutex::new(PreviewSnapshotStore::default())),
        }
    }
}

/// D3 #8 / R-5: `KeyedMutex::sweep()` is the backstop for permits `Drop`'s
/// opportunistic per-guard prune skips (only when the map is contended at
/// release) — it existed with zero production callers. This spawns a 300s
/// periodic sweep of `refresh_locks` for the life of the process, sharing
/// ownership via the `Arc` clone captured in the task. A no-op (never
/// panics) when no Tokio runtime is current — `WorkServiceImpl::new` /
/// `without_enrichment` are plain constructors called from many test
/// contexts, and the sweep is a backstop nothing depends on synchronously.
fn spawn_refresh_locks_sweeper(locks: &Arc<KeyedMutex<(UserId, WorkId)>>) {
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        let locks = Arc::clone(locks);
        handle.spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(300));
            loop {
                ticker.tick().await;
                locks.sweep().await;
            }
        });
    }
}

impl<D, E, H> WorkServiceImpl<D, E, H> {
    /// Inject the multi-provider identity resolver used by the add-time and
    /// mid-enrichment identity leg (`settle_identity`, REQ-010).
    pub fn with_resolver(
        mut self,
        resolver: Arc<crate::english_identity_resolver::LiveEnglishIdentityResolver>,
    ) -> Self {
        self.resolver = Some(resolver);
        self
    }
}

impl<D, H> WorkServiceImpl<D, (), H> {
    pub fn without_enrichment(
        db: D,
        http: H,
        data_dir: PathBuf,
    ) -> WorkServiceImpl<D, StubNoEnrichment, H> {
        let refresh_locks = Arc::new(KeyedMutex::new());
        spawn_refresh_locks_sweeper(&refresh_locks);
        WorkServiceImpl {
            db,
            enrichment: StubNoEnrichment,
            http,
            data_dir,
            refresh_locks,
            bulk_refresh_users: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
            resolver: None,
            enriching: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
            preview_snapshots: Arc::new(std::sync::Mutex::new(PreviewSnapshotStore::default())),
        }
    }
}

pub struct StubNoEnrichment;

impl EnrichmentWorkflow for StubNoEnrichment {
    async fn enrich_work(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
        _mode: EnrichmentMode,
        _candidate_id: Option<livrarr_domain::identity::CandidateId>,
        _priority: RequestPriority,
        _freshness: livrarr_domain::Freshness,
    ) -> Result<EnrichmentResult, EnrichmentWorkflowError> {
        Ok(EnrichmentResult {
            identity_not_found: false,
            changed: false,
            // A no-op workflow never dispatches or merges — not an attempt.
            attempted: false,
            enrichment_status: EnrichmentStatus::Unenriched,
            enrichment_source: None,
            work: Work::default(),
            merge_deferred: false,
            provider_outcomes: HashMap::new(),
            cover_resolution: None,
            audiobook_cover_resolution: None,
        })
    }

    async fn reset_for_manual_refresh(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
    ) -> Result<(), EnrichmentWorkflowError> {
        Ok(())
    }

    async fn inject_source_data(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
        _data: livrarr_domain::services::SourceProviderData,
    ) {
        // no-op stub
    }
}

/// The dead-end attempt threshold above which a missing anchor is no longer
/// chased (REQ-009, PO-locked at 3). The background convergence job reads its
/// threshold from `[convergence]` config; the synchronous refresh gate uses
/// this default directly.
pub(crate) const DEAD_END_THRESHOLD: u32 = 3;

/// The hard-anchor types still worth chasing on a work: a `works.*` column that
/// is NULL, holds no pending (fuzzy-guessed) ledger row, and has not reached the
/// dead-end attempt `threshold`. Shared by the refresh gate (Insertion B) and the
/// background convergence loop so both agree on what "still obtainable" means
/// (REQ-006, RE-007).
pub(crate) fn chaseable_anchor_types(
    work: &Work,
    anchors: &[livrarr_domain::identity::WorkIdentityAnchor],
    dead_ends: &[livrarr_domain::identity::AnchorDeadEnd],
    threshold: u32,
) -> Vec<livrarr_domain::identity::AnchorType> {
    use livrarr_domain::identity::{AnchorConfidence, AnchorType};
    [
        (AnchorType::OL_WORK, work.ol_key.is_none()),
        (AnchorType::GR_WORK, work.gr_key.is_none()),
        (AnchorType::HC_WORK, work.hc_key.is_none()),
        (AnchorType::ISBN_13, work.isbn_13.is_none()),
        (AnchorType::ASIN, work.asin.is_none()),
    ]
    .into_iter()
    .filter(|&(anchor_type, missing)| {
        missing
            && !anchors.iter().any(|a| {
                a.anchor_type.as_str() == anchor_type && a.confidence == AnchorConfidence::Pending
            })
            && !dead_ends
                .iter()
                .any(|d| d.anchor_type.as_str() == anchor_type && d.attempt_count >= threshold)
    })
    .map(|(anchor_type, _)| AnchorType::new(anchor_type))
    .collect()
}

/// Map a candidate's provenance to the conflict-attribution source so a raised
/// identity conflict reflects the creation path that produced it (REQ-020, D-017).
fn conflict_source_for(setter: ProvenanceSetter) -> livrarr_domain::identity::ConflictSource {
    use livrarr_domain::identity::ConflictSource;
    match setter {
        ProvenanceSetter::User => ConflictSource::ManualAdd,
        ProvenanceSetter::Import => ConflictSource::ReadarrImport,
        ProvenanceSetter::Imported => ConflictSource::ListImport,
        ProvenanceSetter::AutoAdded => ConflictSource::AuthorMonitor,
        ProvenanceSetter::Provider | ProvenanceSetter::System => ConflictSource::ManualAdd,
    }
}

/// Sanitize a raw harvest into a usable seed and keep it only when at least
/// one identifier (work anchor or edition bridge) survives — the shared zero-
/// network prefix of `resolve_identity` and `resolve_identity_local` (design
/// §2.3). A malformed-only or title/author-only harvest returns `None`.
fn sanitize_seed_with_anchor(
    harvest: livrarr_domain::identity::RawHarvest,
) -> Option<livrarr_domain::identity::WorkSeed> {
    livrarr_domain::identity::WorkSeed::sanitized(harvest)
        .ok()
        .filter(|s| {
            s.ol_key.is_some()
                || s.gr_key.is_some()
                || s.hc_key.is_some()
                || s.isbn_13.is_some()
                || s.asin.is_some()
        })
}

/// Capture a sanitized seed's identifiers into a [`CapturedIdentity`] —
/// shared by `resolve_identity`'s no-resolver Pending arm and
/// `resolve_identity_local` (design §2.3).
fn captured_from_seed(
    seed: &livrarr_domain::identity::WorkSeed,
) -> livrarr_domain::identity::CapturedIdentity {
    livrarr_domain::identity::CapturedIdentity {
        ol_key: seed.ol_key.clone(),
        gr_key: seed.gr_key.clone(),
        hc_key: seed.hc_key.clone(),
        isbn_13: seed.isbn_13.clone(),
        asin: seed.asin.clone(),
        title: seed.title.clone().unwrap_or_default(),
        author_name: seed.author_name.clone().unwrap_or_default(),
        language: seed.language.clone(),
    }
}

impl<D, E, H> WorkServiceImpl<D, E, H>
where
    D: livrarr_domain::services::WorkIdentityRepository + Send + Sync,
{
    /// Conflict preflight + additive anchor merge for a matched/adopted work:
    /// raise an observable conflict for any work-anchor type whose existing
    /// confirmed value differs from the incoming one (REQ-018/020), then fill
    /// the anchor types the existing work lacks (REQ-028, additive only — a
    /// conflicting same-type anchor is never overwritten by the merge).
    async fn preflight_and_merge_anchors(
        &self,
        existing_work_id: livrarr_domain::WorkId,
        incoming: &livrarr_domain::identity::CapturedIdentity,
        source: livrarr_domain::identity::ConflictSource,
    ) -> Result<(), WorkServiceError> {
        let conflicts = self
            .db
            .detect_conflicting_anchors(existing_work_id, incoming, source)
            .await
            .map_err(|e| WorkServiceError::Validation(format!("conflict detection failed: {e}")))?;
        for conflict in conflicts {
            self.db
                .raise_identity_conflict(conflict)
                .await
                .map_err(|e| WorkServiceError::Validation(format!("conflict raise failed: {e}")))?;
        }
        self.db
            .merge_missing_anchors(existing_work_id, incoming)
            .await
            .map_err(|e| WorkServiceError::Validation(format!("anchor merge failed: {e}")))?;
        Ok(())
    }
}

impl<D, E, H> WorkService for WorkServiceImpl<D, E, H>
where
    D: WorkDb
        + WorkDbCreate
        + AuthorDb
        + LibraryItemDb
        + GrabDb
        + ProvenanceDb
        + EnrichmentRetryDb
        + livrarr_db::ProviderRetryStateDb
        + ConfigDb
        + livrarr_db::SeriesDb
        + livrarr_db::HistoryDb
        + livrarr_db::AuthorLinkDb
        + livrarr_domain::services::WorkIdentityRepository
        + Send
        + Sync,
    E: EnrichmentWorkflow + Send + Sync,
    H: HttpFetcher + Clone + Send + Sync + 'static,
{
    async fn add(
        &self,
        user_id: UserId,
        candidate: livrarr_domain::identity::WorkCandidate,
    ) -> Result<AddWorkResult, WorkServiceError> {
        // add() keeps its name, signature, and synchronous semantics for
        // every existing caller (batch doors, list import, Readarr, monitors)
        // — the implementation is now add_fast plus an awaited complete_add
        // over the work it just created: one pipeline, two entry shapes
        // (design §2.8). Capture what complete_add needs before add_fast
        // consumes the candidate.
        let source_provider_data = candidate.source_provider_data.clone();
        let candidate_id = candidate.candidate_id.clone();
        let identity_setter = candidate
            .provenance_setter
            .unwrap_or(ProvenanceSetter::User);
        let identity_source = conflict_source_for(identity_setter);
        let identity_mode = match identity_setter {
            ProvenanceSetter::Import | ProvenanceSetter::Imported => {
                livrarr_domain::identity::IdentityMode::Background
            }
            _ => livrarr_domain::identity::IdentityMode::Interactive,
        };

        let result = self.add_fast(user_id, candidate).await?;
        if !result.created {
            return Ok(result);
        }

        let work_id = result.work.id;
        self.complete_add(
            user_id,
            work_id,
            source_provider_data,
            candidate_id,
            identity_mode,
            identity_source,
        )
        .await;

        // Re-read + rebuild the result the way finish_created_work did: a
        // synchronous caller's enrichment_status must reflect post-enrichment
        // state, and the cover may have moved if the cover write gate
        // accepted a better image during complete_add.
        let work = self
            .db
            .get_work(user_id, work_id)
            .await
            .unwrap_or(result.work);
        let covers_dir = self.data_dir.join("covers").join(user_id.to_string());
        let cover_mtime = crate::cover::cover_file_mtime(&covers_dir, work.id);
        let audiobook_cover_mtime = crate::cover::audiobook_cover_file_mtime(&covers_dir, work.id);
        let enrichment_status = work.enrichment_status;
        Ok(AddWorkResult {
            work,
            created: true,
            author_created: result.author_created,
            author_id: result.author_id,
            messages: result.messages,
            cover_mtime,
            audiobook_cover_mtime,
            enrichment_status,
        })
    }

    async fn resolve_identity(
        &self,
        user_id: UserId,
        harvest: livrarr_domain::identity::RawHarvest,
        tier: livrarr_domain::identity::LatencyTier,
    ) -> Result<livrarr_domain::identity::ResolvedIdentity, WorkServiceError> {
        use livrarr_domain::identity::{
            IdentityState, PendingReason, Resolution, ResolvedIdentity,
        };

        // Preserve any language the door already knew, before sanitizing
        // consumes the harvest.
        let harvest_language = harvest.language.clone();

        // Sanitize at the boundary: normalize keys, drop malformed ones, and only
        // resolve when a real anchor survives — identical to the Add-Work handler,
        // now the shared path for every door (P1 convergence).
        let Some(seed) = sanitize_seed_with_anchor(harvest) else {
            // No usable anchor — a fuzzy title/author seed. Pending; enrichment or a
            // later pass converges it.
            return Ok(ResolvedIdentity {
                identity: IdentityState::Pending {
                    reason: PendingReason::NoCandidates,
                    seed_anchors: None,
                    top_candidates: vec![],
                },
                candidate_id: None,
                language: harvest_language,
                conflict: None,
            });
        };

        let Some(resolver) = self.resolver.clone() else {
            // No resolver wired (some headless/test configs): keep the anchors as a
            // Pending seed so a background pass can converge — never fabricate a
            // Confirmed badge from un-corroborated keys.
            let language = seed.language.clone();
            return Ok(ResolvedIdentity {
                identity: IdentityState::Pending {
                    reason: PendingReason::NoCandidates,
                    seed_anchors: Some(captured_from_seed(&seed)),
                    top_candidates: vec![],
                },
                candidate_id: None,
                language,
                conflict: None,
            });
        };

        use livrarr_domain::services::IdentityResolver;
        let resolution = resolver
            .resolve(user_id, &seed, tier)
            .await
            .map_err(|e| WorkServiceError::Validation(format!("identity resolve: {e}")))?;

        Ok(match resolution {
            Resolution::Resolved {
                identity,
                method,
                candidate_id,
                ..
            } => ResolvedIdentity {
                language: identity.language.clone(),
                identity: IdentityState::Confirmed {
                    anchors: identity,
                    method,
                    score: None,
                },
                candidate_id: Some(candidate_id),
                conflict: None,
            },
            Resolution::NeedsConfirmation { candidates } => ResolvedIdentity {
                identity: IdentityState::Pending {
                    reason: PendingReason::LowConfidence,
                    seed_anchors: None,
                    top_candidates: candidates,
                },
                candidate_id: None,
                language: None,
                conflict: None,
            },
            Resolution::Unresolved {
                captured,
                reason,
                candidate_id,
                ..
            } => ResolvedIdentity {
                language: captured.language.clone(),
                identity: IdentityState::Pending {
                    reason,
                    seed_anchors: Some(captured),
                    top_candidates: vec![],
                },
                candidate_id,
                conflict: None,
            },
            Resolution::Conflict {
                conflict, captured, ..
            } => ResolvedIdentity {
                language: captured.language.clone(),
                identity: IdentityState::Pending {
                    reason: PendingReason::LowConfidence,
                    seed_anchors: Some(captured),
                    top_candidates: vec![],
                },
                candidate_id: None,
                conflict: Some(conflict),
            },
        })
    }

    fn resolve_identity_local(
        &self,
        harvest: livrarr_domain::identity::RawHarvest,
    ) -> Result<livrarr_domain::identity::ResolvedIdentity, WorkServiceError> {
        use livrarr_domain::identity::{
            IdentityMethod, IdentityState, PendingReason, ResolvedIdentity,
        };

        // Preserve any language the door already knew, before sanitizing
        // consumes the harvest — mirrors resolve_identity's anchorless arm.
        let harvest_language = harvest.language.clone();

        let Some(seed) = sanitize_seed_with_anchor(harvest) else {
            return Ok(ResolvedIdentity {
                identity: IdentityState::Pending {
                    reason: PendingReason::NoCandidates,
                    seed_anchors: None,
                    top_candidates: vec![],
                },
                candidate_id: None,
                language: harvest_language,
                conflict: None,
            });
        };

        let language = seed.language.clone();
        let has_work_anchor =
            seed.ol_key.is_some() || seed.gr_key.is_some() || seed.hc_key.is_some();
        let captured = captured_from_seed(&seed);

        // D-013 derivation, done locally instead of over the network: a work
        // anchor (ol/gr/hc key) already on the seed is a user-confirmed pick
        // (search result, GR link) — Confirmed. Bridge-only (isbn/asin) is
        // Pending with the seed captured; the Provisional badge derives at
        // create from these anchors exactly as today's derived_identity
        // write. `conflict` and a resolved `candidate_id` are never produced
        // here (D-002) — only the network-capable resolve_identity raises
        // those.
        let identity = if has_work_anchor {
            IdentityState::Confirmed {
                anchors: captured,
                method: IdentityMethod::UserSelected,
                score: None,
            }
        } else {
            IdentityState::Pending {
                reason: PendingReason::NoCandidates,
                seed_anchors: Some(captured),
                top_candidates: vec![],
            }
        };

        Ok(ResolvedIdentity {
            identity,
            candidate_id: None,
            language,
            conflict: None,
        })
    }

    async fn add_fast(
        &self,
        user_id: UserId,
        candidate: livrarr_domain::identity::WorkCandidate,
    ) -> Result<AddWorkResult, WorkServiceError> {
        use livrarr_domain::identity::{AnchorSetter, AnchorType, IdentityState};
        use livrarr_domain::identity_matching::{
            author_verdict, parse_title, title_verdict, AuthorVerdict, TitleVerdict,
        };

        let cleaned_title = crate::title_cleanup::clean_title(&candidate.fields.title);
        if cleaned_title.is_empty() {
            return Err(WorkServiceError::Validation(
                "title must not be empty".into(),
            ));
        }
        let cleaned_author = crate::title_cleanup::clean_author(&candidate.fields.author_name);
        // REQ-014: the stored identity key and every add-time lookup that
        // compares against it derive from the same identity_key recipe.
        let (normalized_title, normalized_author) =
            livrarr_domain::identity_matching::identity_key(&cleaned_title, &cleaned_author);

        // The persisted identity badge derived from the candidate's anchors
        // (REQ-014/016, D-013) — written at create and used to gate enrichment.
        let derived_identity = candidate.identity.derived_identity_status();
        // The creation door's label (REQ-001), copied out before candidate
        // fields move below; stamped only by the seed_* constructors.
        let add_source = candidate.add_source;

        // The originating door's identity patience (REQ-005) + conflict
        // attribution (REQ-020), threaded to the one identity road through the
        // chokepoint (ensure_identity_and_enrichment / settle_identity).
        // Spawned/batch import doors resolve in Background; a person-facing add
        // resolves Interactive. Author-monitor seeds a hard key and never reaches
        // the anchorless leg (the RE-009 exception).
        let identity_setter = candidate
            .provenance_setter
            .unwrap_or(ProvenanceSetter::User);
        let identity_source = conflict_source_for(identity_setter);
        let identity_mode = match identity_setter {
            ProvenanceSetter::Import | ProvenanceSetter::Imported => {
                livrarr_domain::identity::IdentityMode::Background
            }
            _ => livrarr_domain::identity::IdentityMode::Interactive,
        };

        match &candidate.identity {
            IdentityState::Confirmed { anchors, .. } => {
                // Step 1: anchor match over the work-anchor types the candidate
                // carries (ol/gr/hc). Bridges (isbn/asin) are edition-level and
                // are not used to dedup works here (bridge-anchor policy).
                let mut anchor_match: Option<livrarr_domain::WorkId> = None;
                for (anchor_ty, value) in [
                    (AnchorType::OL_WORK, anchors.ol_key.as_deref()),
                    (AnchorType::GR_WORK, anchors.gr_key.as_deref()),
                    (AnchorType::HC_WORK, anchors.hc_key.as_deref()),
                ] {
                    let Some(value) = value.filter(|v| !v.is_empty()) else {
                        continue;
                    };
                    if let Ok(Some(existing_id)) = self
                        .db
                        .find_work_by_anchor(user_id, &AnchorType::new(anchor_ty), value)
                        .await
                    {
                        anchor_match = Some(existing_id);
                        break;
                    }
                }
                if let Some(existing_id) = anchor_match {
                    // Conflict preflight + additive anchor merge BEFORE returning
                    // the matched work (REQ-018/020/028).
                    let setter = candidate
                        .provenance_setter
                        .unwrap_or(ProvenanceSetter::User);
                    self.preflight_and_merge_anchors(
                        existing_id,
                        anchors,
                        conflict_source_for(setter),
                    )
                    .await?;
                    let work = self
                        .db
                        .get_work(user_id, existing_id)
                        .await
                        .map_err(WorkServiceError::Db)?;
                    // REQ-010: the matched work takes the same identity +
                    // enrichment road as every other add outcome (the
                    // existing-work doors previously bypassed it).
                    let (enrichment_status, _identity_not_found) = self
                        .ensure_identity_and_enrichment(
                            user_id,
                            work.id,
                            candidate.source_provider_data,
                            None,
                            identity_mode,
                            identity_source,
                        )
                        .await;
                    let work = self.db.get_work(user_id, work.id).await.unwrap_or(work);
                    return Ok(AddWorkResult {
                        work,
                        created: false,
                        author_created: false,
                        author_id: None,
                        messages: vec![],
                        cover_mtime: None,
                        audiobook_cover_mtime: None,
                        enrichment_status,
                    });
                }

                // Step 3: REQ-005 adopt path — existing ol-key-less work with
                // same normalized identity absorbs the incoming confirmed key.
                // REQ-014/ST-04: both sides of the lookup derive from the SAME
                // identity_key recipe on the SAME cleaned inputs — the query
                // values below ARE the stored-key values this candidate would
                // write (identity_key over the clean_title/clean_author
                // output, computed once above). The old code passed the RAW
                // candidate fields through a weak trim+lowercase, so a
                // junk-tailed or accented incoming title could never adopt.
                if let Some(existing) = self
                    .db
                    .find_normalized_match_no_anchor_for_user(
                        user_id,
                        &normalized_title,
                        &normalized_author,
                    )
                    .await
                    .map_err(WorkServiceError::Db)?
                {
                    // Adopt: an anchorless normalized match absorbs the incoming
                    // anchors (REQ-028); the preflight raises a conflict if the
                    // existing work already holds a different confirmed anchor
                    // (REQ-018/020).
                    let setter = candidate
                        .provenance_setter
                        .unwrap_or(ProvenanceSetter::User);
                    self.preflight_and_merge_anchors(
                        existing.id,
                        anchors,
                        conflict_source_for(setter),
                    )
                    .await?;
                    let existing = self
                        .db
                        .get_work(user_id, existing.id)
                        .await
                        .map_err(WorkServiceError::Db)?;
                    // REQ-010: the adopted work takes the same identity +
                    // enrichment road as every other add outcome.
                    let (enrichment_status, _identity_not_found) = self
                        .ensure_identity_and_enrichment(
                            user_id,
                            existing.id,
                            candidate.source_provider_data.clone(),
                            None,
                            identity_mode,
                            identity_source,
                        )
                        .await;
                    let work = self
                        .db
                        .get_work(user_id, existing.id)
                        .await
                        .unwrap_or(existing);
                    return Ok(AddWorkResult {
                        work,
                        created: false,
                        author_created: false,
                        author_id: None,
                        messages: vec![],
                        cover_mtime: None,
                        audiobook_cover_mtime: None,
                        enrichment_status,
                    });
                }

                // Normalized-match dedup with step 3e race-loser detection.
                let existing = self
                    .db
                    .find_by_normalized_match(user_id, &normalized_title, &normalized_author)
                    .await
                    .map_err(WorkServiceError::Db)?;
                if let Some(work) = existing.into_iter().next() {
                    // Step 3e (now wired, REQ-020): conflict preflight + additive
                    // anchor merge on the normalized-identity match — replaces the
                    // former warn-only TODO. A differing confirmed work anchor
                    // (ol/gr/hc) raises an observable conflict (REQ-018).
                    let setter = candidate
                        .provenance_setter
                        .unwrap_or(ProvenanceSetter::User);
                    self.preflight_and_merge_anchors(work.id, anchors, conflict_source_for(setter))
                        .await?;
                    let work = self
                        .db
                        .get_work(user_id, work.id)
                        .await
                        .map_err(WorkServiceError::Db)?;
                    // REQ-010: the deduped work takes the same identity +
                    // enrichment road as every other add outcome.
                    let (enrichment_status, _identity_not_found) = self
                        .ensure_identity_and_enrichment(
                            user_id,
                            work.id,
                            candidate.source_provider_data.clone(),
                            None,
                            identity_mode,
                            identity_source,
                        )
                        .await;
                    let work = self.db.get_work(user_id, work.id).await.unwrap_or(work);
                    return Ok(AddWorkResult {
                        work,
                        created: false,
                        author_created: false,
                        author_id: None,
                        messages: vec![],
                        cover_mtime: None,
                        audiobook_cover_mtime: None,
                        enrichment_status,
                    });
                }

                let (author_created, author_id) = self
                    .find_or_create_author(
                        user_id,
                        &cleaned_author,
                        candidate.fields.author_ol_key.as_deref(),
                    )
                    .await?;

                let setter = candidate
                    .provenance_setter
                    .unwrap_or(ProvenanceSetter::User);
                let anchor_setter = match setter {
                    ProvenanceSetter::User => AnchorSetter::User,
                    ProvenanceSetter::Import => AnchorSetter::Import,
                    _ => AnchorSetter::AutoSearch,
                };

                let is_user_initiated = candidate.source_provider_data.is_none();
                let (work, actually_created) = self
                    .db
                    .create_work(CreateWorkDbRequest {
                        user_id,
                        title: cleaned_title,
                        author_name: cleaned_author,
                        normalized_title,
                        normalized_author,
                        author_id,
                        ol_key: None,
                        gr_key: candidate
                            .identity
                            .seed_or_confirmed_anchors()
                            .and_then(|a| a.gr_key.clone()),
                        year: candidate.fields.year,
                        cover_url: candidate.fields.cover_url.clone(),
                        language: Some(livrarr_domain::normalize_language(
                            &candidate.fields.language,
                        )),
                        series_name: candidate.fields.series_name.clone(),
                        series_position: candidate.fields.series_position,
                        monitor_ebook: candidate.monitor_ebook.unwrap_or(true),
                        monitor_audiobook: candidate.monitor_audiobook.unwrap_or(true),
                        import_id: candidate.import_id.clone(),
                        series_id: candidate.series_id,
                        isbn_13: candidate
                            .identity
                            .seed_or_confirmed_anchors()
                            .and_then(|a| a.isbn_13.clone()),
                        asin: candidate
                            .identity
                            .seed_or_confirmed_anchors()
                            .and_then(|a| a.asin.clone()),
                        description: candidate.fields.description.clone(),
                        source_provider_json: None,
                        cover_manual: candidate.cover_manual,
                    })
                    .await
                    .map_err(WorkServiceError::Db)?;

                if !actually_created {
                    return self
                        .handle_race_loser(
                            user_id,
                            work,
                            author_created,
                            author_id,
                            candidate.source_provider_data,
                            identity_mode,
                            identity_source,
                        )
                        .await;
                }

                // Persist every work-anchor + bridge the candidate carries as a
                // confirmed anchor row (REQ-001/003) — not only the OL key. A
                // resolving-id candidate may have no ol_key (e.g. Readarr gr+isbn).
                for (anchor_ty, value) in [
                    (AnchorType::OL_WORK, anchors.ol_key.as_deref()),
                    (AnchorType::GR_WORK, anchors.gr_key.as_deref()),
                    (AnchorType::HC_WORK, anchors.hc_key.as_deref()),
                    (AnchorType::ISBN_13, anchors.isbn_13.as_deref()),
                    (AnchorType::ASIN, anchors.asin.as_deref()),
                ] {
                    if let Some(value) = value.filter(|v| !v.is_empty()) {
                        self.db
                            .confirm_anchor(
                                work.id,
                                AnchorType::new(anchor_ty),
                                value,
                                anchor_setter,
                            )
                            .await
                            .map_err(|e| {
                                WorkServiceError::Validation(format!("anchor write failed: {e}"))
                            })?;
                    }
                }

                // Reload so add-time provenance sees the anchor-synced identifier
                // columns (ol/gr/hc/isbn/asin), matching the prior
                // create_work_with_anchor reload semantics.
                let work = self
                    .db
                    .get_work(user_id, work.id)
                    .await
                    .map_err(WorkServiceError::Db)?;
                write_addtime_provenance(&self.db, user_id, &work, setter).await;

                self.finish_created_work_fast(
                    user_id,
                    work,
                    author_created,
                    author_id,
                    derived_identity,
                    is_user_initiated,
                    add_source,
                )
                .await
            }
            IdentityState::Pending { .. } => {
                // NEW — verdict-gated local bridge dedup (design §2.4/D-007):
                // a bridge-only candidate (isbn/asin, no work anchor) is
                // checked against the user's works sharing that bridge BEFORE
                // the normalized-title dedup below. Zero network — a local DB
                // lookup gated by the one matching authority
                // (identity_matching); the bridge key is a lookup hint, never
                // merge evidence on its own (bridge-anchor policy stands).
                if let IdentityState::Pending {
                    seed_anchors: Some(anchors),
                    ..
                } = &candidate.identity
                {
                    let bridge_only = anchors.ol_key.is_none()
                        && anchors.gr_key.is_none()
                        && anchors.hc_key.is_none()
                        && (anchors.isbn_13.is_some() || anchors.asin.is_some());
                    if bridge_only {
                        let hits = self
                            .db
                            .find_works_by_bridge(
                                user_id,
                                anchors.isbn_13.as_deref(),
                                anchors.asin.as_deref(),
                            )
                            .await
                            .map_err(WorkServiceError::Db)?;
                        let candidate_title = parse_title(&cleaned_title);
                        // Multi-bridge abstention (identity-edit r4, required
                        // by 076): same-user bridge sharing is now legal, so
                        // a bridge can legitimately match several works.
                        // Collect ALL verdict-eligible hits — exactly one
                        // adopts (unchanged); two or more abstain from bridge
                        // dedup and fall through to the existing
                        // normalized-title dedup/create below.
                        let eligible: Vec<&Work> = hits
                            .iter()
                            .filter(|hit| {
                                let hit_title = parse_title(&hit.title);
                                let title = title_verdict(&candidate_title, &hit_title);
                                let author = author_verdict(
                                    std::slice::from_ref(&cleaned_author),
                                    std::slice::from_ref(&hit.author_name),
                                );
                                matches!(title, TitleVerdict::Same | TitleVerdict::Grey { .. })
                                    && !matches!(author, AuthorVerdict::Disagree)
                            })
                            .collect();
                        if let [only] = eligible.as_slice() {
                            let setter = candidate
                                .provenance_setter
                                .unwrap_or(ProvenanceSetter::User);
                            self.preflight_and_merge_anchors(
                                only.id,
                                anchors,
                                conflict_source_for(setter),
                            )
                            .await?;
                            let work = self
                                .db
                                .get_work(user_id, only.id)
                                .await
                                .map_err(WorkServiceError::Db)?;
                            let enrichment_status = work.enrichment_status;
                            return Ok(AddWorkResult {
                                work,
                                created: false,
                                author_created: false,
                                author_id: None,
                                messages: vec![],
                                cover_mtime: None,
                                audiobook_cover_mtime: None,
                                enrichment_status,
                            });
                        }
                    }
                }

                if let Some((work, enrichment_status)) = self
                    .try_dedup_by_normalized(
                        user_id,
                        &normalized_title,
                        &normalized_author,
                        &candidate.source_provider_data,
                    )
                    .await?
                {
                    return Ok(AddWorkResult {
                        work,
                        created: false,
                        author_created: false,
                        author_id: None,
                        messages: vec![],
                        cover_mtime: None,
                        audiobook_cover_mtime: None,
                        enrichment_status,
                    });
                }

                let (author_created, author_id) = self
                    .find_or_create_author(
                        user_id,
                        &cleaned_author,
                        candidate.fields.author_ol_key.as_deref(),
                    )
                    .await?;

                let is_user_initiated = candidate.source_provider_data.is_none();
                let (work, actually_created) = self
                    .db
                    .create_work(CreateWorkDbRequest {
                        user_id,
                        title: cleaned_title,
                        author_name: cleaned_author,
                        normalized_title,
                        normalized_author,
                        author_id,
                        ol_key: None,
                        gr_key: candidate
                            .identity
                            .seed_or_confirmed_anchors()
                            .and_then(|a| a.gr_key.clone()),
                        year: candidate.fields.year,
                        cover_url: candidate.fields.cover_url.clone(),
                        language: Some(livrarr_domain::normalize_language(
                            &candidate.fields.language,
                        )),
                        series_name: candidate.fields.series_name.clone(),
                        series_position: candidate.fields.series_position,
                        monitor_ebook: candidate.monitor_ebook.unwrap_or(true),
                        monitor_audiobook: candidate.monitor_audiobook.unwrap_or(true),
                        import_id: candidate.import_id.clone(),
                        series_id: candidate.series_id,
                        isbn_13: candidate
                            .identity
                            .seed_or_confirmed_anchors()
                            .and_then(|a| a.isbn_13.clone()),
                        asin: candidate
                            .identity
                            .seed_or_confirmed_anchors()
                            .and_then(|a| a.asin.clone()),
                        description: candidate.fields.description.clone(),
                        source_provider_json: None,
                        cover_manual: candidate.cover_manual,
                    })
                    .await
                    .map_err(WorkServiceError::Db)?;

                if !actually_created {
                    return self
                        .handle_race_loser(
                            user_id,
                            work,
                            author_created,
                            author_id,
                            candidate.source_provider_data,
                            identity_mode,
                            identity_source,
                        )
                        .await;
                }

                let setter = candidate
                    .provenance_setter
                    .unwrap_or(ProvenanceSetter::User);
                write_addtime_provenance(&self.db, user_id, &work, setter).await;

                if let IdentityState::Pending { reason, .. } = &candidate.identity {
                    let anchor_setter = match setter {
                        ProvenanceSetter::User => AnchorSetter::User,
                        ProvenanceSetter::Import => AnchorSetter::Import,
                        _ => AnchorSetter::AutoSearch,
                    };
                    self.db
                        .set_identity_pending(work.id, *reason, anchor_setter)
                        .await
                        .map_err(|e| {
                            WorkServiceError::Validation(format!(
                                "set_identity_pending failed: {e}"
                            ))
                        })?;
                }

                // A Pending identity reaches ensure_identity_and_enrichment via
                // complete_add: the add-time identity leg may resolve it
                // (REQ-010); a still-held identity skips the fan-out there.
                // Display/cover (best-in-hand) is materialized either way.
                self.finish_created_work_fast(
                    user_id,
                    work,
                    author_created,
                    author_id,
                    derived_identity,
                    is_user_initiated,
                    add_source,
                )
                .await
            }
        }
    }

    async fn complete_add(
        &self,
        user_id: UserId,
        work_id: WorkId,
        source_provider_data: Option<SourceProviderData>,
        candidate_id: Option<livrarr_domain::identity::CandidateId>,
        mode: livrarr_domain::identity::IdentityMode,
        source: livrarr_domain::identity::ConflictSource,
    ) {
        use livrarr_domain::IdentityStatus;

        // RAII: visible in the registry for the whole call, including a
        // panic unwind (Drop always runs) — is_enriching reads true for the
        // duration (REQ-005).
        let _guard = EnrichingGuard::enter(self.enriching.clone(), (user_id, work_id));

        // Bridge-only completion (REQ-004): a Pending work that CARRIES
        // anchors (e.g. an isbn-only Google Books pick) gets the same
        // identity chase the refresh door runs — settle via the one identity
        // road, gated by chaseable_anchor_types, so identity resolves inside
        // the enriching-signal window instead of waiting for the top-up
        // refresh. Anchorless works are deliberately excluded here:
        // ensure_identity_and_enrichment runs its own settle leg for those,
        // and chasing both places would fan out twice.
        if let Ok(work) = self.db.get_work(user_id, work_id).await {
            let anchorless = work.ol_key.is_none()
                && work.gr_key.is_none()
                && work.hc_key.is_none()
                && work.isbn_13.is_none()
                && work.asin.is_none();
            if work.identity_status == IdentityStatus::Pending && !anchorless {
                if let Some(resolver) = self.resolver.as_ref() {
                    let anchors = self.db.list_anchors(work.id).await.unwrap_or_default();
                    let dead_ends = self
                        .db
                        .list_anchor_dead_ends(work.id)
                        .await
                        .unwrap_or_default();
                    if !chaseable_anchor_types(&work, &anchors, &dead_ends, DEAD_END_THRESHOLD)
                        .is_empty()
                    {
                        if let Err(e) = crate::async_resolver::settle_identity(
                            resolver.as_ref(),
                            &self.db,
                            user_id,
                            &work,
                            mode,
                            source,
                        )
                        .await
                        {
                            tracing::warn!(work_id, "complete_add identity settle failed: {e}");
                        }
                    }
                }
            }
        }

        // The delayed NotFound conclusion below is DECIDED by the enrichment call that
        // follows, so the generation it claims must be observed HERE, before the wait.
        // Reading it afterwards claims a generation the decision never saw: the CAS then
        // succeeds against a user edit that landed mid-flight and stamps the stale
        // conclusion over the correction. Guarded in form, unguarded in fact.
        //
        // Observed after the bridge-only settle leg above on purpose — that leg is a
        // legitimate identity writer whose own writes are already claimed, so its bump
        // must not invalidate this conclusion.
        let generation_before_enrichment = self
            .db
            .get_work_with_identity_generation(user_id, work_id)
            .await
            .map(|(_, generation)| generation)
            .inspect_err(|e| {
                tracing::warn!(
                    work_id,
                    "complete_add: pre-enrichment generation read failed: {e}"
                );
            })
            .ok();

        let (enrichment_status, identity_not_found) = self
            .ensure_identity_and_enrichment(
                user_id,
                work_id,
                source_provider_data,
                candidate_id,
                mode,
                source,
            )
            .await;

        // Seam-2 (REQ-002/D-013): enrichment SIGNALS that it could not verify
        // the work's identity — the caller writes the badge, mirroring the
        // synchronous add path's write. The fresh read is defense-in-depth:
        // the enrichment gate already refuses to run for a parked
        // (Conflict/NeedsReview) work, so identity_not_found should never
        // coincide with a parked status — this guarantees the invariant
        // structurally rather than relying on the gate alone.
        if identity_not_found {
            // Delayed NotFound conclusion (identity-edit r4 §Writer coverage): the
            // completion claims the PRE-wait generation captured above, so a user edit
            // landing during enrichment supersedes this stale conclusion instead of
            // being overwritten by it. The work re-read below is only for the
            // already-parked check — its generation is deliberately discarded.
            match self
                .db
                .get_work_with_identity_generation(user_id, work_id)
                .await
            {
                Ok((work, _post_wait_generation)) => {
                    let already_parked = matches!(
                        work.identity_status,
                        IdentityStatus::Conflict | IdentityStatus::NeedsReview
                    );
                    // No pre-wait generation means there is nothing legitimate to claim
                    // against, so the conclusion is dropped rather than written blind.
                    if let (false, Some(generation)) =
                        (already_parked, generation_before_enrichment)
                    {
                        match self
                            .db
                            .complete_anchors(
                                work_id,
                                generation,
                                livrarr_domain::services::IdentityCompletion {
                                    target_badge: Some(IdentityStatus::NotFound),
                                    ..Default::default()
                                },
                            )
                            .await
                        {
                            Ok(livrarr_domain::services::IdentityCompletionOutcome::Superseded) => {
                                tracing::debug!(
                                    work_id,
                                    "complete_add: NotFound conclusion superseded by newer identity write"
                                );
                            }
                            Ok(_) => {}
                            Err(e) => {
                                tracing::warn!(
                                    work_id,
                                    "complete_add: NotFound status completion failed: {e}"
                                );
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(work_id, "complete_add: work re-read failed: {e}");
                }
            }
        }

        // ensure_identity_and_enrichment's Err arms (get_work failure, or the
        // enrichment workflow itself erroring) report Failed in their return
        // value but do not persist it — persist it here so a work's stored
        // status never disagrees with what the caller was just told.
        if enrichment_status == EnrichmentStatus::Failed {
            if let Err(e) = self
                .db
                .update_work_enrichment(
                    user_id,
                    work_id,
                    UpdateWorkEnrichmentDbRequest {
                        enrichment_status: EnrichmentStatus::Failed,
                        ..Default::default()
                    },
                )
                .await
            {
                tracing::warn!(
                    work_id,
                    "complete_add: persisting Failed status failed: {e}"
                );
            }
        }
    }

    fn is_enriching(&self, user_id: UserId, work_id: WorkId) -> bool {
        // Poison-proof: a panicked peer must not wedge this read (same
        // pattern as try_start_bulk_refresh / BulkRefreshGuard).
        let set = self
            .enriching
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        set.contains(&(user_id, work_id))
    }

    async fn get(&self, user_id: UserId, work_id: WorkId) -> Result<Work, WorkServiceError> {
        self.db
            .get_work(user_id, work_id)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => WorkServiceError::NotFound,
                other => WorkServiceError::Db(other),
            })
    }

    async fn get_detail(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<WorkDetailView, WorkServiceError> {
        let work = self.get(user_id, work_id).await?;
        let library_items = self
            .db
            .list_library_items_by_work(user_id, work_id)
            .await
            .map_err(WorkServiceError::Db)?;
        let covers_dir = self.data_dir.join("covers").join(user_id.to_string());
        let cover_mtime = crate::cover::cover_file_mtime(&covers_dir, work_id);
        let audiobook_cover_mtime = crate::cover::audiobook_cover_file_mtime(&covers_dir, work_id);
        Ok(WorkDetailView {
            work,
            library_items,
            cover_mtime,
            audiobook_cover_mtime,
        })
    }

    async fn list(
        &self,
        user_id: UserId,
        filter: WorkFilter,
    ) -> Result<Vec<Work>, WorkServiceError> {
        let mut works = if let Some(author_id) = filter.author_id {
            self.db
                .list_works_by_author(user_id, author_id)
                .await
                .map_err(WorkServiceError::Db)?
        } else {
            self.db
                .list_works(user_id)
                .await
                .map_err(WorkServiceError::Db)?
        };

        if let Some(monitored) = filter.monitored {
            works.retain(|w| (w.monitor_ebook || w.monitor_audiobook) == monitored);
        }
        if let Some(ref status) = filter.enrichment_status {
            works.retain(|w| w.enrichment_status == *status);
        }
        if let Some(media_type) = filter.media_type {
            works.retain(|w| match media_type {
                MediaType::Ebook => w.monitor_ebook,
                MediaType::Audiobook => w.monitor_audiobook,
            });
        }
        if let Some(ref language) = filter.language {
            works.retain(|w| w.language.as_deref() == Some(language.as_str()));
        }
        if let Some(sort_by) = filter.sort_by {
            let dir = filter.sort_dir.unwrap_or(SortDirection::Asc);
            works.sort_by(|a, b| {
                let cmp = match sort_by {
                    WorkSortField::Title => a.title.to_lowercase().cmp(&b.title.to_lowercase()),
                    WorkSortField::DateAdded | WorkSortField::RecentlyDownloaded => {
                        a.added_at.cmp(&b.added_at)
                    }
                    WorkSortField::Year => a.year.cmp(&b.year),
                    WorkSortField::Author => a.author_name.cmp(&b.author_name),
                };
                match dir {
                    SortDirection::Asc => cmp,
                    SortDirection::Desc => cmp.reverse(),
                }
            });
        }

        Ok(works)
    }

    async fn list_paginated(
        &self,
        user_id: UserId,
        page: u32,
        page_size: u32,
        sort_by: WorkSortField,
        sort_dir: SortDirection,
        media_type: Option<MediaType>,
        language: Option<&str>,
    ) -> Result<PaginatedWorksView, WorkServiceError> {
        let sort_col = match sort_by {
            WorkSortField::Title => "title",
            WorkSortField::DateAdded => "date_added",
            WorkSortField::Year => "year",
            WorkSortField::Author => "author",
            WorkSortField::RecentlyDownloaded => "recently_downloaded",
        };
        let dir = match sort_dir {
            SortDirection::Asc => "asc",
            SortDirection::Desc => "desc",
        };
        let (works, total) = self
            .db
            .list_works_paginated(
                user_id, page, page_size, sort_col, dir, media_type, language,
            )
            .await
            .map_err(WorkServiceError::Db)?;

        let work_ids: Vec<i64> = works.iter().map(|w| w.id).collect();
        let items = self
            .db
            .list_library_items_by_work_ids(user_id, &work_ids)
            .await
            .map_err(WorkServiceError::Db)?;

        // Pre-index items by work_id to avoid O(works×items) filtering.
        let mut items_by_work: HashMap<WorkId, Vec<LibraryItem>> =
            HashMap::with_capacity(work_ids.len());
        for item in items {
            items_by_work.entry(item.work_id).or_default().push(item);
        }

        let work_views = works
            .into_iter()
            .map(|w| {
                let work_items = items_by_work.remove(&w.id).unwrap_or_default();
                WorkDetailView {
                    work: w,
                    library_items: work_items,
                    cover_mtime: None,
                    audiobook_cover_mtime: None,
                }
            })
            .collect();

        Ok(PaginatedWorksView {
            works: work_views,
            total,
            page,
            page_size,
        })
    }

    async fn update(
        &self,
        user_id: UserId,
        work_id: WorkId,
        req: UpdateWorkRequest,
    ) -> Result<Work, WorkServiceError> {
        self.db
            .get_work(user_id, work_id)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => WorkServiceError::NotFound,
                other => WorkServiceError::Db(other),
            })?;

        let has_title = req.title.is_some();
        let has_author = req.author_name.is_some();
        let series_name_cleared = matches!(req.series_name, Some(None));
        let series_position_cleared = matches!(req.series_position, Some(None));
        let has_series_name = req.series_name.is_some();
        let has_series_position = req.series_position.is_some();
        let cleaned_title = req.title.map(|t| crate::title_cleanup::clean_title(&t));
        let cleaned_author = req
            .author_name
            .map(|a| crate::title_cleanup::clean_author(&a));
        // REQ-014: route through identity_key like every other stored-key
        // write site. Title and author are independently optional here (a
        // user may rename just one); identity_key's two components are
        // independent per-string computations, so pairing whichever side(s)
        // changed with an empty counterpart for a lone-side update is safe.
        let (normalized_title, normalized_author) =
            match (cleaned_title.as_deref(), cleaned_author.as_deref()) {
                (Some(t), Some(a)) => {
                    let (nt, na) = livrarr_domain::identity_matching::identity_key(t, a);
                    (Some(nt), Some(na))
                }
                (Some(t), None) => (
                    Some(livrarr_domain::identity_matching::identity_key(t, "").0),
                    None,
                ),
                (None, Some(a)) => (
                    None,
                    Some(livrarr_domain::identity_matching::identity_key("", a).1),
                ),
                (None, None) => (None, None),
            };
        let db_req = UpdateWorkUserFieldsDbRequest {
            title: cleaned_title,
            author_name: cleaned_author,
            normalized_title,
            normalized_author,
            series_name: req.series_name,
            series_position: req.series_position,
            monitor_ebook: req.monitor_ebook,
            monitor_audiobook: req.monitor_audiobook,
        };

        let work = self
            .db
            .update_work_user_fields(user_id, work_id, db_req)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => WorkServiceError::NotFound,
                other => WorkServiceError::Db(other),
            })?;

        let mut prov_reqs: Vec<SetFieldProvenanceRequest> = Vec::new();
        if has_title {
            prov_reqs.push(SetFieldProvenanceRequest {
                user_id,
                work_id,
                field: WorkField::Title,
                source: None,
                setter: ProvenanceSetter::User,
                cleared: false,
            });
        }
        if has_author {
            prov_reqs.push(SetFieldProvenanceRequest {
                user_id,
                work_id,
                field: WorkField::AuthorName,
                source: None,
                setter: ProvenanceSetter::User,
                cleared: false,
            });
        }
        if has_series_name {
            prov_reqs.push(SetFieldProvenanceRequest {
                user_id,
                work_id,
                field: WorkField::SeriesName,
                source: None,
                setter: ProvenanceSetter::User,
                cleared: series_name_cleared,
            });
        }
        if has_series_position {
            prov_reqs.push(SetFieldProvenanceRequest {
                user_id,
                work_id,
                field: WorkField::SeriesPosition,
                source: None,
                setter: ProvenanceSetter::User,
                cleared: series_position_cleared,
            });
        }
        if !prov_reqs.is_empty() {
            if let Err(e) = self.db.set_field_provenance_batch(prov_reqs).await {
                tracing::warn!(work_id, "user-edit provenance write failed: {e}");
            }
        }

        // Series reconcile (REQ-001, user origin — always wins): the returned
        // work carries the new series_name and the pre-edit series_id, which
        // is exactly the state reconcile arbitrates (relink/unlink + stub GC).
        // Failures PROPAGATE here: a user-edit unlink/relink has no self-heal
        // (the startup back-fill only links series_id-NULL works), so a
        // swallowed error would leave a visibly stale catalog. Reconcile is
        // idempotent — the client retries the edit safely.
        if has_series_name {
            crate::series_link::reconcile_work_series(
                &self.db,
                &work,
                crate::series_link::SeriesLinkOrigin::User,
            )
            .await
            .map_err(WorkServiceError::Db)?;
        }

        Ok(work)
    }

    async fn delete(&self, user_id: UserId, work_id: WorkId) -> Result<(), WorkServiceError> {
        let work = self
            .db
            .get_work(user_id, work_id)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => WorkServiceError::NotFound,
                other => WorkServiceError::Db(other),
            })?;

        let items = self
            .db
            .list_library_items_by_work(user_id, work_id)
            .await
            .map_err(WorkServiceError::Db)?;

        let deleted = self
            .db
            .delete_work(user_id, work_id)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => WorkServiceError::NotFound,
                other => WorkServiceError::Db(other),
            })?;

        // REQ-006: one composite workDeleted after successful deletion, with
        // work_id None (D-DELETE-ORDER: the work row is gone, so an attached
        // insert would violate the FK); the payload snapshot identifies the
        // row. No per-file events on this road.
        livrarr_db::record_history(
            &self.db,
            user_id,
            history_events::work_deleted(&work.title, Some(&work.author_name), items.len(), false),
        )
        .await;

        // REQ-001/AC-012: deleting a work unlinks it; GC an unmonitored stub
        // left with zero linked works.
        if let Some(series_id) = deleted.series_id {
            if let Err(e) = crate::series_link::gc_stub_if_empty(&self.db, user_id, series_id).await
            {
                tracing::warn!(work_id, series_id, error = %e, "stub GC on work delete failed");
            }
        }

        delete_cover_files(&self.data_dir, user_id, work_id).await;

        for item in &items {
            if let Err(e) = tokio::fs::remove_file(&item.path).await {
                tracing::warn!(
                    work_id = work_id,
                    item_id = item.id,
                    path = %item.path,
                    "failed to delete library file on work delete: {e}"
                );
            }
        }

        Ok(())
    }

    async fn refresh(
        &self,
        user_id: UserId,
        work_id: WorkId,
        surface: RefreshSurface,
    ) -> Result<RefreshWorkResult, WorkServiceError> {
        let work = self.get(user_id, work_id).await?;
        let _refresh_span = livrarr_domain::perf::StageTimer::start("refresh_total", work_id);

        let _guard = self.refresh_locks.lock((user_id, work_id)).await;
        // Signal membership (REQ-005): a refresh-driven run reads as
        // "fetching" too — Retry and the post-add top-up show the pill.
        let _enriching = EnrichingGuard::enter(self.enriching.clone(), (user_id, work_id));

        if let Err(e) = self.db.reset_enrichment_for_refresh(user_id, work_id).await {
            tracing::warn!("reset_enrichment_for_refresh failed: {e}");
        }

        if let Err(e) = self
            .enrichment
            .reset_for_manual_refresh(user_id, work_id)
            .await
        {
            tracing::warn!("enrichment reset_for_manual_refresh failed: {e}");
        }

        // REQ-002/REQ-006 (id-completeness): re-chase a work's still-obtainable
        // hard anchors via the one identity road on every refresh door (single +
        // bulk + retry all funnel through here). "Obtainable" = a NULL works.*
        // column with no pending guess and below the dead-end threshold
        // (chaseable_anchor_types). This SUPERSEDES the Sprint-E `!= Confirmed`
        // gate (insight 55): a Confirmed work missing a secondary id is now
        // topped up, while a fully-anchored or fully-dead-ended work skips the
        // resolver fan-out entirely — the cost Sprint-E removed. settle_identity
        // is the identity authority; the smart-skip it deliberately lacks
        // (ST-002) is re-applied here via the chaseable gate.
        //
        // NOTE: reset_for_manual_refresh above already DELETED provider_retry_state,
        // so a refresh always re-attempts providers — no suppression survives.
        let mut work = work;
        if let Some(resolver) = self.resolver.as_ref() {
            let _id_span = livrarr_domain::perf::StageTimer::start("identity", work_id);
            // REQ-009: the single-work manual refresh is the "try again" door for a
            // stuck identity — clear the dead-end counters so the chase gate below
            // can re-attempt. Healthy works keep the Sprint-E skip; bulk sweeps
            // never clear (a routine sweep must not resurrect dead ends).
            if matches!(surface, RefreshSurface::Interactive)
                && work.identity_status == livrarr_domain::IdentityStatus::NotFound
            {
                if let Err(e) = self.db.clear_anchor_dead_ends(work.id).await {
                    tracing::warn!(work_id, "refresh: failed to clear anchor dead-ends: {e}");
                }
                // reset_for_manual_refresh (above) already recovered the terminal
                // status from the anchor columns; re-read so settle_identity sees
                // the recovered status — its REQ-006 terminal guard no-ops on the
                // stale NotFound and the try-again resolve would never run.
                if let Ok(w) = self.db.get_work(user_id, work_id).await {
                    work = w;
                }
            }
            let anchors = self.db.list_anchors(work.id).await.unwrap_or_default();
            let dead_ends = self
                .db
                .list_anchor_dead_ends(work.id)
                .await
                .unwrap_or_default();
            if !chaseable_anchor_types(&work, &anchors, &dead_ends, DEAD_END_THRESHOLD).is_empty() {
                match crate::async_resolver::settle_identity(
                    resolver.as_ref(),
                    &self.db,
                    user_id,
                    &work,
                    match surface {
                        RefreshSurface::Interactive => {
                            livrarr_domain::identity::IdentityMode::Interactive
                        }
                        RefreshSurface::Bulk => livrarr_domain::identity::IdentityMode::Background,
                    },
                    livrarr_domain::identity::ConflictSource::Refresh,
                )
                .await
                {
                    Ok(_) => {
                        if let Ok(w) = self.db.get_work(user_id, work_id).await {
                            work = w;
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            work_id,
                            "refresh identity settle failed; scatter proceeds: {e}"
                        );
                    }
                }
            }
        }

        // Identity gate (REQ-008/AC-012): the same identity_permits check
        // convergence (convergence_service.rs) and the add door
        // (ensure_identity_and_enrichment) already apply — a held identity does
        // not enrich here either. Re-reads the post-settle status above, so a
        // work the settle step just confirmed still enriches this same call;
        // only a work still Pending/Conflict/NeedsReview after settling skips.
        let identity_permits = !matches!(
            work.identity_status,
            IdentityStatus::Pending | IdentityStatus::Conflict | IdentityStatus::NeedsReview
        );
        if identity_permits {
            // Unified enrichment: provider dispatch, merge, cover download, tag sync.
            // Manual mode (not Background) so a transiently-unavailable provider
            // (e.g. Google Books quota 429) does not defer the entire merge and
            // discard the data other providers returned — best-effort merge. (#117)
            // No candidate_id for a manual refresh — always re-fetches from network.
            let _enrichment_status = self
                .run_unified_enrichment(
                    user_id,
                    &work,
                    None,
                    EnrichmentMode::Manual,
                    None,
                    match surface {
                        RefreshSurface::Interactive => RequestPriority::Normal,
                        RefreshSurface::Bulk => RequestPriority::Low,
                    },
                    // Bypass: both RefreshSurface variants are user-triggered —
                    // a user asking for fresh data gets real fetches (REQ-009).
                    livrarr_domain::Freshness::Bypass,
                )
                .await;
        }

        let refreshed_work = match self.db.get_work(user_id, work_id).await {
            Ok(w) => w,
            Err(_) => work,
        };

        Ok(RefreshWorkResult {
            work: refreshed_work,
            messages: vec![],
            taggable_items: vec![],
            merge_deferred: false,
        })
    }

    async fn retry_all_incomplete(
        &self,
        user_id: UserId,
    ) -> Result<livrarr_domain::services::RetrySummary, WorkServiceError> {
        crate::convergence_service::retry_all_incomplete(self, user_id).await
    }

    // Dead: bulk refresh is implemented at the handler layer
    // (`crates/livrarr-handlers/src/work.rs::refresh_all`) per insight 9g
    // (handler-level spawning for long-running background work). This stub
    // never wired up — the handler does its own list + spawn + iterate +
    // finish_bulk_refresh directly.

    async fn upload_cover(
        &self,
        user_id: UserId,
        work_id: WorkId,
        bytes: &[u8],
    ) -> Result<(), WorkServiceError> {
        // 5 MB — accommodates high-resolution covers from providers like Google Books.
        // TODO(alpha6+): reduce stored cover resolution to limit on-disk footprint.
        const MAX_COVER_BYTES: usize = 5 * 1024 * 1024;

        if bytes.len() > MAX_COVER_BYTES {
            return Err(WorkServiceError::Enrichment(format!(
                "cover too large: {} bytes (max {})",
                bytes.len(),
                MAX_COVER_BYTES
            )));
        }
        if bytes.is_empty() {
            return Err(WorkServiceError::Enrichment("empty image data".into()));
        }
        if !is_supported_image(bytes) {
            return Err(WorkServiceError::Enrichment(
                "unrecognized image format (expected JPEG, PNG, GIF, or WebP)".into(),
            ));
        }

        let _work = self.get(user_id, work_id).await?;

        let covers_dir = self.data_dir.join("covers").join(user_id.to_string());
        tokio::fs::create_dir_all(&covers_dir)
            .await
            .map_err(|e| WorkServiceError::Enrichment(format!("create covers dir: {e}")))?;

        let cover_path = crate::cover_write_gate::final_cover_path(&covers_dir, work_id, "");
        let tmp_path = cover_path.with_extension("jpg.tmp");
        let tmp_clone = tmp_path.clone();
        let target = cover_path.clone();
        let bytes_vec = bytes.to_vec();
        let write_result = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
            use std::io::Write;
            let mut f = std::fs::File::create(&tmp_clone)?;
            f.write_all(&bytes_vec)?;
            f.sync_all()?;
            drop(f);
            std::fs::rename(&tmp_clone, &target)
        })
        .await
        .map_err(|e| WorkServiceError::Enrichment(format!("spawn error: {e}")))?;

        if let Err(e) = write_result {
            let _ = tokio::fs::remove_file(&tmp_path).await;
            return Err(WorkServiceError::Enrichment(format!("write cover: {e}")));
        }

        let thumb_path = crate::cover_write_gate::final_cover_path(&covers_dir, work_id, "_thumb");
        let _ = tokio::fs::remove_file(&thumb_path).await;

        self.db
            .set_cover_manual(user_id, work_id, true)
            .await
            .map_err(WorkServiceError::Db)?;

        Ok(())
    }

    async fn download_cover(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<Vec<u8>, WorkServiceError> {
        let _work = self.get(user_id, work_id).await?;

        // Try new tenant-aware path first, fall back to old flat layout.
        let new_path = crate::cover_write_gate::final_cover_path(
            &self.data_dir.join("covers").join(user_id.to_string()),
            work_id,
            "",
        );
        let cover_path = if new_path.exists() {
            new_path
        } else {
            crate::cover_write_gate::final_cover_path(&self.data_dir.join("covers"), work_id, "")
        };
        let bytes = tokio::fs::read(&cover_path).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                WorkServiceError::NotFound
            } else {
                WorkServiceError::Enrichment(format!("read cover: {e}"))
            }
        })?;
        Ok(bytes)
    }

    async fn search_works(
        &self,
        user_id: UserId,
        query: &str,
        page: u32,
        page_size: u32,
    ) -> Result<(Vec<Work>, i64), WorkServiceError> {
        WorkDb::search_works(&self.db, user_id, query, page, page_size)
            .await
            .map_err(WorkServiceError::Db)
    }

    fn try_start_bulk_refresh(
        &self,
        user_id: i64,
    ) -> Option<livrarr_domain::services::BulkRefreshGuard> {
        let inserted = {
            // Poison-proof: a panicked peer must not wedge the slot set.
            let mut slots = self
                .bulk_refresh_users
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            slots.insert(user_id)
        };
        inserted.then(|| {
            livrarr_domain::services::BulkRefreshGuard::new(
                self.bulk_refresh_users.clone(),
                user_id,
            )
        })
    }

    async fn converge_work(
        &self,
        user_id: UserId,
        work_id: WorkId,
        threshold: u32,
    ) -> Result<ConvergeOutcome, WorkServiceError> {
        crate::convergence_service::converge_work(self, user_id, work_id, threshold).await
    }

    async fn preview_merge_works(
        &self,
        user_id: UserId,
        survivor_id: WorkId,
        loser_id: WorkId,
    ) -> Result<MergePreview, WorkServiceError> {
        if survivor_id == loser_id {
            return Err(WorkServiceError::Validation(
                "cannot merge a work into itself".into(),
            ));
        }

        let survivor = self.get(user_id, survivor_id).await?;
        let loser = self.get(user_id, loser_id).await?;

        let items = self
            .db
            .list_library_items_by_work(user_id, loser_id)
            .await
            .map_err(WorkServiceError::Db)?;
        let grabs = self
            .db
            .list_grabs_by_work(user_id, loser_id)
            .await
            .map_err(WorkServiceError::Db)?;

        Ok(MergePreview {
            survivor_id,
            loser_id,
            library_items_moving: items.len(),
            grabs_moving: grabs.len(),
            monitor_ebook_result: survivor.monitor_ebook || loser.monitor_ebook,
            monitor_audiobook_result: survivor.monitor_audiobook || loser.monitor_audiobook,
            conflicts: merge_field_conflicts(&survivor, &loser),
        })
    }

    async fn merge_works(
        &self,
        user_id: UserId,
        survivor_id: WorkId,
        loser_id: WorkId,
        choices: Vec<MergeFieldChoiceEntry>,
    ) -> Result<MergeWorksResult, WorkServiceError> {
        if survivor_id == loser_id {
            return Err(WorkServiceError::Validation(
                "cannot merge a work into itself".into(),
            ));
        }

        let survivor = self.get(user_id, survivor_id).await?;
        let loser = self.get(user_id, loser_id).await?;

        // Recompute conflicts fresh rather than trusting the caller's
        // (possibly stale) preview — every conflict needs a matching entry
        // in `choices` or the whole call refuses (AC-025).
        let conflicts = merge_field_conflicts(&survivor, &loser);
        let missing: Vec<MergeableField> = conflicts
            .iter()
            .map(|c| c.field)
            .filter(|field| !choices.iter().any(|entry| entry.field == *field))
            .collect();
        if !missing.is_empty() {
            return Err(WorkServiceError::MergeChoiceRequired(missing));
        }

        let choice_for = |field: MergeableField| {
            choices
                .iter()
                .find(|entry| entry.field == field)
                .map(|entry| entry.choice)
        };

        // A field with no conflict is additive: whichever side actually has
        // a value wins, so no data is lost when only one side was ever set
        // (REQ-015 d). A field WITH a conflict follows the caller's choice.
        let series_name = match choice_for(MergeableField::SeriesName) {
            Some(MergeFieldChoice::KeepSurvivor) => survivor.series_name.clone(),
            Some(MergeFieldChoice::TakeLoser) => loser.series_name.clone(),
            None => survivor.series_name.clone().or(loser.series_name.clone()),
        };
        let series_position = match choice_for(MergeableField::SeriesPosition) {
            Some(MergeFieldChoice::KeepSurvivor) => survivor.series_position,
            Some(MergeFieldChoice::TakeLoser) => loser.series_position,
            None => survivor.series_position.or(loser.series_position),
        };
        let monitor_ebook = survivor.monitor_ebook || loser.monitor_ebook;
        let monitor_audiobook = survivor.monitor_audiobook || loser.monitor_audiobook;

        // Snapshot counts before the DB call folds the loser's rows into
        // the survivor — afterward there is no way to tell "moved" from
        // "was already the survivor's."
        let library_items_moved = self
            .db
            .list_library_items_by_work(user_id, loser_id)
            .await
            .map_err(WorkServiceError::Db)?
            .len();
        let grabs_moved = self
            .db
            .list_grabs_by_work(user_id, loser_id)
            .await
            .map_err(WorkServiceError::Db)?
            .len();

        let updated_survivor = self
            .db
            .merge_works(MergeWorksDbRequest {
                user_id,
                survivor_id,
                loser_id,
                monitor_ebook,
                monitor_audiobook,
                series_name,
                series_position,
            })
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => WorkServiceError::NotFound,
                other => WorkServiceError::Db(other),
            })?;

        // REQ-007(b): the survivor gains one worksMerged naming the merged-
        // away work, after the transactional repoint+delete committed.
        livrarr_db::record_history(
            &self.db,
            user_id,
            history_events::works_merged(
                survivor_id,
                &updated_survivor.title,
                &loser.title,
                loser_id,
            ),
        )
        .await;

        // Physical file reorganization (REQ-015 c) is a separate, best-effort
        // step the caller runs via `ImportService::reorganize_work_files` —
        // this service has no filesystem access (compile-wall seam,
        // livrarr-metadata may not depend on livrarr-library). `warnings`
        // starts empty; the handler appends the reorg step's warnings.
        //
        // Identity-edit r4: both generations were advanced by the merge
        // transaction's first statement; eagerly drop both works' local
        // preview snapshots (the durable generation already makes them
        // stale — removal only frees capacity).
        self.remove_preview_snapshots_for(&[survivor_id, loser_id]);
        Ok(MergeWorksResult {
            survivor: updated_survivor,
            library_items_moved,
            grabs_moved,
            warnings: Vec::new(),
        })
    }

    async fn preview_identity_edit(
        &self,
        user_id: UserId,
        work_id: WorkId,
        input: &str,
        slot_hint: Option<livrarr_domain::identity::AnchorType>,
    ) -> Result<IdentityEditPreview, livrarr_domain::identity_edit::IdentityEditError> {
        use livrarr_domain::identity_edit::{classify_identifier_input, IdentityEditError};

        let work = self.get(user_id, work_id).await.map_err(edit_service_err)?;

        // One coherent user-scoped basis: generation + validated
        // ledger∪column slots + open conflicts. Every assessment below uses
        // it; the stored snapshot carries ITS generation, never a later read.
        let basis = self
            .db
            .read_identity_edit_basis(user_id, work_id)
            .await
            .map_err(|e| IdentityEditError::Db(e.to_string()))?;

        let (slot, canonical) = classify_identifier_input(input, slot_hint)
            .map_err(|e| IdentityEditError::InvalidValue(e.to_string()))?;

        // This path used to be silent end to end: a run of failed previews wrote
        // call records and not one log line, so the only way to learn what a
        // provider had actually answered was to reproduce the failure by hand.
        tracing::info!(
            work_id,
            slot = %slot.as_str(),
            value = %canonical,
            "identity preview: fetching"
        );

        // Fetch the certified record for the submitted value (§Preview seam).
        let (resolved, leg_outcomes) = self
            .fetch_slot_record(&slot, &canonical, work.language.clone())
            .await;
        tracing::info!(
            work_id,
            slot = %slot.as_str(),
            resolved = resolved.is_some(),
            legs = ?leg_outcomes,
            "identity preview: fetched"
        );
        let Some(resolved) = resolved else {
            let failure_reason = if leg_outcomes
                .iter()
                .all(|o| matches!(o, SlotFetchClass::NotFound))
            {
                "not_found"
            } else {
                "provider_unavailable"
            };
            // Provider failure → 200 with resolved: null + reason; nothing is
            // certifiable; NO snapshot stored.
            return Ok(IdentityEditPreview {
                slot: Some(slot),
                canonical_value: Some(canonical),
                conflict_warning: !basis.open_conflict_kinds.is_empty(),
                failure_reason: Some(failure_reason.to_string()),
                ..IdentityEditPreview::default()
            });
        };

        // Collision check (work-key slots): ledger ∪ same-user-filtered
        // column scan; the owning work's id/title block certification and
        // the UI offers Merge works. Bridges became legally shareable at 076.
        if is_work_key_slot(&slot) {
            if let Some(owner) = self
                .db
                .find_anchor_owner(user_id, &slot, &canonical, work_id)
                .await
                .map_err(|e| IdentityEditError::Db(e.to_string()))?
            {
                return Ok(IdentityEditPreview {
                    resolved: Some(resolved),
                    slot: Some(slot),
                    canonical_value: Some(canonical),
                    collision: Some(owner),
                    conflict_warning: !basis.open_conflict_kinds.is_empty(),
                    ..IdentityEditPreview::default()
                });
            }
        }

        // Sibling assessment (work-key slots): every OTHER work-key slot with
        // an effective (ledger∪column) value gets a proven-agreement verdict
        // against the certified record; bridges are informational only.
        let mut siblings = Vec::new();
        let mut bridge_warnings = Vec::new();
        if is_work_key_slot(&slot) {
            for sibling_slot in [
                livrarr_domain::identity::AnchorType::new(
                    livrarr_domain::identity::AnchorType::OL_WORK,
                ),
                livrarr_domain::identity::AnchorType::new(
                    livrarr_domain::identity::AnchorType::GR_WORK,
                ),
                livrarr_domain::identity::AnchorType::new(
                    livrarr_domain::identity::AnchorType::HC_WORK,
                ),
            ] {
                if sibling_slot == slot {
                    continue;
                }
                let Some(value) = basis.slot(&sibling_slot).effective().map(str::to_string) else {
                    continue;
                };
                siblings.push(
                    self.assess_sibling(&sibling_slot, &value, &resolved, &canonical, &slot, &work)
                        .await,
                );
            }
            for bridge_slot in [
                livrarr_domain::identity::AnchorType::new(
                    livrarr_domain::identity::AnchorType::ISBN_13,
                ),
                livrarr_domain::identity::AnchorType::new(
                    livrarr_domain::identity::AnchorType::ASIN,
                ),
            ] {
                let Some(value) = basis.slot(&bridge_slot).effective().map(str::to_string) else {
                    continue;
                };
                if let Some(warning) = self
                    .assess_bridge(&bridge_slot, &value, &resolved, &canonical, &slot, &work)
                    .await
                {
                    bridge_warnings.push(warning);
                }
            }
        }

        // Certifiable — store the single-use intent token. The basis
        // generation (not a later read) is the commit staleness authority.
        let drop_slots: Vec<livrarr_domain::identity::AnchorType> = siblings
            .iter()
            .filter(|s| s.action == SiblingAction::Drop)
            .map(|s| s.slot.clone())
            .collect();
        let preview_id = self.store_preview_snapshot(PreviewSnapshot {
            user_id,
            work_id,
            slot: slot.clone(),
            canonical_value: canonical.clone(),
            generation: basis.generation,
            drop_slots,
            expires_at: std::time::Instant::now() + PREVIEW_SNAPSHOT_TTL,
            seq: 0,
        })?;

        Ok(IdentityEditPreview {
            resolved: Some(resolved),
            slot: Some(slot),
            canonical_value: Some(canonical),
            preview_id: Some(preview_id),
            siblings,
            bridge_warnings,
            collision: None,
            conflict_warning: !basis.open_conflict_kinds.is_empty(),
            failure_reason: None,
        })
    }

    async fn commit_identity_edit(
        &self,
        user_id: UserId,
        work_id: WorkId,
        slot: livrarr_domain::identity::AnchorType,
        preview_id: &str,
    ) -> Result<IdentityEditCommit, livrarr_domain::identity_edit::IdentityEditError> {
        use livrarr_domain::identity::AnchorSetter;
        use livrarr_domain::identity_edit::IdentityEditError;

        self.get(user_id, work_id).await.map_err(edit_service_err)?;

        // Consume the snapshot atomically (remove-on-read; matching
        // user+work+slot required) — missing, expired, or already used is
        // one and the same 409 preview_required.
        let snapshot = self
            .consume_preview_snapshot(preview_id, user_id, work_id, &slot)
            .ok_or(IdentityEditError::StalePreview)?;

        // One current repository snapshot decides true-no-op; a generation
        // mismatch is stale, never a no-op.
        let basis = self
            .db
            .read_identity_edit_basis(user_id, work_id)
            .await
            .map_err(|e| IdentityEditError::Db(e.to_string()))?;
        if basis.generation != snapshot.generation {
            return Err(IdentityEditError::StalePreview);
        }

        let slot_basis = basis.slot(&slot);
        let old_value = slot_basis
            .confirmed
            .as_ref()
            .map(|(v, _)| v.clone())
            .or_else(|| slot_basis.column.clone());

        let same_user_confirmed = matches!(
            &slot_basis.confirmed,
            Some((v, AnchorSetter::User)) if v == &snapshot.canonical_value
        );
        let column_agrees = slot_basis.column.as_deref() == Some(snapshot.canonical_value.as_str());
        let no_implicated_conflict = !basis
            .open_conflict_kinds
            .iter()
            .any(|k| conflict_kind_implicates(*k, &slot));
        // A no-op must be a no-op in the database too: AC-20 requires a commit to clear
        // the slot's pending guesses, so a surviving pending row is outstanding work and
        // the commit has to run. Without this the user re-certifies the value they
        // already have, is told nothing changed, and the stale guess stays affirmable.
        let is_true_no_op = same_user_confirmed
            && column_agrees
            && snapshot.drop_slots.is_empty()
            && no_implicated_conflict
            && !slot_basis.dead_end
            && slot_basis.pending.is_empty()
            && basis.stored_badge == basis.derived_badge;
        if is_true_no_op {
            let work = self.get(user_id, work_id).await.map_err(edit_service_err)?;
            return Ok(IdentityEditCommit {
                work,
                no_op: true,
                old_value,
                new_value: snapshot.canonical_value,
            });
        }

        self.db
            .apply_identity_edit(
                work_id,
                user_id,
                slot,
                &snapshot.canonical_value,
                snapshot.generation,
                &snapshot.drop_slots,
            )
            .await?;

        // The durable generation already stales every remaining token for
        // this work; eager removal only frees capacity.
        self.remove_preview_snapshots_for(&[work_id]);

        let work = self.get(user_id, work_id).await.map_err(edit_service_err)?;
        Ok(IdentityEditCommit {
            work,
            no_op: false,
            old_value,
            new_value: snapshot.canonical_value,
        })
    }

    async fn clear_identity_slot(
        &self,
        user_id: UserId,
        work_id: WorkId,
        slot: livrarr_domain::identity::AnchorType,
    ) -> Result<IdentityEditClear, livrarr_domain::identity_edit::IdentityEditError> {
        self.get(user_id, work_id).await.map_err(edit_service_err)?;

        let cleared = self.db.apply_identity_clear(work_id, user_id, slot).await?;

        // Eagerly consume/invalidate all of the work's preview snapshots —
        // the generation bump is the durable backstop.
        self.remove_preview_snapshots_for(&[work_id]);

        let work = self.get(user_id, work_id).await.map_err(edit_service_err)?;
        Ok(IdentityEditClear {
            work,
            old_value: cleared.old_value,
            parked_by_conflicts: cleared.parked_by_conflicts,
        })
    }
}

/// Fields where both works carry a differing non-empty user value (REQ-015
/// d). Title/author are deliberately excluded — the survivor's identity
/// fields are not renegotiated by a merge.
fn merge_field_conflicts(survivor: &Work, loser: &Work) -> Vec<MergeFieldConflict> {
    let mut conflicts = Vec::new();

    if let (Some(s), Some(l)) = (
        survivor.series_name.as_deref().map(str::trim),
        loser.series_name.as_deref().map(str::trim),
    ) {
        if !s.is_empty() && !l.is_empty() && s != l {
            conflicts.push(MergeFieldConflict {
                field: MergeableField::SeriesName,
                survivor_value: s.to_string(),
                loser_value: l.to_string(),
            });
        }
    }

    if let (Some(s), Some(l)) = (survivor.series_position, loser.series_position) {
        if s != l {
            conflicts.push(MergeFieldConflict {
                field: MergeableField::SeriesPosition,
                survivor_value: s.to_string(),
                loser_value: l.to_string(),
            });
        }
    }

    conflicts
}

// =============================================================================
// Identity-edit preview/commit machinery (design identity-edit r4)
// =============================================================================

/// Bounds for the process-local preview snapshot store (§Preview 6):
/// per-user cap 4, global cap 64, TTL 10 min. Load-bearing — the process has
/// one shared live work service, so saturation is cross-tenant.
const PREVIEW_PER_USER_CAP: usize = 4;
const PREVIEW_GLOBAL_CAP: usize = 64;
const PREVIEW_SNAPSHOT_TTL: std::time::Duration = std::time::Duration::from_secs(600);
const PREVIEW_CAPACITY_RETRY_SECS: u64 = 30;

/// One stored single-use preview intent (§Preview 6). The observed
/// `identity_generation` — not a later read — is the commit staleness
/// authority; the drop set is server-computed and applied exactly.
struct PreviewSnapshot {
    user_id: UserId,
    work_id: WorkId,
    slot: livrarr_domain::identity::AnchorType,
    canonical_value: String,
    generation: i64,
    drop_slots: Vec<livrarr_domain::identity::AnchorType>,
    expires_at: std::time::Instant,
    /// Monotonic insertion order — "oldest" for the per-user eviction.
    seq: u64,
}

#[derive(Default)]
struct PreviewSnapshotStore {
    map: HashMap<String, PreviewSnapshot>,
    next_seq: u64,
}

fn edit_service_err(e: WorkServiceError) -> livrarr_domain::identity_edit::IdentityEditError {
    match e {
        WorkServiceError::NotFound => livrarr_domain::identity_edit::IdentityEditError::NotFound,
        other => livrarr_domain::identity_edit::IdentityEditError::Db(other.to_string()),
    }
}

fn is_work_key_slot(slot: &livrarr_domain::identity::AnchorType) -> bool {
    use livrarr_domain::identity::AnchorType;
    matches!(
        slot.as_str(),
        AnchorType::OL_WORK | AnchorType::GR_WORK | AnchorType::HC_WORK
    )
}

/// Whether an open conflict of `kind` implicates an edit of `slot` (the
/// no-op check and the commit's closure set agree on this mapping): the
/// slot's own kind(s), plus QuorumTie for any work-key edit.
fn conflict_kind_implicates(
    kind: livrarr_domain::identity::IdentityConflictKind,
    slot: &livrarr_domain::identity::AnchorType,
) -> bool {
    use livrarr_domain::identity::{AnchorType, IdentityConflictKind};
    match kind {
        IdentityConflictKind::IncomingDifferentOlKey
        | IdentityConflictKind::OlRedirectCollision => slot.as_str() == AnchorType::OL_WORK,
        IdentityConflictKind::IncomingDifferentGrKey => slot.as_str() == AnchorType::GR_WORK,
        IdentityConflictKind::IncomingDifferentHcKey => slot.as_str() == AnchorType::HC_WORK,
        IdentityConflictKind::QuorumTie => is_work_key_slot(slot),
    }
}

/// Ordered preview fallback legs per slot (§Preview seam): gr→Goodreads;
/// ol→OpenLibrary; asin→Audnexus then Audible; isbn→Google Books (ISBN-echo-
/// verified) then OpenLibrary; hc→Hardcover with `AnchorQuery::HcKey`.
fn preview_legs(
    slot: &livrarr_domain::identity::AnchorType,
    value: &str,
) -> Vec<(MetadataProvider, AnchorQuery)> {
    use livrarr_domain::identity::AnchorType;
    let v = value.to_string();
    match slot.as_str() {
        AnchorType::GR_WORK => vec![(MetadataProvider::Goodreads, AnchorQuery::GrKey(v))],
        AnchorType::OL_WORK => vec![(MetadataProvider::OpenLibrary, AnchorQuery::OlKey(v))],
        AnchorType::HC_WORK => vec![(MetadataProvider::Hardcover, AnchorQuery::HcKey(v))],
        AnchorType::ISBN_13 => vec![
            (
                MetadataProvider::GoogleBooks,
                AnchorQuery::Isbn13(v.clone()),
            ),
            (MetadataProvider::OpenLibrary, AnchorQuery::Isbn13(v)),
        ],
        _ => vec![
            (MetadataProvider::Audnexus, AnchorQuery::Asin(v.clone())),
            (MetadataProvider::Audible, AnchorQuery::Asin(v)),
        ],
    }
}

/// Aggregated non-success class of one preview leg.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SlotFetchClass {
    NotFound,
    NotConfigured,
    Unavailable,
}

/// Identity evidence for the matching authority: the record's own keys with
/// the queried slot value overlaid (the record WAS fetched by that key).
fn record_evidence<'a>(
    record: &'a livrarr_domain::services::IdentityPreviewRecord,
    slot: &livrarr_domain::identity::AnchorType,
    value: &'a str,
) -> livrarr_domain::identity_matching::IdEvidence<'a> {
    use livrarr_domain::identity::AnchorType;
    let mut evidence = livrarr_domain::identity_matching::IdEvidence {
        ol_key: record.ol_key.as_deref(),
        gr_key: record.gr_key.as_deref(),
        hc_key: record.hc_key.as_deref(),
        isbn_13: record.isbn_13.as_deref(),
        asin: record.asin.as_deref(),
    };
    match slot.as_str() {
        AnchorType::OL_WORK => evidence.ol_key = Some(value),
        AnchorType::GR_WORK => evidence.gr_key = Some(value),
        AnchorType::HC_WORK => evidence.hc_key = Some(value),
        AnchorType::ISBN_13 => evidence.isbn_13 = Some(value),
        _ => evidence.asin = Some(value),
    }
    evidence
}

/// The proven-agreement bar (§Preview 3): keep iff `title_id_trust` holds —
/// title Same, or Grey{OneSidedSubtitle}, in either case with no same-provider
/// work-key contradiction — AND author Agree. Everything else is not proven.
/// A one-sided subtitle no longer needs an agreeing hard identifier: a subtitle
/// is edition-level, so demanding an edition bridge between two different
/// printings asked a question that by construction has no answer.
fn proven_agreement(
    certified: &livrarr_domain::services::IdentityPreviewRecord,
    certified_evidence: &livrarr_domain::identity_matching::IdEvidence<'_>,
    sibling: &livrarr_domain::services::IdentityPreviewRecord,
    sibling_evidence: &livrarr_domain::identity_matching::IdEvidence<'_>,
) -> Option<bool> {
    use livrarr_domain::identity_matching::{
        author_verdict, parse_title, title_id_trust, title_verdict, AuthorVerdict,
    };
    let (Some(cert_title), Some(cert_author)) = (&certified.title, &certified.author) else {
        return None;
    };
    let (Some(sib_title), Some(sib_author)) = (&sibling.title, &sibling.author) else {
        return None;
    };
    let title = title_verdict(&parse_title(cert_title), &parse_title(sib_title));
    let title_ok = title_id_trust(&title, sibling_evidence, certified_evidence);
    let author_ok = matches!(
        author_verdict(
            std::slice::from_ref(cert_author),
            std::slice::from_ref(sib_author),
        ),
        AuthorVerdict::Agree
    );
    Some(title_ok && author_ok)
}

impl<D, E, H> WorkServiceImpl<D, E, H> {
    /// Insert a snapshot per the §Preview 6 bounds: expired entries removed
    /// first, then the user's own oldest evicted at the per-user cap; a
    /// still-full global cap of OTHER tenants' live tokens is a retryable
    /// 503 — never evict another tenant's token.
    fn store_preview_snapshot(
        &self,
        mut snapshot: PreviewSnapshot,
    ) -> Result<String, livrarr_domain::identity_edit::IdentityEditError> {
        let mut store = self
            .preview_snapshots
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = std::time::Instant::now();
        store.map.retain(|_, s| s.expires_at > now);

        let own: Vec<(String, u64)> = store
            .map
            .iter()
            .filter(|(_, s)| s.user_id == snapshot.user_id)
            .map(|(k, s)| (k.clone(), s.seq))
            .collect();
        // Replace the caller's own oldest token when they are at their per-user cap OR
        // when the store is globally full. Evicting only at the per-user cap refuses a
        // caller who holds a perfectly evictable token just because other users filled
        // the store — the specified policy is to spend your own slot before you are told
        // to come back later.
        let at_own_cap = own.len() >= PREVIEW_PER_USER_CAP;
        let globally_full = store.map.len() >= PREVIEW_GLOBAL_CAP;
        if at_own_cap || globally_full {
            if let Some((oldest, _)) = own.into_iter().min_by_key(|(_, seq)| *seq) {
                store.map.remove(&oldest);
            }
        }
        // Only now, with the caller's own slot spent, is capacity a real refusal.
        if store.map.len() >= PREVIEW_GLOBAL_CAP {
            return Err(livrarr_domain::identity_edit::IdentityEditError::Capacity {
                retry_after_secs: PREVIEW_CAPACITY_RETRY_SECS,
            });
        }

        snapshot.seq = store.next_seq;
        store.next_seq += 1;
        let preview_id = uuid::Uuid::new_v4().to_string();
        store.map.insert(preview_id.clone(), snapshot);
        Ok(preview_id)
    }

    /// Remove-on-read, single-use: a hit must match user, work, AND slot and
    /// be unexpired; anything else reads as consumed.
    fn consume_preview_snapshot(
        &self,
        preview_id: &str,
        user_id: UserId,
        work_id: WorkId,
        slot: &livrarr_domain::identity::AnchorType,
    ) -> Option<PreviewSnapshot> {
        let mut store = self
            .preview_snapshots
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let snapshot = store.map.remove(preview_id)?;
        let matches = snapshot.user_id == user_id
            && snapshot.work_id == work_id
            && &snapshot.slot == slot
            && snapshot.expires_at > std::time::Instant::now();
        matches.then_some(snapshot)
    }

    fn remove_preview_snapshots_for(&self, work_ids: &[WorkId]) {
        let mut store = self
            .preview_snapshots
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        store.map.retain(|_, s| !work_ids.contains(&s.work_id));
    }
}

impl<D, E, H> WorkServiceImpl<D, E, H>
where
    E: EnrichmentWorkflow + Send + Sync,
{
    /// Ordered-fallback fetch of one slot's record (one `preview_fetch` per
    /// leg, Interactive priority, riding the process-global outbound queue).
    /// Returns the first resolved record plus every leg's non-success class.
    async fn fetch_slot_record(
        &self,
        slot: &livrarr_domain::identity::AnchorType,
        value: &str,
        language: Option<String>,
    ) -> (
        Option<livrarr_domain::services::IdentityPreviewRecord>,
        Vec<SlotFetchClass>,
    ) {
        use livrarr_domain::identity::AnchorType;
        use livrarr_domain::services::IdentityPreviewOutcome;
        let mut classes = Vec::new();
        for (provider, query) in preview_legs(slot, value) {
            let is_gb = provider == MetadataProvider::GoogleBooks;
            match self
                .enrichment
                .fetch_anchor_preview(
                    provider,
                    query,
                    language.clone(),
                    RequestPriority::Interactive,
                )
                .await
            {
                Ok(IdentityPreviewOutcome::Resolved(record)) => {
                    // GB serves editions by ISBN — trust it only when it
                    // echoes the queried ISBN back (§Preview seam).
                    if is_gb
                        && slot.as_str() == AnchorType::ISBN_13
                        && record.isbn_13.as_deref() != Some(value)
                    {
                        classes.push(SlotFetchClass::NotFound);
                        continue;
                    }
                    return (Some(*record), classes);
                }
                Ok(IdentityPreviewOutcome::NotFound) => classes.push(SlotFetchClass::NotFound),
                Ok(IdentityPreviewOutcome::NotConfigured) => {
                    classes.push(SlotFetchClass::NotConfigured)
                }
                Ok(IdentityPreviewOutcome::Unavailable) | Err(_) => {
                    classes.push(SlotFetchClass::Unavailable)
                }
            }
        }
        (None, classes)
    }

    /// §Preview 3 — one sibling work-key slot vs the certified record. The
    /// keep bar is PROVEN agreement; uncorroborated Grey, VetoVolume,
    /// Different, author Abstain/Grey/Disagree, and failed fetches all drop
    /// (labeled). One deliberate distinction: an unconfigured Hardcover
    /// contributes no payload to enrichment, so `NotConfigured` keeps —
    /// HC-only; an unwired GR/OL sibling is unproven and drops.
    async fn assess_sibling(
        &self,
        sibling_slot: &livrarr_domain::identity::AnchorType,
        value: &str,
        certified: &livrarr_domain::services::IdentityPreviewRecord,
        certified_value: &str,
        certified_slot: &livrarr_domain::identity::AnchorType,
        work: &Work,
    ) -> SiblingAssessment {
        use livrarr_domain::identity::AnchorType;
        let (record, classes) = self
            .fetch_slot_record(sibling_slot, value, work.language.clone())
            .await;
        let (action, cause) = match record {
            Some(record) => {
                let certified_evidence =
                    record_evidence(certified, certified_slot, certified_value);
                let sibling_evidence = record_evidence(&record, sibling_slot, value);
                match proven_agreement(certified, &certified_evidence, &record, &sibling_evidence) {
                    Some(true) => (SiblingAction::Keep, None),
                    Some(false) => (SiblingAction::Drop, Some("disagrees".to_string())),
                    // A payload without usable title/author proves nothing.
                    None => (SiblingAction::Drop, Some("unproven".to_string())),
                }
            }
            None => {
                let all_not_configured = !classes.is_empty()
                    && classes.iter().all(|c| *c == SlotFetchClass::NotConfigured);
                if all_not_configured && sibling_slot.as_str() == AnchorType::HC_WORK {
                    (SiblingAction::Keep, None)
                } else if classes.contains(&SlotFetchClass::Unavailable) {
                    (SiblingAction::Drop, Some("unverifiable".to_string()))
                } else {
                    (SiblingAction::Drop, Some("unproven".to_string()))
                }
            }
        };
        SiblingAssessment {
            slot: sibling_slot.clone(),
            action,
            cause,
        }
    }

    /// Bridges are assessed informationally only (§Preview 3, ratified): a
    /// PROVEN disagreement warns by name; anything unresolvable stays silent.
    /// Never enters the drop set.
    async fn assess_bridge(
        &self,
        bridge_slot: &livrarr_domain::identity::AnchorType,
        value: &str,
        certified: &livrarr_domain::services::IdentityPreviewRecord,
        certified_value: &str,
        certified_slot: &livrarr_domain::identity::AnchorType,
        work: &Work,
    ) -> Option<BridgeWarning> {
        let (record, _) = self
            .fetch_slot_record(bridge_slot, value, work.language.clone())
            .await;
        let record = record?;
        let certified_evidence = record_evidence(certified, certified_slot, certified_value);
        let bridge_evidence = record_evidence(&record, bridge_slot, value);
        match proven_agreement(certified, &certified_evidence, &record, &bridge_evidence) {
            Some(false) => Some(BridgeWarning {
                slot: bridge_slot.clone(),
                message: format!(
                    "your stored {} resolves to a different book — consider fixing or clearing it",
                    if bridge_slot.as_str() == livrarr_domain::identity::AnchorType::ISBN_13 {
                        "ISBN"
                    } else {
                        "ASIN"
                    }
                ),
            }),
            _ => None,
        }
    }
}

// =============================================================================
// add() helpers
// =============================================================================

impl<D, E, H> WorkServiceImpl<D, E, H>
where
    D: WorkDb
        + WorkDbCreate
        + AuthorDb
        + LibraryItemDb
        + ProvenanceDb
        + EnrichmentRetryDb
        + livrarr_db::ProviderRetryStateDb
        + livrarr_db::SeriesDb
        + livrarr_db::HistoryDb
        + livrarr_db::AuthorLinkDb
        + livrarr_domain::services::WorkIdentityRepository
        + Send
        + Sync,
    E: EnrichmentWorkflow + Send + Sync,
    H: HttpFetcher + Clone + Send + Sync + 'static,
{
    async fn try_dedup_by_normalized(
        &self,
        user_id: UserId,
        normalized_title: &str,
        normalized_author: &str,
        source_provider_data: &Option<SourceProviderData>,
    ) -> Result<Option<(Work, EnrichmentStatus)>, WorkServiceError> {
        let existing = self
            .db
            .find_by_normalized_match(user_id, normalized_title, normalized_author)
            .await
            .map_err(WorkServiceError::Db)?;
        if let Some(work) = existing.into_iter().next() {
            let (work, enrichment_status) = if source_provider_data.is_some() {
                // No candidate_id: dedup path re-enriches existing work from network.
                // High: reached only from `add()`'s Pending-identity arm — the
                // same interactive Add/manual-import door as
                // `ensure_identity_and_enrichment`, not a background call
                // (unlisted call site — B4 table's "add door" bucket applied
                // by extension; flagged in the B4 report).
                let (status, _) = self
                    .run_unified_enrichment(
                        user_id,
                        &work,
                        source_provider_data.clone(),
                        EnrichmentMode::Background,
                        None,
                        RequestPriority::High,
                        livrarr_domain::Freshness::PreferCache,
                    )
                    .await;
                let refreshed = self.db.get_work(user_id, work.id).await.unwrap_or(work);
                (refreshed, status)
            } else {
                let status = work.enrichment_status;
                (work, status)
            };
            Ok(Some((work, enrichment_status)))
        } else {
            Ok(None)
        }
    }

    /// Find, adopt, or create the author of a work being added.
    ///
    /// Every arm leaves the author with one due author-link task: a new author
    /// gets it from the shared create/adopt gate in the same transaction as the
    /// row itself, and an author that predates the gate is repaired here. A
    /// route the user explicitly selected in the add flow is attached as their
    /// own choice afterwards — a separate, independently retryable step, so a
    /// failure there leaves the author enqueued rather than orphaned.
    async fn find_or_create_author(
        &self,
        user_id: UserId,
        cleaned_author: &str,
        author_ol_key: Option<&str>,
    ) -> Result<(bool, Option<i64>), WorkServiceError> {
        if cleaned_author.is_empty() {
            return Ok((false, None));
        }
        let normalized = cleaned_author.to_lowercase();
        let (created, author_id) = match self
            .db
            .find_author_by_name(user_id, &normalized)
            .await
            .map_err(WorkServiceError::Db)?
        {
            Some(existing) => {
                self.arm_author_link(user_id, existing.id).await;
                (false, existing.id)
            }
            None => {
                let authors = self
                    .db
                    .list_authors(user_id)
                    .await
                    .map_err(WorkServiceError::Db)?;
                let names: Vec<String> = authors.iter().map(|a| a.name.clone()).collect();
                if let Some(i) = livrarr_domain::identity_matching::unambiguous_author_match(
                    cleaned_author,
                    &names,
                ) {
                    // The adopted author's provider key is not filled in here:
                    // `authors.ol_key` is frozen (FP-031) and the selected key
                    // becomes a route row below, the same as on every other arm.
                    let adopted = &authors[i];
                    self.arm_author_link(user_id, adopted.id).await;
                    (false, adopted.id)
                } else {
                    // `created` is the DB's own verdict: a creation-race loser
                    // converges on the winning row and reports false, exactly
                    // like a lookup hit (REQ-002).
                    let (author, created) = self
                        .db
                        .create_or_adopt_author(livrarr_db::CreateAuthorGateRequest {
                            user_id,
                            name: cleaned_author.to_string(),
                            sort_name: None,
                            import_id: None,
                            initial_name_source: AuthorNameSource::Import,
                            trigger: AuthorLinkTrigger::AuthorCreated,
                        })
                        .await
                        .map_err(WorkServiceError::Db)?;
                    (created, author.id)
                }
            }
        };
        self.attach_selected_author_route(user_id, author_id, author_ol_key)
            .await;
        Ok((created, Some(author_id)))
    }

    /// Make sure an adopted author has a due author-link task.
    ///
    /// Warn-only: an author that already exists is not made worse by a
    /// bookkeeping failure here, and the startup repair pass covers a row that
    /// never got one.
    async fn arm_author_link(&self, user_id: UserId, author_id: AuthorId) {
        if let Err(e) = self
            .db
            .ensure_enqueued(user_id, author_id, AuthorLinkTrigger::AuthorAdopted)
            .await
        {
            tracing::warn!(author_id, error = %e, "author-link enqueue failed for adopted author");
        }
    }

    /// Record a provider route the user picked in the add flow.
    ///
    /// This is a user selection, not provider proof, so it is stored as one. A
    /// value that is not a canonical author route is dropped with a warning
    /// rather than persisted raw, and no failure here fails the add.
    async fn attach_selected_author_route(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        author_ol_key: Option<&str>,
    ) {
        let Some(raw) = author_ol_key.map(str::trim).filter(|raw| !raw.is_empty()) else {
            return;
        };
        let key = match AuthorRouteKey::parse(AuthorProvider::OpenLibrary, raw) {
            Ok(key) => key,
            Err(e) => {
                tracing::warn!(
                    author_id,
                    %raw,
                    ?e,
                    "selected author route is not a canonical OpenLibrary author key"
                );
                return;
            }
        };
        if let Err(e) = self.db.attach_route_as_user(user_id, author_id, key).await {
            tracing::warn!(author_id, error = %e, "selected author route attach failed");
        }
    }

    /// REQ-010 (#144): the single identity+enrichment decision EVERY add
    /// outcome takes (created, anchor-matched, adopted, deduped, race-loser,
    /// and `complete_add`'s background completion). An anchor-less work first
    /// runs the add-time identity leg via the one identity road
    /// (`settle_identity`) — the engine resolves the seed, partitions hard vs
    /// fuzzy anchors (REQ-004), and raises the badge itself. Enrichment then
    /// runs only when the identity permits and the work needs it — an
    /// already-Enriched dedup re-add is never re-enriched, and a held identity
    /// (Pending/Conflict/NeedsReview) blocks enrichment unconditionally: a
    /// disputed identity must settle before any provider fetch, whichever
    /// door reached it. `(mode, source)` are threaded from the originating
    /// door (REQ-001/005).
    async fn ensure_identity_and_enrichment(
        &self,
        user_id: UserId,
        work_id: WorkId,
        source_provider_data: Option<SourceProviderData>,
        candidate_id: Option<livrarr_domain::identity::CandidateId>,
        mode: livrarr_domain::identity::IdentityMode,
        source: livrarr_domain::identity::ConflictSource,
    ) -> (EnrichmentStatus, bool) {
        use livrarr_domain::IdentityStatus;

        let mut work = match self.db.get_work(user_id, work_id).await {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!(
                    work_id,
                    "ensure_identity_and_enrichment: get_work failed: {e}"
                );
                return (EnrichmentStatus::Failed, false);
            }
        };

        let anchorless = work.ol_key.is_none()
            && work.gr_key.is_none()
            && work.hc_key.is_none()
            && work.isbn_13.is_none()
            && work.asin.is_none();
        if anchorless && work.identity_status != IdentityStatus::Conflict {
            if let Some(resolver) = self.resolver.as_ref() {
                // The add-time identity leg routes through the one identity road
                // (settle_identity): resolve the anchorless seed, hard/fuzzy
                // split (REQ-004), monotonic badge raise. (mode, source) come
                // from the door.
                match crate::async_resolver::settle_identity(
                    resolver.as_ref(),
                    &self.db,
                    user_id,
                    &work,
                    mode,
                    source,
                )
                .await
                {
                    Ok(_) => {
                        if let Ok(w) = self.db.get_work(user_id, work.id).await {
                            work = w;
                        }
                    }
                    Err(e) => {
                        tracing::warn!(work_id, "add-time identity settle failed: {e}");
                    }
                }
            } else {
                tracing::warn!(
                    work_id,
                    "no resolver composed — skipping add-time identity leg"
                );
            }
        }

        // Identity gate (unchanged shape): a held identity does not enrich
        // here — the fan-out waits for identity convergence.
        if matches!(
            work.identity_status,
            IdentityStatus::Pending | IdentityStatus::Conflict | IdentityStatus::NeedsReview
        ) {
            return (work.enrichment_status, false);
        }

        // Needs-enrichment gate: Unenriched/Failed or fresh source data; an
        // already-Enriched dedup re-add returns untouched.
        let needs = matches!(
            work.enrichment_status,
            EnrichmentStatus::Unenriched | EnrichmentStatus::Failed
        ) || source_provider_data.is_some();
        if !needs {
            return (work.enrichment_status, false);
        }

        // High: the interactive Add/manual-import flow's provider work (B4
        // table) — mode stays Background (suppression/budget semantics
        // untouched), priority is the door's own queue-ordering hint.
        self.run_unified_enrichment(
            user_id,
            &work,
            source_provider_data,
            EnrichmentMode::Background,
            candidate_id,
            RequestPriority::High,
            livrarr_domain::Freshness::PreferCache,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn handle_race_loser(
        &self,
        user_id: UserId,
        work: Work,
        author_created: bool,
        author_id: Option<i64>,
        source_provider_data: Option<SourceProviderData>,
        mode: livrarr_domain::identity::IdentityMode,
        source: livrarr_domain::identity::ConflictSource,
    ) -> Result<AddWorkResult, WorkServiceError> {
        let (enrichment_status, _identity_not_found) = self
            .ensure_identity_and_enrichment(
                user_id,
                work.id,
                source_provider_data,
                None,
                mode,
                source,
            )
            .await;
        let work = self.db.get_work(user_id, work.id).await.unwrap_or(work);
        Ok(AddWorkResult {
            work,
            created: false,
            author_created,
            author_id,
            messages: vec![],
            cover_mtime: None,
            audiobook_cover_mtime: None,
            enrichment_status,
        })
    }

    /// REQ-004 (responsiveness): the response-path portion of work creation —
    /// everything `finish_created_work` used to do up through the phase-1
    /// cover (`:1836-1900` at the time of the split), MINUS
    /// `ensure_identity_and_enrichment`. `add_fast` calls this and returns;
    /// `complete_add` runs the identity+enrichment remainder separately over
    /// the work this persists.
    #[allow(clippy::too_many_arguments)]
    async fn finish_created_work_fast(
        &self,
        user_id: UserId,
        work: Work,
        author_created: bool,
        author_id: Option<i64>,
        derived_identity: livrarr_domain::IdentityStatus,
        is_user_initiated: bool,
        add_source: history_events::WorkAddSource,
    ) -> Result<AddWorkResult, WorkServiceError> {
        // REQ-001: the birth event, at the one created:true funnel — the work
        // row is already committed. Dedup returns, adopt returns, and race
        // losers never pass here, so exactly-once holds by construction.
        livrarr_db::record_history(
            &self.db,
            user_id,
            history_events::added(work.id, &work.title, Some(&work.author_name), add_source),
        )
        .await;

        // Persist the identity-confidence badge derived at resolution time
        // (REQ-014/D-013) — independent of enrichment, written once at create.
        self.db
            .set_identity_status(user_id, work.id, derived_identity)
            .await
            .map_err(WorkServiceError::Db)?;

        // Series reconcile (REQ-001): a metadata-provided series_name gets a
        // series row (stub if absent) and the FK link. Worker-created works
        // arrive already linked (series_id set) — reconcile no-ops on them.
        // Warn-only by design: a failed link here self-heals at the next
        // startup back-fill (REQ-002 targets series_id-NULL works), and a
        // successful add must not fail over series bookkeeping.
        if work.series_id.is_none() && work.series_name.is_some() {
            if let Err(e) = crate::series_link::reconcile_work_series(
                &self.db,
                &work,
                crate::series_link::SeriesLinkOrigin::System,
            )
            .await
            {
                tracing::warn!(work_id = work.id, error = %e, "series reconcile at create failed");
            }
        }

        // Phase 1: synchronous cover download within 3s budget (REQ-010).
        let covers_dir = self.data_dir.join("covers").join(user_id.to_string());
        let phase1_mtime = crate::cover::fetch_phase1_cover(
            &self.http,
            &work.title,
            &work.author_name,
            work.cover_url.as_deref(),
            None,
            &covers_dir,
            work.id,
        )
        .await;

        // Assign trust based on how the work was added.
        if phase1_mtime.is_some() || work.cover_url.is_some() {
            // A user-picked cover (cover_manual, set from the selected search
            // result) is locked at User trust so enrichment never overrides it
            // (resolve_cover bails on User) and update_cover_metadata keeps the
            // cover_manual flag set. Without this, the phase-1 write would assign
            // Validated trust and reset cover_manual to false, letting background
            // enrichment replace the user's chosen cover. That lock is earned
            // only when the download actually succeeded — see
            // `addtime_cover_trust`.
            let trust = addtime_cover_trust(
                work.cover_manual,
                work.cover_url.is_some(),
                phase1_mtime.is_some(),
                is_user_initiated,
            );
            let source = work.cover_source.as_deref().unwrap_or("add");
            // REQ-017/S3: measure the bytes phase-1 just wrote instead of
            // stamping 0x0 — the file only exists when phase1_mtime is Some;
            // a failed download (cover_url set, no file) keeps 0x0, matching
            // the "row describes what's on disk" invariant.
            let (width, height) = if phase1_mtime.is_some() {
                crate::cover_resolution::measure_dimensions(
                    &crate::cover_write_gate::final_cover_path(&covers_dir, work.id, ""),
                )
                .map(|(w, h)| (w as i32, h as i32))
                .unwrap_or((0, 0))
            } else {
                (0, 0)
            };
            let _ = self
                .db
                .update_cover_metadata(
                    user_id,
                    work.id,
                    work.cover_url.as_deref(),
                    source,
                    trust,
                    width,
                    height,
                )
                .await;
        }

        let updated_work = self
            .db
            .get_work(user_id, work.id)
            .await
            .map_err(WorkServiceError::Db)?;
        let covers_dir = self.data_dir.join("covers").join(user_id.to_string());
        let cover_mtime = crate::cover::cover_file_mtime(&covers_dir, updated_work.id);
        let audiobook_cover_mtime =
            crate::cover::audiobook_cover_file_mtime(&covers_dir, updated_work.id);
        let enrichment_status = updated_work.enrichment_status;
        Ok(AddWorkResult {
            work: updated_work,
            created: true,
            author_created,
            author_id,
            messages: vec![],
            cover_mtime,
            audiobook_cover_mtime,
            enrichment_status,
        })
    }
}

// =============================================================================
// Unified enrichment pipeline
// =============================================================================

impl<D, E, H> WorkServiceImpl<D, E, H>
where
    D: WorkDb
        + LibraryItemDb
        + ProvenanceDb
        + EnrichmentRetryDb
        + livrarr_db::SeriesDb
        + livrarr_db::ProviderRetryStateDb
        + livrarr_db::HistoryDb
        + livrarr_db::AuthorLinkDb
        + livrarr_domain::services::WorkIdentityRepository
        + Send
        + Sync,
    E: EnrichmentWorkflow + Send + Sync,
    H: HttpFetcher + Clone + Send + Sync + 'static,
{
    /// Run the full enrichment pipeline synchronously (REQ-001/012).
    ///
    /// Steps:
    ///   1. Inject source provider data (if present) via `enrichment.inject_source_data`
    ///   2. Dispatch to providers via `enrichment.enrich_work` (candidate_id → cache reuse)
    ///   3. Reload work from DB (post-merge state)
    ///   4. Materialize: cover download + tag write, change-gated (REQ-012)
    ///
    /// Returns `(enrichment_status, identity_not_found)`. Never returns `Err` — all
    /// failures are absorbed, producing `Failed` status when enrichment itself fails
    /// and otherwise continuing past materialize errors (non-fatal, warned).
    /// `priority` (B4) is the queue-ordering hint threaded to the scatter —
    /// independent of `mode`, so a door can request Background mode
    /// (suppression/budget semantics) while still queuing ahead of a
    /// background scan.
    /// `freshness` (REQ-009) decides whether the scatter's provider fetches
    /// may be served from the persistent provider-response cache — Bypass for
    /// user-triggered refresh, PreferCache for background/add flows (D-004).
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn run_unified_enrichment(
        &self,
        user_id: UserId,
        work: &Work,
        source_provider_data: Option<livrarr_domain::services::SourceProviderData>,
        mode: EnrichmentMode,
        candidate_id: Option<livrarr_domain::identity::CandidateId>,
        priority: RequestPriority,
        freshness: livrarr_domain::Freshness,
    ) -> (EnrichmentStatus, bool) {
        let work_id = work.id;

        // REQ-008 parity at the add door: an anchor-poor work starves the
        // scatter — every provider skips on "no anchor" and the status lands
        // Failed with zero network (e.g. a GR-link add carries only gr_key,
        // which no enrich provider consumes). Run the same identity
        // anchor-completion the refresh door runs, so fresh anchors are in
        // the DB before the scatter reads it. One-shot per add — the
        // refresh door keeps the terminal-outcome bookkeeping for its loop.
        if work.ol_key.is_none()
            && work.isbn_13.is_none()
            && work.asin.is_none()
            && work.hc_key.is_none()
        {
            if let Some(resolver) = self.resolver.as_ref() {
                // Same identity anchor-completion the refresh door runs, via the
                // one identity road (settle_identity). Background mode: this
                // fires mid-enrichment for an anchor-poor work (e.g. a GR-only
                // add) so fresh anchors land in the DB before the scatter reads.
                if let Err(e) = crate::async_resolver::settle_identity(
                    resolver.as_ref(),
                    &self.db,
                    user_id,
                    work,
                    livrarr_domain::identity::IdentityMode::Background,
                    livrarr_domain::identity::ConflictSource::Refresh,
                )
                .await
                {
                    tracing::warn!(work_id, "add-door anchor completion failed: {e}");
                }
            }
        }

        // Step 1: Inject source provider data (Readarr import etc.)
        if let Some(src) = source_provider_data {
            self.enrichment
                .inject_source_data(user_id, work_id, src)
                .await;
        }

        // Step 2: Provider dispatch — scatter-gather enrichment. `candidate_id`
        // enables cache reuse (zero network) when the user picked a specific
        // candidate during discovery (AC-001).
        let enrich_result = match self
            .enrichment
            .enrich_work(user_id, work_id, mode, candidate_id, priority, freshness)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(work_id, "run_unified_enrichment: enrich_work failed: {e}");
                // REQ-002: a failing attempt records exactly one
                // enrichmentFailed; the host road still returns normally.
                livrarr_db::record_history(
                    &self.db,
                    user_id,
                    history_events::enrichment_failed(
                        work_id,
                        &work.title,
                        Some(&work.author_name),
                        &e.to_string(),
                    ),
                )
                .await;
                return (EnrichmentStatus::Failed, false);
            }
        };

        // Step 3: After enrichment, reload work from DB (reflects merged state).
        let post_enrich_work = match self.db.get_work(user_id, work_id).await {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!(work_id, "run_unified_enrichment: get_work failed: {e}");
                livrarr_db::record_history(
                    &self.db,
                    user_id,
                    history_events::enrichment_failed(
                        work_id,
                        &work.title,
                        Some(&work.author_name),
                        &format!("work reload after enrichment failed: {e}"),
                    ),
                )
                .await;
                return (EnrichmentStatus::Failed, false);
            }
        };

        // Series reconcile (REQ-001, system origin): an enriched series_name
        // gets its stub/link; a work already linked to a GR-backed series is
        // left alone (string-only — system writes never displace GR-grounded
        // assignment, AC-021).
        if let Err(e) = crate::series_link::reconcile_work_series(
            &self.db,
            &post_enrich_work,
            crate::series_link::SeriesLinkOrigin::System,
        )
        .await
        {
            tracing::warn!(work_id, error = %e, "series reconcile after enrichment failed");
        }

        let final_status = enrich_result.enrichment_status;
        let identity_not_found = enrich_result.identity_not_found;

        // Step 4 (S2): the cover write gate — the ONE save chokepoint every
        // non-User cover resolution routes through, add/refresh/background-
        // retry alike (all three funnel through this function). Downloads
        // the candidate, lets the comparator decide, and — on accept —
        // commits url/source/trust/dims to the DB as one atomic write. The
        // binding invariant: cover DB fields update ONLY here (or at
        // phase-1 create), never in the generic field merge above. The gate
        // reads the incumbent's state itself, under its per-slot lock —
        // passing a snapshot from here would hand it stale values whenever
        // another writer commits between our read and the lock acquisition.
        let covers_dir = self.data_dir.join("covers").join(user_id.to_string());

        let mut ebook_prefetched: Option<Vec<u8>> = None;
        if let Some(resolution) = enrich_result.cover_resolution.clone() {
            let outcome = crate::cover_write_gate::run_cover_write_gate(
                &self.db,
                &self.http,
                user_id,
                crate::cover_write_gate::CoverWriteGateInput {
                    covers_dir: covers_dir.clone(),
                    work_id,
                    media_type: livrarr_domain::CoverMediaType::Ebook,
                    resolution,
                },
            )
            .await;
            if let crate::cover_write_gate::GateOutcome::Accepted { bytes, .. } = outcome {
                ebook_prefetched = Some(bytes);
            }
        }

        if let Some(resolution) = enrich_result.audiobook_cover_resolution.clone() {
            let _ = crate::cover_write_gate::run_cover_write_gate(
                &self.db,
                &self.http,
                user_id,
                crate::cover_write_gate::CoverWriteGateInput {
                    covers_dir: covers_dir.clone(),
                    work_id,
                    media_type: livrarr_domain::CoverMediaType::Audiobook,
                    resolution,
                },
            )
            .await;
        }

        // Re-read: the gate above may have just committed new cover state.
        let post_enrich_work = self
            .db
            .get_work(user_id, work_id)
            .await
            .unwrap_or(post_enrich_work);

        // Step 5: Materialize — tag write only for covers now (REQ-012); the
        // gate owns cover download/decision/DB-commit entirely, so both
        // chosen_new_url fields stay None (materialize's own download
        // branches — kept intact for defense-in-depth — never re-fire for
        // this caller). Accepted ebook bytes ride through so the retag
        // embeds the new art (V2).
        let items = self
            .db
            .list_taggable_items_by_work(user_id, work_id)
            .await
            .unwrap_or_default();
        let file_paths: Vec<std::path::PathBuf> = items
            .iter()
            .map(|i| std::path::PathBuf::from(&i.path))
            .collect();

        let mat_req = livrarr_domain::services::MaterializeRequest {
            work_id,
            changed: enrich_result.changed,
            tag_fields_changed: enrich_result.changed,
            ebook_cover: livrarr_domain::services::CoverSlotState {
                chosen_new_url: None,
                current_url: None,
                current_path: None,
                user_locked: post_enrich_work.cover_trust == livrarr_domain::CoverTrust::User,
                prefetched_bytes: ebook_prefetched,
            },
            audiobook_cover: livrarr_domain::services::CoverSlotState::default(),
            file_paths,
            tags: livrarr_domain::services::MaterializeTags {
                title: post_enrich_work.title.clone(),
                subtitle: post_enrich_work.subtitle.clone(),
                author: post_enrich_work.author_name.clone(),
                narrator: post_enrich_work.narrator.clone(),
                year: post_enrich_work.year,
                genre: post_enrich_work.genres.clone(),
                description: post_enrich_work.description.clone(),
                publisher: post_enrich_work.publisher.clone(),
                isbn: post_enrich_work.isbn_13.clone(),
                language: post_enrich_work.language.clone(),
                series_name: post_enrich_work.series_name.clone(),
                series_position: post_enrich_work.series_position,
            },
            covers_dir: covers_dir.clone(),
        };

        let materialize =
            livrarr_materialize::LiveMaterializeService::new(Arc::new(self.http.clone()));
        let mat_outcome = {
            let _mat_span = livrarr_domain::perf::StageTimer::start("materialize", work_id);
            livrarr_domain::services::MaterializeService::materialize(&materialize, mat_req).await
        };
        if let Err(e) = &mat_outcome {
            tracing::warn!(work_id, "run_unified_enrichment: materialize failed: {e}");
        }

        // REQ-017 belt-and-braces: a stored cover file with no measured dims
        // (pre-existing data, or a slot the gate above didn't touch this
        // pass) gets a lazy opportunistic re-measure. This writes ONLY the
        // dims columns of an already-provenanced row — not a cover write, so
        // it does not reopen AC-10 (no fourth road).
        if post_enrich_work.cover_width == 0 && post_enrich_work.cover_url.is_some() {
            if let Some((w, h)) = measure_cover_file_dims(&covers_dir, work_id, "").await {
                if let Err(e) = self
                    .db
                    .update_cover_dimensions(user_id, work_id, w, h)
                    .await
                {
                    tracing::warn!(work_id, "cover dimension backfill failed: {e}");
                }
            }
        }
        if post_enrich_work.audiobook_cover_width == 0
            && post_enrich_work.audiobook_cover_url.is_some()
        {
            if let Some((w, h)) = measure_cover_file_dims(&covers_dir, work_id, "_audio").await {
                if let Err(e) = self
                    .db
                    .update_audiobook_cover_dimensions(user_id, work_id, w, h)
                    .await
                {
                    tracing::warn!(work_id, "audiobook cover dimension backfill failed: {e}");
                }
            }
        }

        // REQ-002: exactly one of {enriched, enrichmentFailed, nothing} per
        // invocation. A deferred merge records nothing — the concluding pass
        // is the moment; a pass that never attempted (no provider dispatch,
        // no merge application) records nothing.
        if !enrich_result.merge_deferred && enrich_result.attempted {
            let tags_written = matches!(&mat_outcome, Ok(o) if o.tags_written);
            livrarr_db::record_history(
                &self.db,
                user_id,
                history_events::enriched(
                    work_id,
                    &post_enrich_work.title,
                    Some(&post_enrich_work.author_name),
                    enrich_result.changed,
                    &final_status,
                    tags_written,
                ),
            )
            .await;
        }

        (final_status, identity_not_found)
    }
}

async fn measure_cover_file_dims(
    covers_dir: &std::path::Path,
    work_id: WorkId,
    suffix: &str,
) -> Option<(i32, i32)> {
    let path = crate::cover_write_gate::final_cover_path(covers_dir, work_id, suffix);
    let bytes = tokio::fs::read(&path).await.ok()?;
    tokio::task::spawn_blocking(move || {
        image::load_from_memory(&bytes)
            .map(|img| (img.width() as i32, img.height() as i32))
            .ok()
    })
    .await
    .ok()
    .flatten()
}

async fn write_addtime_provenance<D: ProvenanceDb>(
    db: &D,
    user_id: i64,
    work: &Work,
    setter: ProvenanceSetter,
) {
    crate::provenance::write_addtime_provenance(db, user_id, work, setter).await;
}

pub async fn delete_cover_files(data_dir: &std::path::Path, user_id: i64, work_id: i64) {
    for dir in [
        data_dir.join("covers").join(user_id.to_string()),
        data_dir.join("covers"),
    ] {
        let _ =
            tokio::fs::remove_file(crate::cover_write_gate::final_cover_path(&dir, work_id, ""))
                .await;
        let _ = tokio::fs::remove_file(crate::cover_write_gate::final_cover_path(
            &dir, work_id, "_thumb",
        ))
        .await;
        let _ = tokio::fs::remove_file(crate::cover_write_gate::final_cover_path(
            &dir, work_id, "_audio",
        ))
        .await;
        let _ = tokio::fs::remove_file(crate::cover_write_gate::final_cover_path(
            &dir,
            work_id,
            "_audio_thumb",
        ))
        .await;
        let _ = tokio::fs::remove_file(dir.join(format!("{work_id}.candidate.tmp"))).await;
        let _ = tokio::fs::remove_file(dir.join(format!("{work_id}_audio.candidate.tmp"))).await;
    }
}

fn is_supported_image(bytes: &[u8]) -> bool {
    if bytes.len() < 12 {
        return false;
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return true;
    }
    if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
        return true;
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return true;
    }
    if &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return true;
    }
    false
}

/// Trust to stamp for the phase-1 add-time cover write (REQ-010).
///
/// A user-picked candidate (`cover_manual` with a `cover_url` set from the
/// selected search result) locks at `CoverTrust::User` so background
/// enrichment never overrides it (`resolve_cover` bails on `User`) — but
/// only when the phase-1 download actually produced a file
/// (`download_succeeded`). A failed download leaves no file on disk; locking
/// that slot at `User` trust would permanently refuse every future
/// candidate before the write gate ever checks whether a file exists (the
/// bug this guards against). A failed user-pick download instead falls back
/// to `CoverTrust::Unvalidated` — the weakest trust, since
/// `allows_replacement_by` returns `true` for every incoming trust — leaving
/// the slot fully replaceable, consistent with how `derive_cover_trust`
/// already maps every non-success provider outcome.
///
/// Every other add (no manual pick, or a manual pick with no URL) keeps the
/// existing `phase1_trust` computation unchanged.
fn addtime_cover_trust(
    cover_manual: bool,
    has_cover_url: bool,
    download_succeeded: bool,
    is_user_initiated: bool,
) -> livrarr_domain::CoverTrust {
    if cover_manual && has_cover_url {
        return if download_succeeded {
            livrarr_domain::CoverTrust::User
        } else {
            livrarr_domain::CoverTrust::Unvalidated
        };
    }
    let is_fallback = download_succeeded && !has_cover_url;
    crate::cover_resolution::phase1_trust(is_user_initiated, is_fallback)
}

#[cfg(test)]
mod addtime_cover_trust_tests {
    use super::*;

    #[test]
    fn manual_pick_failed_download_is_replaceable() {
        // BUG: a user-picked candidate whose phase-1 download failed must
        // not lock the slot at User trust — no file landed, so the write
        // gate would refuse every future candidate forever.
        let trust = addtime_cover_trust(true, true, false, true);
        assert_ne!(trust, livrarr_domain::CoverTrust::User);
        assert_eq!(trust, livrarr_domain::CoverTrust::Unvalidated);
    }

    #[test]
    fn manual_pick_successful_download_stays_user_locked() {
        // Deliberate product behavior: a successful user-picked download
        // keeps the absolute User lock so enrichment never overrides it.
        let trust = addtime_cover_trust(true, true, true, true);
        assert_eq!(trust, livrarr_domain::CoverTrust::User);
    }

    #[test]
    fn manual_pick_without_url_is_unaffected() {
        // cover_manual with no cover_url never entered the User-lock branch
        // before this fix and must not start now — falls through to
        // phase1_trust exactly as it always has (is_fallback = downloaded
        // && !has_cover_url).
        assert_eq!(
            addtime_cover_trust(true, false, true, true),
            crate::cover_resolution::phase1_trust(true, true)
        );
    }

    #[test]
    fn non_manual_add_delegates_to_phase1_trust_unchanged() {
        // Non-manual path is untouched by this fix — delegates to
        // phase1_trust exactly as before.
        assert_eq!(
            addtime_cover_trust(false, true, true, true),
            crate::cover_resolution::phase1_trust(true, false)
        );
        assert_eq!(
            addtime_cover_trust(false, false, false, false),
            crate::cover_resolution::phase1_trust(false, false)
        );
    }
}

#[cfg(test)]
mod refresh_locks_sweeper_tests {
    use super::*;

    // The inherent constructors impose no trait bounds on D/E/H — `()`
    // stands in for both `db` and `http` since this test never calls a
    // trait method on either, only inspects the sweep wiring.
    fn new_test_service() -> WorkServiceImpl<(), StubNoEnrichment, ()> {
        WorkServiceImpl::without_enrichment((), (), PathBuf::from("unused"))
    }

    /// D3 #8 / R-5: `sweep()` existed with zero production callers. This
    /// proves the constructor now spawns a task holding a live `Arc` clone
    /// of the SAME `refresh_locks` the service locks against — strong_count
    /// is 2 (the struct's own field + the spawned task's clone) only if a
    /// task was actually spawned and targets this instance; it stays 1 if
    /// the wiring regresses or spawns an unrelated instance.
    #[tokio::test]
    async fn constructor_spawns_a_sweeper_holding_the_live_refresh_locks_arc() {
        let svc = new_test_service();
        assert_eq!(
            Arc::strong_count(&svc.refresh_locks),
            2,
            "constructor must spawn exactly one sweep task holding its own \
             Arc clone of refresh_locks"
        );
    }

    /// The sweeper must be a recurring loop, not a one-shot: after the real
    /// 300s production interval elapses (via tokio's mock clock — no real
    /// waiting), the task must still be alive and holding its Arc clone.
    #[tokio::test(start_paused = true)]
    async fn the_spawned_sweeper_survives_past_one_full_interval_without_dying() {
        let svc = new_test_service();
        assert_eq!(Arc::strong_count(&svc.refresh_locks), 2);

        tokio::time::advance(std::time::Duration::from_secs(301)).await;
        // Let the woken task actually run its tick and re-arm the next one.
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;

        assert_eq!(
            Arc::strong_count(&svc.refresh_locks),
            2,
            "the sweep loop must still be alive (looping, not one-shot) \
             after a full interval elapses"
        );
    }

    /// Defensive guard regression test: constructing a service outside any
    /// Tokio runtime (a handful of the 80+ call sites across the workspace
    /// are test fixtures; this crate cannot prove every one of them runs
    /// under `#[tokio::test]`) must never panic. If the guard is ever
    /// weakened to an unconditional `tokio::spawn`, this turns into a panic.
    #[test]
    fn constructor_does_not_panic_outside_a_tokio_runtime() {
        let svc = new_test_service();
        assert_eq!(
            Arc::strong_count(&svc.refresh_locks),
            1,
            "no runtime is current here, so no sweep task should be spawned"
        );
    }
}
