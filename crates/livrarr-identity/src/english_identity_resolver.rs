use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use livrarr_domain::identity::*;
use livrarr_domain::identity_matching;
use livrarr_domain::normalization::normalize_language;
use livrarr_domain::services::WorkIdentityError;
use livrarr_domain::{MetadataProvider, UserId, Work};
use uuid::Uuid;

use livrarr_external_data::provider_client::ProviderClient;
use livrarr_external_data::transport_cache::TransportCache;
use livrarr_external_data::{NormalizedWorkDetail, ProviderOutcome};

pub use livrarr_domain::identity::WorkSeed;
pub use livrarr_domain::services::IdentityResolver as EnglishIdentityResolver;

#[derive(Debug, Clone)]
pub struct ResolverConfig {
    pub confirm_runner_up_delta: f64,
    /// Per-provider call budget; a provider exceeding it abstains (REQ-025).
    pub call_timeout: Duration,
    /// A Google Books API key is configured (ST-009) — gates GB selection.
    pub gb_key_present: bool,
    /// The install's default language (REQ-007/REQ-013), read from
    /// `metadata_config.default_language`. The one named indirection point
    /// this module reads for "what counts as this install's language" —
    /// every language-silent decision below reads this field, never a
    /// hardcoded "en".
    pub default_language_source: String,
}

impl Default for ResolverConfig {
    fn default() -> Self {
        Self {
            confirm_runner_up_delta: 0.10,
            call_timeout: Duration::from_secs(10),
            gb_key_present: false,
            default_language_source: "en".to_string(),
        }
    }
}

/// Tier-scoped, multi-provider identity resolver. Fans out to every provider
/// relevant to the seed (the #97 fix — never a single hardcoded provider),
/// keeps the full per-provider payloads (cached under a fresh `candidate_id` for
/// `add()` to reuse), and resolves identity by deterministic quorum.
pub struct LiveEnglishIdentityResolver {
    /// Provider clients keyed by provider — the same shape `fetch_internal_alternatives`
    /// uses. Tests inject `ProviderClient::Stub` variants.
    pub clients: HashMap<MetadataProvider, ProviderClient>,
    /// Server-side payload cache; populated on every resolve that fetched payloads.
    pub cache: Arc<TransportCache>,
    pub config: ResolverConfig,
}

impl EnglishIdentityResolver for LiveEnglishIdentityResolver {
    async fn resolve(
        &self,
        user_id: UserId,
        seed: &WorkSeed,
        tier: LatencyTier,
    ) -> Result<Resolution, WorkIdentityError> {
        if !seed_has_signal(seed) {
            return Err(WorkIdentityError::EmptySeed);
        }

        // "The user's pick is the identity vote": a user-confirmed seed that
        // already carries a work anchor (ol/gr/hc) is trusted directly — no
        // provider fan-out, so an interactive add is zero-network. Bridge-only
        // (isbn/asin) or automated (non-confirmed) seeds resolve normally below.
        if seed.user_confirmed
            && (seed.ol_key.is_some() || seed.gr_key.is_some() || seed.hc_key.is_some())
        {
            let identity = captured_from_seed(seed);
            let provenance = provenance_all_hard(&identity);
            return Ok(Resolution::Resolved {
                identity,
                method: method_for_seed(seed),
                candidate_id: CandidateId(Uuid::new_v4().to_string()),
                provenance,
            });
        }

        let providers = self.select_providers(seed, tier);
        let work = build_transient_work_from_seed(seed, user_id);

        // Tier→priority (B4 table): Interactive (Add/manual-import) is High,
        // Bulk (list/Readarr import) and Background (monitors/convergence)
        // are both Low — neither should queue ahead of an interactive add or
        // a foreground refresh.
        let priority = match tier {
            LatencyTier::Interactive => livrarr_domain::RequestPriority::High,
            LatencyTier::Bulk | LatencyTier::Background => livrarr_domain::RequestPriority::Low,
        };

        // Fan out to the eligible providers in parallel, each under the per-call
        // timeout. A timeout or any non-Success outcome is an abstention — it
        // neither errors the resolve nor counts as quorum disagreement (REQ-025).
        let mut futures = Vec::new();
        for provider in providers {
            if let Some(client) = self.clients.get(&provider) {
                let client = client.clone();
                let work = work.clone();
                let timeout = self.config.call_timeout;
                futures.push(async move {
                    match tokio::time::timeout(timeout, client.fetch(&work, priority)).await {
                        Ok(ProviderOutcome::Success(detail)) => Some((provider, *detail)),
                        _ => None,
                    }
                });
            }
        }
        let mut responders: HashMap<MetadataProvider, NormalizedWorkDetail> =
            futures::future::join_all(futures)
                .await
                .into_iter()
                .flatten()
                .collect();

        // Identity fan-outs are otherwise forensically invisible (per-client
        // sinks instrument the anchor-fetch surface, not this road) — log the
        // responder shapes the quorum will arbitrate.
        for (provider, d) in &responders {
            tracing::debug!(
                provider = provider.record_key(),
                title = d.title.as_deref().unwrap_or(""),
                has_author = d.author_name.is_some(),
                ol = d.ol_key.is_some(),
                gr = d.gr_key.is_some(),
                hc = d.hc_key.is_some(),
                isbn = d.isbn_13.is_some(),
                asin = d.asin.is_some(),
                "identity fan-out responder"
            );
        }

        // Language veto (REQ-007): a payload whose declared language flatly
        // contradicts the work's own established language can never merge or
        // absorb, regardless of title similarity — treated exactly like a
        // provider that never responded. A solo responder excluded this way
        // falls through to the existing no-responder path below; a responder
        // alongside others simply loses the vote.
        if let Some(work_lang) = norm_lang(seed.language.as_deref()) {
            responders.retain(|_, d| {
                !matches!(
                    identity_matching::language_verdict(
                        Some(work_lang.as_str()),
                        norm_lang(d.language.as_deref()).as_deref(),
                        &self.config.default_language_source,
                    ),
                    identity_matching::LanguageVerdict::Veto
                )
            });
        }

        // REQ-024: a Goodreads key the seed did NOT carry is trusted only if the
        // payload Goodreads already returned matches the resolved identity — no
        // extra network. Strip it otherwise (anti-bot / LLM-misresolved edition).
        if seed.gr_key.is_none() {
            if let Some(gr) = responders.get(&MetadataProvider::Goodreads) {
                if gr.gr_key.is_some() && !verify_gr_payload(gr, &captured_from_seed(seed)) {
                    if let Some(gr) = responders.get_mut(&MetadataProvider::Goodreads) {
                        gr.gr_key = None;
                    }
                }
            }
        }

        // Mint a handle and cache the harvested payloads for in-process reuse by
        // WorkService::add (D-005). An empty harvest caches nothing — add() will
        // cache-miss and enrich from the network (the exceptional path).
        let candidate_id = CandidateId(Uuid::new_v4().to_string());
        if !responders.is_empty() {
            self.cache
                .cache_put(user_id, candidate_id.clone(), responders.clone());
        }

        // No provider responded. A hard-identifier seed still resolves Tier-A by
        // its own identifier (REQ-011, incl. the GB-only ISBN case); a title-only
        // seed is transiently unresolved and converges on a later pass (REQ-025).
        if responders.is_empty() {
            let captured = captured_from_seed(seed);
            let provenance = provenance_all_hard(&captured);
            if seed_has_hard_id(seed) {
                return Ok(Resolution::Resolved {
                    identity: captured,
                    method: method_for_seed(seed),
                    candidate_id,
                    provenance,
                });
            }
            return Ok(Resolution::Unresolved {
                captured,
                reason: PendingReason::NoCandidates,
                candidate_id: None,
                provenance,
            });
        }

        let mut resolution = run_quorum(&responders, seed);

        match &resolution {
            Resolution::Resolved { identity, .. } => tracing::debug!(
                ol = identity.ol_key.as_deref().unwrap_or(""),
                gr = identity.gr_key.as_deref().unwrap_or(""),
                hc = identity.hc_key.as_deref().unwrap_or(""),
                isbn = identity.isbn_13.as_deref().unwrap_or(""),
                asin = identity.asin.as_deref().unwrap_or(""),
                "identity quorum resolved"
            ),
            Resolution::Conflict { .. } => tracing::debug!("identity quorum: conflict (tie)"),
            Resolution::NeedsConfirmation { .. } => {
                tracing::debug!("identity quorum: needs confirmation")
            }
            Resolution::Unresolved { .. } => tracing::debug!("identity quorum: unresolved"),
        }

        // Tier-B downgrade (REQ-011): a clear quorum winner that rests on no
        // resolving hard identifier is still a guess — require confirmation rather
        // than auto-committing it (unless the user explicitly picked it).
        if let Resolution::Resolved { identity, .. } = &resolution {
            if !seed_has_hard_id(seed) && !identity_has_anchor(identity) && !seed.user_confirmed {
                resolution = Resolution::NeedsConfirmation {
                    candidates: candidates_from_responders(&responders, seed, &candidate_id),
                };
            }
        }

        // Language-silent downgrade (REQ-007/AC-011): every surviving responder
        // declared no usable language, and the work's own language sits outside
        // the install default — there is no positive evidence this resolution is
        // even in the right language, so it is a guess like any other Tier-B
        // candidate rather than an auto-apply. Independent of anchor strength:
        // an anchored winner is downgraded here too, unlike the Tier-B check above.
        // "Silent" has one definition module-wide: nothing usable after
        // normalization — a declared-but-unparseable value is silent here for
        // the same reason it cannot veto above.
        if let Resolution::Resolved { .. } = &resolution {
            let all_silent = responders
                .values()
                .all(|d| norm_lang(d.language.as_deref()).is_none());
            let off_default_silent = matches!(
                identity_matching::language_verdict(
                    norm_lang(seed.language.as_deref()).as_deref(),
                    None,
                    &self.config.default_language_source,
                ),
                identity_matching::LanguageVerdict::Grey
            );
            if all_silent && off_default_silent {
                resolution = Resolution::NeedsConfirmation {
                    candidates: candidates_from_responders(&responders, seed, &candidate_id),
                };
            }
        }

        attach_candidate_id(&mut resolution, candidate_id, user_id);
        Ok(resolution)
    }
}

impl LiveEnglishIdentityResolver {
    /// REQ-008 provider matrix scoped by seed + tier + prerequisites; never narrows
    /// a multi-eligible seed to a single provider (the #97 guard). Excludes any
    /// provider lacking its prerequisite (GB key per ST-009, Audnexus on a
    /// non-background tier per REQ-021). Goodreads carries no prerequisite
    /// (REQ-012/D6): its matching is deterministic (junk filter + shared 0.75
    /// picker + explicit abstain), so it participates in identity for every
    /// install, LLM or not.
    pub fn select_providers(&self, seed: &WorkSeed, tier: LatencyTier) -> Vec<MetadataProvider> {
        let mut out = Vec::new();

        // Ebook / print axis: an ISBN, a native ebook key, or title+author reaches
        // OpenLibrary + Hardcover (work anchors), Google Books (edition metadata),
        // and Goodreads (deterministic match). Foreign-language metadata is
        // excluded at the merge layer (REQ-027), not here — anchor capture is
        // language-agnostic.
        let ebook_axis = seed.isbn_13.is_some()
            || seed.ol_key.is_some()
            || seed.gr_key.is_some()
            || seed.hc_key.is_some()
            || (seed.title.is_some() && seed.author_name.is_some());
        if ebook_axis {
            out.push(MetadataProvider::OpenLibrary);
            out.push(MetadataProvider::Hardcover);
            if self.config.gb_key_present {
                out.push(MetadataProvider::GoogleBooks);
            }
            out.push(MetadataProvider::Goodreads);
        }

        // Audiobook axis: an ASIN reaches Audible interactively; Audnexus is
        // background-only (REQ-021). REQ-010: a seed without an ASIN but with
        // title+author still gets the leg, so add-time resolution can populate
        // CapturedIdentity.asin when deterministically resolvable — the
        // Audible client's own match guard and the quorum arbitrate; a fuzzy
        // hit is never adopted as identity.
        let audio_axis =
            seed.asin.is_some() || (seed.title.is_some() && seed.author_name.is_some());
        if audio_axis {
            out.push(MetadataProvider::Audible);
            if tier == LatencyTier::Background {
                out.push(MetadataProvider::Audnexus);
            }
        }

        out
    }
}

// ---------------------------------------------------------------------------
// Discovery helpers (module-level free functions; the resolver calls them and
// the behavioral tests target them directly).
// ---------------------------------------------------------------------------

/// Build an in-memory, never-persisted `Work` from the seed to drive
/// `ProviderClient::fetch` (which takes `&Work`) during pre-create discovery
/// (cR-004). It carries only discovery inputs and is never written to the DB.
pub fn build_transient_work_from_seed(seed: &WorkSeed, user_id: UserId) -> Work {
    Work {
        user_id,
        title: seed.title.clone().unwrap_or_default(),
        author_name: seed.author_name.clone().unwrap_or_default(),
        language: seed.language.clone(),
        isbn_13: seed.isbn_13.clone(),
        asin: seed.asin.clone(),
        ol_key: seed.ol_key.clone(),
        gr_key: seed.gr_key.clone(),
        hc_key: seed.hc_key.clone(),
        series_name: seed.series_name.clone(),
        year: seed.year,
        ..Default::default()
    }
}

/// Trust a non-harvested Goodreads key only by inspecting the payload the fetch
/// already returned (no extra network, REQ-014): require a populated title that
/// matches the resolved identity beyond the similarity threshold (REQ-024);
/// otherwise the key is not trusted. A title-less anti-bot payload always fails.
pub fn verify_gr_payload(payload: &NormalizedWorkDetail, captured: &CapturedIdentity) -> bool {
    let payload_title = match payload.title.as_deref() {
        Some(t) if !t.trim().is_empty() => t,
        _ => return false,
    };
    if captured.title.trim().is_empty() {
        return false;
    }
    let payload_author = payload.author_name.as_deref().unwrap_or("");
    let pt = identity_matching::parse_title(payload_title);
    let ct = identity_matching::parse_title(&captured.title);
    if identity_matching::title_verdict(&pt, &ct) != identity_matching::TitleVerdict::Same {
        return false;
    }
    // REQ-005c: an authorless side abstains rather than vetoing — agreement
    // then rests on the exact title equality already established above.
    matches!(
        identity_matching::author_verdict(
            &[payload_author.to_string()],
            std::slice::from_ref(&captured.author_name),
        ),
        identity_matching::AuthorVerdict::Agree | identity_matching::AuthorVerdict::Abstain
    )
}

/// Group responders by shared returned anchor or normalized title+author:
/// a majority cluster wins, a 1-vs-1 split with no majority is a `QuorumTie`,
/// and a single responder resolves trivially (REQ-018/020). The returned
/// `Resolved`/`Conflict` carries a placeholder `candidate_id`; `resolve` injects
/// the real one.
pub fn run_quorum(
    responders: &HashMap<MetadataProvider, NormalizedWorkDetail>,
    seed: &WorkSeed,
) -> Resolution {
    // Deterministic order: HashMap iteration is unordered, which would make the
    // representative pick (`max_by_key`) and the `merge_missing` order below
    // depend on hash layout — i.e. the captured identity could vary run-to-run
    // when providers disagree on secondary fields. Sort by provider so the
    // outcome is reproducible for a given input set.
    let mut entries: Vec<(&MetadataProvider, &NormalizedWorkDetail)> = responders.iter().collect();
    entries.sort_by_key(|(p, _)| format!("{p:?}"));
    let items: Vec<&NormalizedWorkDetail> = entries.into_iter().map(|(_, d)| d).collect();
    if items.is_empty() {
        let captured = captured_from_seed(seed);
        let provenance = provenance_all_hard(&captured);
        return Resolution::Unresolved {
            captured,
            reason: PendingReason::NoCandidates,
            candidate_id: None,
            provenance,
        };
    }

    // Transitive-closure clustering by the agreement relation (cheap for the
    // small provider set).
    let n = items.len();
    let mut assigned = vec![false; n];
    let mut clusters: Vec<Vec<usize>> = Vec::new();
    for i in 0..n {
        if assigned[i] {
            continue;
        }
        assigned[i] = true;
        let mut members = vec![i];
        let mut changed = true;
        while changed {
            changed = false;
            for (j, done) in assigned.iter_mut().enumerate() {
                if !*done && members.iter().any(|&m| agree(items[m], items[j])) {
                    *done = true;
                    members.push(j);
                    changed = true;
                }
            }
        }
        clusters.push(members);
    }

    // A work anchor (ol/gr/hc) outranks an ISBN/ASIN bridge for *winning* the
    // quorum (REQ-018/020): when any anchored cluster exists, only anchored
    // clusters compete, so an anchorless cluster can *corroborate* an anchored
    // winner (its bridge is merged below) but can neither beat it nor tie it into
    // a false Conflict. When NO provider carries a work anchor, the anchorless
    // clusters compete normally and the winner is still `Resolved` — an ISBN-only
    // identity is *provisional*, not a non-identity: `derived_identity_status`
    // renders it `Provisional` (never a Confirmed lock), consistent with the
    // no-responder Tier-A path (`resolve()` above). "ISBN is a bridge, not a
    // lock" means Provisional, not Pending — so it is resolved, not held.
    let any_anchored = clusters
        .iter()
        .any(|c| c.iter().any(|&i| has_work_anchor(items[i])));
    let mut competing: Vec<Vec<usize>> = if any_anchored {
        clusters
            .into_iter()
            .filter(|c| c.iter().any(|&i| has_work_anchor(items[i])))
            .collect()
    } else {
        clusters
    };

    competing.sort_by_key(|m| std::cmp::Reverse(m.len()));
    let top = &competing[0];
    let no_majority = competing.len() > 1 && competing[1].len() == top.len();
    if no_majority {
        let tie_len = top.len();
        // Q-008/AC-018: project every tied cluster with the same most-anchored
        // projection the winning path uses, so a settled-work contradiction on a
        // non-representative cluster — or on a cluster whose provider-sorted-first
        // member is a bridge — stays detectable (R-001). Winner/clustering unchanged.
        let tied: Vec<CapturedIdentity> = competing
            .iter()
            .take_while(|c| c.len() == tie_len)
            .map(|c| project_cluster(c, &items, seed).0)
            .collect();
        let rep_idx = *top
            .iter()
            .max_by_key(|&&i| anchor_count(items[i]))
            .expect("non-empty cluster");
        let rep = items[rep_idx];
        return Resolution::Conflict {
            conflict: quorum_tie_conflict(rep, seed),
            captured: project_cluster(top, &items, seed).0,
            tied,
        };
    }

    // The winning cluster projects to its most-anchored member plus every
    // anchor/bridge the other members contribute (convergence adds, never
    // clobbers) — the same projection the tie branch uses (R-001).
    let (identity, provenance) = project_cluster(top, &items, seed);
    Resolution::Resolved {
        identity,
        method: method_for_seed(seed),
        candidate_id: CandidateId(String::new()),
        provenance,
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

fn seed_has_hard_id(seed: &WorkSeed) -> bool {
    seed.isbn_13.is_some()
        || seed.asin.is_some()
        || seed.ol_key.is_some()
        || seed.gr_key.is_some()
        || seed.hc_key.is_some()
}

fn seed_has_signal(seed: &WorkSeed) -> bool {
    seed_has_hard_id(seed) || (seed.title.is_some() && seed.author_name.is_some())
}

fn identity_has_anchor(c: &CapturedIdentity) -> bool {
    c.ol_key.is_some() || c.gr_key.is_some() || c.hc_key.is_some()
}

fn method_for_seed(seed: &WorkSeed) -> IdentityMethod {
    if seed.user_confirmed {
        IdentityMethod::UserSelected
    } else if seed_has_hard_id(seed) {
        IdentityMethod::IsbnDirect
    } else {
        IdentityMethod::TitleAuthorSearch
    }
}

/// A payload carries a *work* anchor (ol/gr/hc) — the only kind that votes on
/// work identity in the quorum. An edition bridge (isbn/asin) alone does not: an
/// ISBN is a bridge to an anchor, never a lock on its own (REQ-018/020).
fn has_work_anchor(d: &NormalizedWorkDetail) -> bool {
    d.ol_key.as_deref().is_some_and(|v| !v.trim().is_empty())
        || d.gr_key.as_deref().is_some_and(|v| !v.trim().is_empty())
        || d.hc_key.as_deref().is_some_and(|v| !v.trim().is_empty())
}

fn anchor_count(d: &NormalizedWorkDetail) -> usize {
    d.ol_key.as_deref().is_some_and(|v| !v.trim().is_empty()) as usize
        + d.gr_key.as_deref().is_some_and(|v| !v.trim().is_empty()) as usize
        + d.hc_key.as_deref().is_some_and(|v| !v.trim().is_empty()) as usize
}

/// Two provider results agree if they returned the same work anchor of any
/// type, or the same normalized title + a shared author token (edition
/// variance corroborates; a genuinely different work does not — REQ-018).
///
/// A payload with no author ABSTAINS from the author comparison rather than
/// vetoing it (the REQ-025 missing-data principle, #148): agreement then
/// requires the stricter bar of exact normalized-title equality, so an
/// authorless provider can corroborate the same title but a fuzzy variant
/// cannot ride in on a missing field.
///
/// A shared edition bridge (ISBN/ASIN) corroborates ONLY in the absence of
/// contradicting text evidence (#148): it clusters a key-only payload (e.g.
/// an OL detail fetched by that very ISBN) with its edition-mates, but a
/// shared ISBN carrying flatly disagreeing titles is an ISBN collision —
/// AC-020 surfaces that as a conflict, never a silent merge.
fn agree(a: &NormalizedWorkDetail, b: &NormalizedWorkDetail) -> bool {
    let idv = identity_matching::id_verdict(&id_evidence(a), &id_evidence(b));
    if idv == identity_matching::IdVerdict::WorkKeyEqual {
        return true;
    }
    // Provably different identities never merge on a fuzzy title: the SAME anchor
    // type carrying DIFFERENT values means two distinct works. Mirrors the AC-020
    // ISBN-collision guard, extended to work keys.
    if idv == identity_matching::IdVerdict::WorkKeyContradiction {
        return false;
    }

    // REQ-007: two payloads that each declare a language, and it differs
    // after normalization, can never describe the same work — the
    // language-dimension analog of the work-key contradiction above. (The
    // work-vs-established-language veto and the language-silent grey rule
    // live at the `resolve()` level, where the work's own language is in
    // scope; this is the narrower payload-vs-payload check available here.)
    if languages_conflict(a.language.as_deref(), b.language.as_deref()) {
        return false;
    }

    let pa = identity_matching::parse_title(a.title.as_deref().unwrap_or(""));
    let pb = identity_matching::parse_title(b.title.as_deref().unwrap_or(""));
    let tv = identity_matching::title_verdict(&pa, &pb);

    // Payload series positions participate in the volume VETO only: a bare
    // "Alpha" payload must never corroborate an "Alpha" that carries a
    // conflicting volume (e.g. a GR pick whose search-card decoration said
    // #3 while its title was stripped to the bare form). One-sided position
    // evidence deliberately does NOT demote an equal-main pair — most
    // providers omit positions, and demotion would strip corroboration from
    // correct clusters wholesale.
    if identity_matching::title_verdict_with_positions(
        &pa,
        a.series_position,
        &pb,
        b.series_position,
    ) == identity_matching::TitleVerdict::VetoVolume
    {
        return false;
    }

    let author_a = a.author_name.as_deref().unwrap_or("");
    let author_b = b.author_name.as_deref().unwrap_or("");
    let av = identity_matching::author_verdict(&[author_a.to_string()], &[author_b.to_string()]);

    // Text agreement (REQ-004/005): exact main-title equality plus author
    // agreement is the only auto-same path. An authorless side abstains and
    // falls back to the stricter bar of exact title equality, which `Same`
    // already satisfies (REQ-005c) — kept in this form since #148 predates
    // this rewrite: authorless corroboration already required exact titles.
    let text_agrees = tv == identity_matching::TitleVerdict::Same
        && matches!(
            av,
            identity_matching::AuthorVerdict::Agree | identity_matching::AuthorVerdict::Abstain
        );
    if text_agrees {
        return true;
    }

    // Edition bridge fallback (#148/AC-020): a shared ISBN/ASIN still bridges
    // an ambiguous pair (no title on one side, or mains too dissimilar to
    // resolve as unrelated-but-not-contradicting) — but a flat contradiction
    // (both sides titled, and either clearly different or a volume conflict)
    // is never rescued: that is the ISBN-collision shape, never a silent merge.
    let a_title_empty = a.title.as_deref().unwrap_or("").trim().is_empty();
    let b_title_empty = b.title.as_deref().unwrap_or("").trim().is_empty();
    let titles_contradict = !a_title_empty
        && !b_title_empty
        && matches!(
            tv,
            identity_matching::TitleVerdict::Different
                | identity_matching::TitleVerdict::VetoVolume
        );
    idv == identity_matching::IdVerdict::EditionBridge && !titles_contradict
}

fn opt_eq(a: &Option<String>, b: &Option<String>) -> bool {
    matches!((a, b), (Some(x), Some(y)) if x == y)
}

/// Strip a blank anchor value to `None`. A `Some("")` (or whitespace-only)
/// value would be counted by `anchor_count` yet rejected by `confirm_anchor`
/// (`InvalidAnchorValue` — it checks `value.trim().is_empty()`), which aborts
/// the settle. Stripping it at the projection point guarantees no projected
/// `CapturedIdentity` ever carries a blank anchor.
fn non_blank(value: Option<String>) -> Option<String> {
    value.filter(|s| !s.trim().is_empty())
}

fn captured_from_seed(seed: &WorkSeed) -> CapturedIdentity {
    CapturedIdentity {
        ol_key: non_blank(seed.ol_key.clone()),
        gr_key: non_blank(seed.gr_key.clone()),
        hc_key: non_blank(seed.hc_key.clone()),
        isbn_13: non_blank(seed.isbn_13.clone()),
        asin: non_blank(seed.asin.clone()),
        title: seed.title.clone().unwrap_or_default(),
        author_name: seed.author_name.clone().unwrap_or_default(),
        language: seed.language.clone(),
    }
}

fn captured_from_detail(d: &NormalizedWorkDetail, seed: &WorkSeed) -> CapturedIdentity {
    use livrarr_domain::normalization::{normalize_asin, normalize_gr_key, AsinNorm};
    CapturedIdentity {
        ol_key: non_blank(d.ol_key.clone()),
        gr_key: non_blank(d.gr_key.clone()).and_then(|k| normalize_gr_key(&k)),
        hc_key: non_blank(d.hc_key.clone()),
        isbn_13: non_blank(d.isbn_13.clone()).or_else(|| non_blank(seed.isbn_13.clone())),
        asin: non_blank(d.asin.clone())
            .or_else(|| non_blank(seed.asin.clone()))
            .and_then(|a| match normalize_asin(&a) {
                AsinNorm::Asin(s) => Some(s),
                _ => None,
            }),
        title: d
            .title
            .clone()
            .or_else(|| seed.title.clone())
            .unwrap_or_default(),
        author_name: d
            .author_name
            .clone()
            .or_else(|| seed.author_name.clone())
            .unwrap_or_default(),
        language: d.language.clone().or_else(|| seed.language.clone()),
    }
}

/// Project a cluster to a single `CapturedIdentity` plus the per-anchor
/// [`AnchorProvenance`] describing how each anchor matched. The identity is
/// byte-identical to the pre-provenance projection (most-anchored member as the
/// base, then additive merge of every member's missing anchors — never
/// clobbering). Provenance records, per anchor, the [`MatchBasis`] of the FIRST
/// detail to contribute it; the basis is read from each RAW detail (never from
/// `cap`, whose isbn/asin may be seed-backfilled — reading `cap` would misread
/// the seed bridge as Hard). Extracted so the no-majority tie branch reuses it
/// (R-001).
fn project_cluster(
    cluster: &[usize],
    items: &[&NormalizedWorkDetail],
    seed: &WorkSeed,
) -> (CapturedIdentity, AnchorProvenance) {
    let rep_idx = *cluster
        .iter()
        .max_by_key(|&&i| anchor_count(items[i]))
        .expect("non-empty cluster");
    let mut cap = captured_from_detail(items[rep_idx], seed);
    let mut prov = AnchorProvenance::default();
    let rep_basis = basis_of(items[rep_idx], seed);
    if cap.ol_key.is_some() {
        prov.ol_key = Some(rep_basis);
    }
    if cap.gr_key.is_some() {
        prov.gr_key = Some(rep_basis);
    }
    if cap.hc_key.is_some() {
        prov.hc_key = Some(rep_basis);
    }
    if cap.isbn_13.is_some() {
        prov.isbn_13 = Some(rep_basis);
    }
    if cap.asin.is_some() {
        prov.asin = Some(rep_basis);
    }
    for &i in cluster {
        let had_ol = cap.ol_key.is_some();
        let had_gr = cap.gr_key.is_some();
        let had_hc = cap.hc_key.is_some();
        let had_isbn = cap.isbn_13.is_some();
        let had_asin = cap.asin.is_some();
        cap.merge_missing(&captured_from_detail(items[i], seed));
        let basis = basis_of(items[i], seed);
        if !had_ol && cap.ol_key.is_some() {
            prov.ol_key = Some(basis);
        }
        if !had_gr && cap.gr_key.is_some() {
            prov.gr_key = Some(basis);
        }
        if !had_hc && cap.hc_key.is_some() {
            prov.hc_key = Some(basis);
        }
        if !had_isbn && cap.isbn_13.is_some() {
            prov.isbn_13 = Some(basis);
        }
        if !had_asin && cap.asin.is_some() {
            prov.asin = Some(basis);
        }
    }
    (cap, prov)
}

/// The [`MatchBasis`] of a single provider detail: `Hard` iff the RAW record
/// shares a hard identifier the seed already carries (an exact cross-reference),
/// otherwise `Fuzzy` (it matched on title/author only). Every anchor a given
/// detail contributes shares this basis. Reads the raw detail, NOT a
/// seed-backfilled `CapturedIdentity` (the safe/guessed split, REQ-003/004).
fn basis_of(detail: &NormalizedWorkDetail, seed: &WorkSeed) -> MatchBasis {
    let shares = opt_eq(&seed.ol_key, &detail.ol_key)
        || opt_eq(&seed.gr_key, &detail.gr_key)
        || opt_eq(&seed.hc_key, &detail.hc_key)
        || opt_eq(&seed.isbn_13, &detail.isbn_13)
        || opt_eq(&seed.asin, &detail.asin);
    if shares {
        MatchBasis::Hard
    } else {
        MatchBasis::Fuzzy
    }
}

/// Provenance for an identity captured directly from the seed (a user's pick or
/// the work's own seeded ids): every present anchor is `Hard` — a seed's own
/// identifiers are established, not fuzzy guesses.
fn provenance_all_hard(cap: &CapturedIdentity) -> AnchorProvenance {
    let hard = |v: &Option<String>| v.as_ref().map(|_| MatchBasis::Hard);
    AnchorProvenance {
        ol_key: hard(&cap.ol_key),
        gr_key: hard(&cap.gr_key),
        hc_key: hard(&cap.hc_key),
        isbn_13: hard(&cap.isbn_13),
        asin: hard(&cap.asin),
    }
}

fn incoming_from_detail(d: &NormalizedWorkDetail, seed: &WorkSeed) -> IncomingConflictPayload {
    let c = captured_from_detail(d, seed);
    IncomingConflictPayload {
        ol_key: c.ol_key,
        gr_key: c.gr_key,
        hc_key: c.hc_key,
        isbn_13: c.isbn_13,
        asin: c.asin,
        title: c.title,
        author_name: c.author_name,
        year: d.year.or(seed.year),
        cover_url: d.cover_url.clone(),
        top_candidates: Vec::new(),
    }
}

/// A provider quorum tie predates any existing Work, so `existing_work_id`/`user_id`
/// are placeholders here; `resolve` fills in the real `user_id`.
fn quorum_tie_conflict(rep: &NormalizedWorkDetail, seed: &WorkSeed) -> NewIdentityConflict {
    NewIdentityConflict {
        user_id: 0,
        existing_work_id: 0,
        kind: IdentityConflictKind::QuorumTie,
        incoming: incoming_from_detail(rep, seed),
        raised_by: ConflictSource::ManualAdd,
        raised_source_path: None,
    }
}

fn candidates_from_responders(
    responders: &HashMap<MetadataProvider, NormalizedWorkDetail>,
    seed: &WorkSeed,
    candidate_id: &CandidateId,
) -> Vec<Candidate> {
    let seed_title = identity_matching::parse_title(seed.title.as_deref().unwrap_or(""));
    responders
        .iter()
        .map(|(provider, detail)| {
            // REQ-010: a genuinely computed similarity against what the seed
            // was looking for, not the hardcoded 1.0 every candidate used to
            // carry regardless of how well it actually matched.
            let detail_title =
                identity_matching::parse_title(detail.title.as_deref().unwrap_or(""));
            let title_jaccard = match identity_matching::title_verdict(&seed_title, &detail_title) {
                identity_matching::TitleVerdict::Same => 1.0,
                identity_matching::TitleVerdict::Grey { score } => score,
                identity_matching::TitleVerdict::Different
                | identity_matching::TitleVerdict::VetoVolume => 0.0,
            };
            Candidate {
                candidate_id: candidate_id.clone(),
                anchors: captured_from_detail(detail, seed),
                cover_url: detail.cover_url.clone(),
                sources: vec![*provider],
                score: ResolutionScore {
                    title_jaccard,
                    author_overlap: 0,
                    runner_up_delta: 0.0,
                },
                existing_work_id: None,
            }
        })
        .collect()
}

/// Inject the real cache handle into a verdict produced before the id was minted,
/// and stamp the real `user_id` onto a quorum-tie conflict.
fn attach_candidate_id(resolution: &mut Resolution, candidate_id: CandidateId, user_id: UserId) {
    match resolution {
        Resolution::Resolved {
            candidate_id: c, ..
        } => *c = candidate_id,
        Resolution::Unresolved {
            candidate_id: c, ..
        } => *c = Some(candidate_id),
        Resolution::NeedsConfirmation { candidates } => {
            for c in candidates.iter_mut() {
                c.candidate_id = candidate_id.clone();
            }
        }
        Resolution::Conflict { conflict, .. } => conflict.user_id = user_id,
    }
}

/// Project a provider payload's identifier fields into the authority's
/// evidence shape for [`identity_matching::id_verdict`].
fn id_evidence(d: &NormalizedWorkDetail) -> identity_matching::IdEvidence<'_> {
    identity_matching::IdEvidence {
        ol_key: d.ol_key.as_deref(),
        gr_key: d.gr_key.as_deref(),
        hc_key: d.hc_key.as_deref(),
        isbn_13: d.isbn_13.as_deref(),
        asin: d.asin.as_deref(),
    }
}

/// Normalize via the shared reconciler (ST-08) before any language compare —
/// GB's ISO codes and GR's English names already reconcile through it, and
/// this guarantees a correct compare even if some upstream write path stored
/// an unnormalized value in `works.language`.
fn norm_lang(raw: Option<&str>) -> Option<String> {
    raw.and_then(normalize_language)
}

/// Both sides declare a language, and it differs after normalization
/// (REQ-007) — the language-dimension analog of the work-key contradiction
/// in [`agree`]. Reuses the shared reconciler; never reimplements it.
fn languages_conflict(a: Option<&str>, b: Option<&str>) -> bool {
    match (norm_lang(a), norm_lang(b)) {
        (Some(x), Some(y)) => x != y,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detail(
        title: &str,
        author: Option<&str>,
        ol_key: Option<&str>,
        hc_key: Option<&str>,
    ) -> NormalizedWorkDetail {
        NormalizedWorkDetail {
            title: Some(title.to_string()),
            author_name: author.map(str::to_string),
            ol_key: ol_key.map(str::to_string),
            hc_key: hc_key.map(str::to_string),
            ..Default::default()
        }
    }

    fn seed(title: &str, author: &str) -> WorkSeed {
        WorkSeed {
            ol_key: None,
            gr_key: None,
            hc_key: None,
            isbn_13: Some("9780000000002".to_string()),
            asin: None,
            title: Some(title.to_string()),
            author_name: Some(author.to_string()),
            language: Some("en".to_string()),
            series_name: None,
            year: None,
            user_confirmed: false,
        }
    }

    // #148: an authorless provider answer (Hardcover) corroborates an
    // identically-titled answer instead of vetoing it into a quorum tie.
    #[test]
    fn authorless_responder_agrees_on_exact_normalized_title() {
        let hc = detail("Summer Knight", None, None, Some("341498"));
        let ol = detail("Summer Knight", Some("Jim Butcher"), Some("OL123W"), None);
        assert!(agree(&hc, &ol));
    }

    // Payload positions feed the volume veto: a bare-titled payload whose
    // series_position contradicts the other side's must never corroborate —
    // the GR-picker shape where a later volume's stripped title reads as a
    // twin while the volume evidence rides the position field.
    #[test]
    fn conflicting_payload_positions_veto_agreement() {
        let mut gr = detail("Alpha", Some("Ann Author"), None, None);
        gr.series_position = Some(3.0);
        let mut ol = detail("Alpha", Some("Ann Author"), Some("OL1W"), None);
        ol.series_position = Some(1.0);
        assert!(!agree(&gr, &ol));
    }

    // One-sided position evidence must NOT demote an equal-main pair — most
    // providers omit positions; demotion would strip corroboration from
    // correct clusters wholesale.
    #[test]
    fn one_sided_payload_position_still_agrees() {
        let mut gr = detail("Storm Front", Some("Jim Butcher"), None, None);
        gr.series_position = Some(1.0);
        let ol = detail("Storm Front", Some("Jim Butcher"), Some("OL2W"), None);
        assert!(agree(&gr, &ol));
    }

    // A conflicting position also blocks the edition-bridge rescue: shared
    // ISBN plus contradicting volumes is the collision shape (AC-020), never
    // a silent merge.
    #[test]
    fn edition_bridge_does_not_rescue_conflicting_positions() {
        let mut a = detail("Alpha", Some("Ann Author"), None, None);
        a.isbn_13 = Some("9780000000010".to_string());
        a.series_position = Some(2.0);
        let mut b = detail("Alpha", Some("Ann Author"), None, None);
        b.isbn_13 = Some("9780000000010".to_string());
        b.series_position = Some(3.0);
        assert!(!agree(&a, &b));
    }

    // The abstention path holds the STRICTER bar: a non-colon title variant
    // that would pass the jaccard gate with an author present does not
    // corroborate an authorless payload.
    #[test]
    fn authorless_responder_requires_exact_title_not_fuzzy() {
        let hc = detail("Summer Knight", None, None, Some("341498"));
        let variant = detail(
            "Summer Knight, Book 4",
            Some("Jim Butcher"),
            Some("OL999W"),
            None,
        );
        assert!(!agree(&hc, &variant));
    }

    #[test]
    fn authored_responders_keep_the_author_gate() {
        let a = detail("Hunger", Some("Knut Hamsun"), Some("OL1W"), None);
        let b = detail("Hunger", Some("Roxane Gay"), Some("OL2W"), None);
        assert!(!agree(&a, &b));
    }

    // The live 2026-06-11 refresh shape: OL responds key-only (no title, no
    // author — the detail payload pre-title-fix) but shares the ISBN it was
    // resolved from with HC/GB. Bridge equality must cluster it so the
    // ol_key reaches the captured identity.
    #[test]
    fn quorum_captures_ol_key_from_keyonly_payload_via_isbn_bridge() {
        let isbn = "9780451458926";
        let mut responders = HashMap::new();
        responders.insert(
            MetadataProvider::OpenLibrary,
            NormalizedWorkDetail {
                ol_key: Some("OL85586W".to_string()),
                isbn_13: Some(isbn.to_string()),
                ..Default::default()
            },
        );
        responders.insert(
            MetadataProvider::Hardcover,
            NormalizedWorkDetail {
                title: Some("Summer Knight".to_string()),
                author_name: Some("Jim Butcher".to_string()),
                hc_key: Some("341498".to_string()),
                isbn_13: Some(isbn.to_string()),
                ..Default::default()
            },
        );
        responders.insert(
            MetadataProvider::GoogleBooks,
            NormalizedWorkDetail {
                title: Some("Summer Knight".to_string()),
                author_name: Some("Jim Butcher".to_string()),
                isbn_13: Some(isbn.to_string()),
                ..Default::default()
            },
        );

        match run_quorum(&responders, &seed("Summer Knight", "Jim Butcher")) {
            Resolution::Resolved { identity, .. } => {
                assert_eq!(identity.ol_key.as_deref(), Some("OL85586W"));
                assert_eq!(identity.hc_key.as_deref(), Some("341498"));
                assert_eq!(identity.isbn_13.as_deref(), Some(isbn));
            }
            other => panic!("expected Resolved, got {other:?}"),
        }
    }

    // AC-020 protection survives bridge equality: a shared ISBN carrying
    // flatly disagreeing titles is an ISBN collision — never an agreement.
    #[test]
    fn shared_isbn_with_contradicting_titles_does_not_agree() {
        let a = NormalizedWorkDetail {
            title: Some("Dune".to_string()),
            author_name: Some("Frank Herbert".to_string()),
            ol_key: Some("OL-DUNE".to_string()),
            isbn_13: Some("9780441013593".to_string()),
            ..Default::default()
        };
        let b = NormalizedWorkDetail {
            title: Some("Different Book".to_string()),
            author_name: Some("Other Author".to_string()),
            hc_key: Some("HC-OTHER".to_string()),
            isbn_13: Some("9780441013593".to_string()),
            ..Default::default()
        };
        assert!(!agree(&a, &b));
    }

    // C1 (historical): the deleted private `normalize_match_title` truncated at
    // ':' — two different books in the same series ("The Dresden Files: Summer
    // Knight" vs "The Dresden Files: Dead Beat") both normalized to "the dresden
    // files", making `agree()` cluster them together. That matcher no longer
    // exists (REQ-001/AC-002); this is now a plain regression guard: same-series,
    // different-subtitle entries must never cluster — neither with distinct OL
    // anchors (work-key contradiction) nor with no anchors at all (the text
    // path alone).
    #[test]
    fn c1_colon_truncation_causes_same_series_cross_book_clustering() {
        let summer_knight = detail(
            "The Dresden Files: Summer Knight",
            Some("Jim Butcher"),
            Some("OL_SUMMER"),
            None,
        );
        let dead_beat = detail(
            "The Dresden Files: Dead Beat",
            Some("Jim Butcher"),
            Some("OL_DEAD_BEAT"),
            None,
        );
        // Different books with different OL anchors: the work-key contradiction
        // vetoes outright.
        assert!(
            !agree(&summer_knight, &dead_beat),
            "agree() must never cluster two different Dresden Files books"
        );

        // Anchor-less variant: no work keys, no bridges — the text path alone
        // decides. Equal mains ("dresden files") with disagreeing true
        // subtitles demote to grey, and grey never clusters.
        let summer_knight_bare = detail(
            "The Dresden Files: Summer Knight",
            Some("Jim Butcher"),
            None,
            None,
        );
        let dead_beat_bare = detail(
            "The Dresden Files: Dead Beat",
            Some("Jim Butcher"),
            None,
            None,
        );
        assert!(
            !agree(&summer_knight_bare, &dead_beat_bare),
            "the text path alone must never cluster same-series different-subtitle books"
        );
    }

    // A bare-key payload sharing NOTHING (no title, no bridge) still cannot
    // cluster — a lone unverifiable key never rides into the identity.
    #[test]
    fn keyonly_payload_without_shared_bridge_stays_unclustered() {
        let a = NormalizedWorkDetail {
            gr_key: Some("10266".to_string()),
            ..Default::default()
        };
        let b = detail("Summer Knight", Some("Jim Butcher"), Some("OL85586W"), None);
        assert!(!agree(&a, &b));
    }

    // The #148 shape end to end: HC (hc_key, authorless) + OL (ol_key,
    // authored), same title — one cluster, Resolved, both anchors captured.
    #[test]
    fn quorum_resolves_authorless_hc_with_ol_pair() {
        let mut responders = HashMap::new();
        responders.insert(
            MetadataProvider::Hardcover,
            detail("Summer Knight", None, None, Some("341498")),
        );
        responders.insert(
            MetadataProvider::OpenLibrary,
            detail("Summer Knight", Some("Jim Butcher"), Some("OL123W"), None),
        );

        match run_quorum(&responders, &seed("Summer Knight", "Jim Butcher")) {
            Resolution::Resolved { identity, .. } => {
                assert_eq!(identity.ol_key.as_deref(), Some("OL123W"));
                assert_eq!(identity.hc_key.as_deref(), Some("341498"));
            }
            other => panic!("expected Resolved, got {other:?}"),
        }
    }

    // --- AC-021: work-key-vs-edition arbitration (REQ-006) ---

    // ISBN equal + same-provider work keys (both `ol_key`) different: the
    // work-key contradiction outranks the edition-id agreement outright — a
    // collision surfaced as Conflict, never an auto-merge.
    #[test]
    fn ac021_isbn_equal_but_work_keys_differ_is_conflict_not_merge() {
        let isbn = "9780000000401";
        let mut responders = HashMap::new();
        responders.insert(
            MetadataProvider::OpenLibrary,
            NormalizedWorkDetail {
                title: Some("Arbitration Case".to_string()),
                author_name: Some("Case Author".to_string()),
                ol_key: Some("OL-ARB-A".to_string()),
                isbn_13: Some(isbn.to_string()),
                ..Default::default()
            },
        );
        responders.insert(
            MetadataProvider::Hardcover,
            NormalizedWorkDetail {
                title: Some("Arbitration Case".to_string()),
                author_name: Some("Case Author".to_string()),
                ol_key: Some("OL-ARB-B".to_string()),
                isbn_13: Some(isbn.to_string()),
                ..Default::default()
            },
        );
        let resolution = run_quorum(&responders, &seed("Arbitration Case", "Case Author"));
        assert!(
            matches!(resolution, Resolution::Conflict { .. }),
            "AC-021: an ISBN match with contradicting same-provider work keys \
             must surface as a conflict, got {resolution:?}"
        );
    }

    // ASINs differing while all else (title, author, and a shared ol_key)
    // agrees carries zero penalty: an edition-level id's inequality is no
    // evidence and never vetoes.
    #[test]
    fn ac021_differing_asin_with_agreeing_everything_else_is_no_penalty() {
        let mut responders = HashMap::new();
        responders.insert(
            MetadataProvider::OpenLibrary,
            NormalizedWorkDetail {
                title: Some("Arbitration Case".to_string()),
                author_name: Some("Case Author".to_string()),
                ol_key: Some("OL-ARB".to_string()),
                asin: Some("B000ARBA1".to_string()),
                ..Default::default()
            },
        );
        responders.insert(
            MetadataProvider::Audible,
            NormalizedWorkDetail {
                title: Some("Arbitration Case".to_string()),
                author_name: Some("Case Author".to_string()),
                asin: Some("B000ARBA2".to_string()),
                ..Default::default()
            },
        );
        match run_quorum(&responders, &seed("Arbitration Case", "Case Author")) {
            Resolution::Resolved { identity, .. } => {
                assert_eq!(identity.ol_key.as_deref(), Some("OL-ARB"));
            }
            other => panic!(
                "AC-021: differing ASINs must carry zero penalty when everything \
                 else agrees, got {other:?}"
            ),
        }
    }

    // --- REQ-007 language dimension + REQ-010 real scores (resolve()-level) ---

    fn resolver_with(
        stubs: Vec<livrarr_external_data::StubProviderClient>,
        default_language_source: &str,
    ) -> LiveEnglishIdentityResolver {
        let clients = stubs
            .into_iter()
            .map(|s| (s.provider, ProviderClient::Stub(s)))
            .collect::<HashMap<_, _>>();
        LiveEnglishIdentityResolver {
            clients,
            cache: Arc::new(TransportCache::new(Duration::from_secs(30))),
            config: ResolverConfig {
                gb_key_present: false,
                default_language_source: default_language_source.to_string(),
                ..ResolverConfig::default()
            },
        }
    }

    fn plain_seed(title: &str, author: &str, language: Option<&str>) -> WorkSeed {
        WorkSeed {
            ol_key: None,
            gr_key: None,
            hc_key: None,
            isbn_13: None,
            asin: None,
            title: Some(title.to_string()),
            author_name: Some(author.to_string()),
            language: language.map(str::to_string),
            series_name: None,
            year: None,
            user_confirmed: false,
        }
    }

    // AC-010: a French-declared payload never merges onto an English work even
    // with an identical main title — the language veto outranks title similarity.
    #[tokio::test]
    async fn ac010_declared_language_mismatch_never_merges_onto_work() {
        let seed = plain_seed("Dune", "Frank Herbert", Some("en"));
        let ol = livrarr_external_data::StubProviderClient::new(
            MetadataProvider::OpenLibrary,
            ProviderOutcome::Success(Box::new(NormalizedWorkDetail {
                title: Some("Dune".to_string()),
                author_name: Some("Frank Herbert".to_string()),
                ol_key: Some("OL-DUNE-FR".to_string()),
                language: Some("fr".to_string()),
                ..NormalizedWorkDetail::default()
            })),
        );
        let resolver = resolver_with(vec![ol], "en");

        let resolution = resolver
            .resolve(1, &seed, LatencyTier::Interactive)
            .await
            .expect("resolve");

        assert!(
            !matches!(resolution, Resolution::Resolved { .. }),
            "AC-010: a French-declared payload must never merge onto an English \
             work, got {resolution:?}"
        );
    }

    // AC-011: a language-silent payload on a work outside the install default
    // lands grey (NeedsConfirmation), never auto-applies — even though the
    // payload carries a work anchor and would otherwise auto-confirm.
    #[tokio::test]
    async fn ac011_language_silent_payload_on_non_default_work_lands_grey() {
        let seed = plain_seed(
            "Le Petit Prince",
            "Antoine de Saint-Exupery",
            Some("fr"), // the work's own language: not the install default below
        );
        let ol = livrarr_external_data::StubProviderClient::new(
            MetadataProvider::OpenLibrary,
            ProviderOutcome::Success(Box::new(NormalizedWorkDetail {
                title: Some("Le Petit Prince".to_string()),
                author_name: Some("Antoine de Saint-Exupery".to_string()),
                ol_key: Some("OL-PETIT-PRINCE".to_string()),
                language: None, // silent
                ..NormalizedWorkDetail::default()
            })),
        );
        let resolver = resolver_with(vec![ol], "en");

        let resolution = resolver
            .resolve(1, &seed, LatencyTier::Interactive)
            .await
            .expect("resolve");

        assert!(
            matches!(resolution, Resolution::NeedsConfirmation { .. }),
            "AC-011: a language-silent payload on a non-default-language work \
             must land grey, got {resolution:?}"
        );
    }

    // REQ-010: a NeedsConfirmation candidate carries a genuinely computed
    // title-similarity score, not the old hardcoded 1.0 — a near (not exact)
    // title must land strictly between the grey floor and 1.0.
    #[tokio::test]
    async fn real_score_population_computes_actual_similarity_not_hardcoded_one() {
        let seed = plain_seed("The Wise Man's Fear", "Patrick Rothfuss", Some("en"));
        let ol = livrarr_external_data::StubProviderClient::new(
            MetadataProvider::OpenLibrary,
            ProviderOutcome::Success(Box::new(NormalizedWorkDetail {
                title: Some("The Wise Man's Fear Chronicle".to_string()),
                author_name: Some("Patrick Rothfuss".to_string()),
                language: Some("en".to_string()),
                ..NormalizedWorkDetail::default()
            })),
        );
        let resolver = resolver_with(vec![ol], "en");

        let resolution = resolver
            .resolve(1, &seed, LatencyTier::Interactive)
            .await
            .expect("resolve");

        match resolution {
            Resolution::NeedsConfirmation { candidates } => {
                assert_eq!(candidates.len(), 1);
                let score = candidates[0].score.title_jaccard;
                assert!(
                    (identity_matching::TITLE_GREY_FLOOR..1.0).contains(&score),
                    "REQ-010: expected a genuinely computed near-match score, got {score}"
                );
            }
            other => panic!("expected NeedsConfirmation with a real score, got {other:?}"),
        }
    }
}
