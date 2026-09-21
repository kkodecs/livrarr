//! Facts a search result supplied for a Work at creation time, and the typed
//! source references that preserve provider context without becoming identity
//! routes.
//!
//! [`SelectedFacts`] is the wire object a discovery card carries and the
//! browser echoes verbatim into the Add request. [`CreationFacts`] is the
//! persistence-ready form the creation door hands to the identity road: the
//! ordinary Work column values, one provenance row per populated value, and
//! the typed [`SourceReference`] rows. References are source facts only; they
//! never create Authors or identity routes.

use serde::{Deserialize, Serialize};

use crate::enrichment_types::{MetadataProvider, ProvenanceSetter, WorkField};

/// Everything useful a single provider's search result supplied, in the
/// provider's own terms. Every value is optional except the provider; an
/// absent value stays absent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectedFacts {
    pub provider: MetadataProvider,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    /// The provider's actual book language, never a search or default language.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Original publication year (a Work fact), never an edition date.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_year: Option<i32>,
    /// Original publication date verbatim (`YYYY`, `YYYY-MM` or `YYYY-MM-DD`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_publish_date: Option<String>,
    /// Edition publication date verbatim; describes one edition only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edition_publish_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// True when the provider marked its description as shortened.
    #[serde(default)]
    pub description_truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_count: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series_position: Option<f64>,
    #[serde(default)]
    pub genres: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating_count: Option<i32>,
    /// Raw provider cover address (never the browser proxy form).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_url: Option<String>,
    /// Undecorated title when the provider supplied it separately.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bare_title: Option<String>,
    /// The provider's decorated display title when it differs from the bare one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decorated_title: Option<String>,
    /// Every supplied contributor name in provider order, primary first.
    #[serde(default)]
    pub contributors: Vec<SelectedContributor>,
    /// Typed identifiers beyond the identity anchors the card already carries.
    #[serde(default)]
    pub references: Vec<SelectedReference>,
}

/// One credited contributor and, when supplied, the same provider's author id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectedContributor {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_author_id: Option<String>,
}

/// One typed identifier supplied by the selected result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectedReference {
    pub kind: SourceReferenceKind,
    pub value: String,
}

/// The namespaces a source reference can belong to. Book, Work, volume, ISBN
/// and author identifiers stay distinct; none of them is an identity route.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceReferenceKind {
    ContributorName,
    OpenLibraryAuthor,
    HardcoverAuthor,
    GoodreadsAuthor,
    OpenLibraryWork,
    HardcoverWork,
    GoodreadsBook,
    GoodreadsWork,
    GoogleVolume,
    #[serde(rename = "isbn_10")]
    Isbn10,
    #[serde(rename = "isbn_13")]
    Isbn13,
    Amazon,
    Subtitle,
    BareTitle,
    DecoratedTitle,
    CoverUrl,
    CoverUrlExplicit,
    UnclassifiedYear,
}

/// One stored source-reference row. `provider` is `None` for legacy input
/// that named no provider; `ordinal` keeps a contributor name and its
/// same-provider author id associated and orders repeated kinds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceReference {
    pub provider: Option<MetadataProvider>,
    pub kind: SourceReferenceKind,
    pub value: String,
    pub ordinal: i64,
}

/// The ordinary Work column values a creation door supplies. Absent stays
/// absent; nothing here is invented from defaults.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CreationFields {
    pub language: Option<String>,
    pub description: Option<String>,
    pub description_truncated: bool,
    pub year: Option<i32>,
    pub original_publish_date: Option<String>,
    pub publish_date: Option<String>,
    pub publisher: Option<String>,
    pub page_count: Option<i32>,
    pub series_name: Option<String>,
    pub series_position: Option<f64>,
    pub genres: Option<Vec<String>>,
    pub rating: Option<f64>,
    pub rating_count: Option<i32>,
    pub cover_url: Option<String>,
}

/// Provenance for one populated creation value. Copied provider facts use
/// `Provider` with the provider; supplied legacy input uses a source-free
/// setter. Never `User`: a copied fact is not a personal edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreationProvenance {
    pub field: WorkField,
    pub setter: ProvenanceSetter,
    pub source: Option<MetadataProvider>,
}

/// Everything the created branch of the settlement transaction writes
/// alongside the new Work row and its birth event.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CreationFacts {
    pub fields: CreationFields,
    pub provenance: Vec<CreationProvenance>,
    pub references: Vec<SourceReference>,
}

impl SourceReferenceKind {
    /// The same-provider author identifier namespace a provider supplies, if
    /// it supplies one. Google Books credits names only.
    pub fn author_kind_for(provider: MetadataProvider) -> Option<Self> {
        match provider {
            MetadataProvider::OpenLibrary => Some(Self::OpenLibraryAuthor),
            MetadataProvider::Hardcover => Some(Self::HardcoverAuthor),
            MetadataProvider::Goodreads => Some(Self::GoodreadsAuthor),
            _ => None,
        }
    }

    /// Whether a selected result may carry this kind in its `references`.
    /// The remaining kinds are derived by the creation door from dedicated
    /// fields (contributors, titles, cover intent, legacy year), never
    /// supplied as bare references.
    pub fn is_selectable_identifier(self) -> bool {
        matches!(
            self,
            Self::OpenLibraryWork
                | Self::HardcoverWork
                | Self::GoodreadsBook
                | Self::GoodreadsWork
                | Self::GoogleVolume
                | Self::Isbn10
                | Self::Isbn13
                | Self::Amazon
        )
    }
}

impl SelectedReference {
    /// Type one ISBN spelling by its digit count, keeping the spelling
    /// verbatim: thirteen digits is an ISBN-13, ten characters ending in a
    /// digit or `X` is an ISBN-10, and anything else cannot be typed.
    pub fn typed_isbn(raw: &str) -> Option<Self> {
        let value = raw.trim();
        let compact: Vec<char> = value.chars().filter(|c| !matches!(c, '-' | ' ')).collect();
        let kind = match compact.len() {
            13 if compact.iter().all(char::is_ascii_digit) => SourceReferenceKind::Isbn13,
            10 if compact[..9].iter().all(char::is_ascii_digit)
                && (compact[9].is_ascii_digit() || matches!(compact[9], 'X' | 'x')) =>
            {
                SourceReferenceKind::Isbn10
            }
            _ => return None,
        };
        Some(Self {
            kind,
            value: value.to_string(),
        })
    }
}
