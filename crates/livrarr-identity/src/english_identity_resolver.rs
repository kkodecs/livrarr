use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use livrarr_domain::identity::*;
use livrarr_domain::services::WorkIdentityError;
use livrarr_domain::{MetadataProvider, UserId, Work};
use uuid::Uuid;

use livrarr_external_data::provider_client::ProviderClient;
use livrarr_external_data::transport_cache::TransportCache;
use livrarr_external_data::{NormalizedWorkDetail, ProviderOutcome};

pub use livrarr_domain::identity::WorkSeed;
pub use livrarr_domain::services::IdentityResolver as EnglishIdentityResolver;

/// Minimum normalized-title Jaccard for two provider results (or a Goodreads
/// payload vs the resolved identity) to count as the same work. Edition variants
/// ("Dune" vs "Dune (Illustrated Edition)") normalize to the same tokens and pass;
/// a different work ("Dune" vs "Dune Messiah") falls below it.
const TITLE_MATCH_JACCARD: f64 = 0.75;

#[derive(Debug, Clone)]
pub struct ResolverConfig {
    pub confirm_title_jaccard: f64,
    pub confirm_runner_up_delta: f64,
    /// Per-provider call budget; a provider exceeding it abstains (REQ-025).
    pub call_timeout: Duration,
    /// A Google Books API key is configured (ST-009) — gates GB selection.
    pub gb_key_present: bool,
    /// An LLM is configured (ST-001) — gates Goodreads selection.
    pub llm_configured: bool,
}

impl Default for ResolverConfig {
    fn default() -> Self {
        Self {
            confirm_title_jaccard: 0.75,
            confirm_runner_up_delta: 0.10,
            call_timeout: Duration::from_secs(10),
            gb_key_present: false,
            llm_configured: false,
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
                    match tokio::time::timeout(timeout, client.fetch(&work)).await {
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

        attach_candidate_id(&mut resolution, candidate_id, user_id);
        Ok(resolution)
    }
}

impl LiveEnglishIdentityResolver {
    /// REQ-008 provider matrix scoped by seed + tier + prerequisites; never narrows
    /// a multi-eligible seed to a single provider (the #97 guard). Excludes any
    /// provider lacking its prerequisite (GB key per ST-009, LLM for GR per ST-001,
    /// Audnexus on a non-background tier per REQ-021).
    pub fn select_providers(&self, seed: &WorkSeed, tier: LatencyTier) -> Vec<MetadataProvider> {
        let mut out = Vec::new();

        // Ebook / print axis: an ISBN, a native ebook key, or title+author reaches
        // OpenLibrary + Hardcover (work anchors), Google Books (edition metadata),
        // and Goodreads (LLM-verified). Foreign-language metadata is excluded at the
        // merge layer (REQ-027), not here — anchor capture is language-agnostic.
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
            if self.config.llm_configured {
                out.push(MetadataProvider::Goodreads);
            }
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
    title_matches(payload_title, &captured.title)
        && author_matches(payload_author, &captured.author_name)
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
    if opt_eq(&a.ol_key, &b.ol_key) || opt_eq(&a.gr_key, &b.gr_key) || opt_eq(&a.hc_key, &b.hc_key)
    {
        return true;
    }
    // Provably different identities never merge on a fuzzy title: the SAME anchor
    // type carrying DIFFERENT values means two distinct works. Mirrors the AC-020
    // ISBN-collision guard, extended to work keys.
    if opt_differs(&a.ol_key, &b.ol_key)
        || opt_differs(&a.gr_key, &b.gr_key)
        || opt_differs(&a.hc_key, &b.hc_key)
    {
        return false;
    }
    let at = a.title.as_deref().unwrap_or("");
    let bt = b.title.as_deref().unwrap_or("");
    let aa = a.author_name.as_deref().unwrap_or("");
    let ba = b.author_name.as_deref().unwrap_or("");
    let sa = token_set(&aa.to_lowercase());
    let sb = token_set(&ba.to_lowercase());

    let text_agrees = if sa.is_empty() || sb.is_empty() {
        let na = normalize_match_title(at);
        let nb = normalize_match_title(bt);
        !na.is_empty() && na == nb
    } else {
        title_matches(at, bt) && author_matches(aa, ba)
    };
    if text_agrees {
        return true;
    }

    let titles_contradict = {
        let na = normalize_match_title(at);
        let nb = normalize_match_title(bt);
        !na.is_empty() && !nb.is_empty() && !title_matches(at, bt)
    };
    (opt_eq(&a.isbn_13, &b.isbn_13) || opt_eq(&a.asin, &b.asin)) && !titles_contradict
}

fn opt_eq(a: &Option<String>, b: &Option<String>) -> bool {
    matches!((a, b), (Some(x), Some(y)) if x == y)
}

/// Both present and unequal — a provable conflict (vs `opt_eq`, both present and
/// equal). Used to veto merging works whose same-type anchor holds different values.
fn opt_differs(a: &Option<String>, b: &Option<String>) -> bool {
    matches!((a, b), (Some(x), Some(y)) if x != y)
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
    responders
        .iter()
        .map(|(provider, detail)| Candidate {
            candidate_id: candidate_id.clone(),
            anchors: captured_from_detail(detail, seed),
            cover_url: detail.cover_url.clone(),
            sources: vec![*provider],
            score: ResolutionScore {
                title_jaccard: 1.0,
                author_overlap: 0,
                runner_up_delta: 0.0,
            },
            existing_work_id: None,
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

fn normalize_match_title(s: &str) -> String {
    let lower = s.to_lowercase();
    let mut out = String::new();
    let mut depth = 0i32;
    for ch in lower.chars() {
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = (depth - 1).max(0),
            ':' => break,
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn token_set(s: &str) -> HashSet<String> {
    s.split_whitespace().map(|t| t.to_string()).collect()
}

fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }
    let inter = a.intersection(b).count() as f64;
    let union = a.union(b).count() as f64;
    inter / union
}

fn title_matches(a: &str, b: &str) -> bool {
    let na = normalize_match_title(a);
    let nb = normalize_match_title(b);
    if na.is_empty() || nb.is_empty() {
        return false;
    }
    if na == nb {
        return true;
    }
    jaccard(&token_set(&na), &token_set(&nb)) >= TITLE_MATCH_JACCARD
}

fn author_matches(a: &str, b: &str) -> bool {
    let sa = token_set(&a.to_lowercase());
    let sb = token_set(&b.to_lowercase());
    if sa.is_empty() || sb.is_empty() {
        return false;
    }
    sa.intersection(&sb).next().is_some()
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

    // C1: `normalize_match_title` truncates at ':' — two different books in the
    // same series ("The Dresden Files: Summer Knight" vs "The Dresden Files:
    // Dead Beat") both normalize to "the dresden files", making `agree()` cluster
    // them together. This test proves the bug: same-series, different-subtitle
    // entries MUST NOT cluster when they have no shared anchor.
    #[test]
    fn c1_colon_truncation_causes_same_series_cross_book_clustering() {
        // Step 1: prove normalize_match_title collapses both to the same prefix.
        let summer_knight_normalized = normalize_match_title("The Dresden Files: Summer Knight");
        let dead_beat_normalized = normalize_match_title("The Dresden Files: Dead Beat");
        assert_eq!(
            summer_knight_normalized, dead_beat_normalized,
            "BUG CONFIRMED: both titles collapse to {:?} — subtitle is lost",
            summer_knight_normalized
        );

        // Step 2: prove agree() clusters two different books as a result.
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
        // These are different books with different OL anchors. agree() must return
        // false — but due to C1, the colon truncation makes title_matches() return
        // true, so agree() incorrectly returns true.
        assert!(
            !agree(&summer_knight, &dead_beat),
            "BUG CONFIRMED: agree() clustered two different Dresden Files books \
             because colon truncation collapsed both titles to 'the dresden files'"
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
}
