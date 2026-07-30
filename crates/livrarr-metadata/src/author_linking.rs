use livrarr_db::{AuthorDb, AuthorLinkClaim, AuthorLinkDb, WorkDb};
use livrarr_domain::seed::dominant_language;
use livrarr_domain::services::{
    AuthorLinkService, AuthorLinkWorkflow, AuthorProviderGateway, AuthorServiceError,
};
use livrarr_domain::{
    AgreedAuthorRouteEvidence, Author, AuthorCompatibilityProjection, AuthorId,
    AuthorLinkCandidate, AuthorLinkError, AuthorLinkProgress, AuthorLinkProgressUpdate,
    AuthorLinkReview, AuthorLinkState, AuthorLinkTrigger, AuthorNameSource, AuthorNameVariant,
    AuthorRoute, AuthorRouteKey, AuthorSweepProgress, AuthorSweepTickSummary, OpenLibraryNameRole,
    RejectedAuthorRouteEvidence, RouteWriteOutcome, UserId,
};
use livrarr_external_data::language::{provider_priority, ProviderPriority};
use tokio_util::sync::CancellationToken;

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

impl<D, G> AuthorLinkingServiceImpl<D, G>
where
    D: AuthorLinkDb + AuthorDb + WorkDb,
    G: AuthorProviderGateway,
{
    pub async fn run_author(
        &self,
        claim: AuthorLinkClaim,
    ) -> Result<AuthorLinkProgressUpdate, AuthorLinkError> {
        todo!()
    }
}

impl<D, G> AuthorLinkService for AuthorLinkingServiceImpl<D, G>
where
    D: AuthorLinkDb + AuthorDb + WorkDb + Send + Sync,
    G: AuthorProviderGateway + Send + Sync,
{
    async fn list_review(&self, user_id: UserId) -> Result<Vec<AuthorLinkReview>, AuthorLinkError> {
        todo!()
    }

    async fn pick_candidate(
        &self,
        user_id: UserId,
        candidate_id: i64,
    ) -> Result<AuthorRoute, AuthorLinkError> {
        todo!()
    }

    async fn attach_selected_route(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        key: AuthorRouteKey,
    ) -> Result<AuthorRoute, AuthorLinkError> {
        todo!()
    }

    async fn dismiss_candidate(
        &self,
        user_id: UserId,
        candidate_id: i64,
    ) -> Result<(), AuthorLinkError> {
        todo!()
    }

    async fn remove_route(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        route_id: i64,
    ) -> Result<(), AuthorLinkError> {
        todo!()
    }

    async fn re_resolve(
        &self,
        user_id: UserId,
        author_id: AuthorId,
    ) -> Result<AuthorLinkProgress, AuthorLinkError> {
        todo!()
    }

    async fn progress(&self, user_id: UserId) -> Result<AuthorSweepProgress, AuthorLinkError> {
        todo!()
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
        todo!()
    }

    async fn submit_evidence(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        evidence: AgreedAuthorRouteEvidence,
    ) -> Result<RouteWriteOutcome, AuthorLinkError> {
        todo!()
    }

    async fn record_readarr_rejection(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        rejected: RejectedAuthorRouteEvidence,
    ) -> Result<AuthorLinkCandidate, AuthorLinkError> {
        todo!()
    }

    async fn run_due(
        &self,
        batch_size: u32,
        cancel: CancellationToken,
    ) -> Result<AuthorSweepTickSummary, AuthorLinkError> {
        todo!()
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
