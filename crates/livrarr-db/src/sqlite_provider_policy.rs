use crate::sqlite::SqliteDb;
use crate::{DbError, ProviderPolicyDb};

use std::collections::HashMap;

use livrarr_domain::services::{
    ListKind, ProviderList, ProviderPolicy, ProviderPolicySnapshot, ProviderRef,
};
use livrarr_domain::MetadataProvider;

use crate::sqlite_common::{from_str, map_db_err};

impl ProviderPolicyDb for SqliteDb {
    async fn load_provider_policy_snapshot(
        &self,
    ) -> Result<livrarr_domain::services::ProviderPolicySnapshot, DbError> {
        let rows = sqlx::query_as::<_, (String, String, String, i64)>(
            "SELECT language, kind, provider, rank FROM provider_policy \
             ORDER BY language, kind, rank, provider",
        )
        .fetch_all(self.pool())
        .await
        .map_err(map_db_err)?;

        // Group rows into per-language (ebook, audiobook) entry lists.
        let mut grouped: HashMap<String, (Vec<ProviderRef>, Vec<ProviderRef>)> = HashMap::new();
        for (language, kind, provider, rank) in rows {
            let kind: ListKind = from_str(&kind)?;
            let provider: MetadataProvider = from_str(&provider)?;
            let rank = u8::try_from(rank).unwrap_or(u8::MAX);
            let entry = ProviderRef { provider, rank };
            let lists = grouped.entry(language).or_default();
            match kind {
                ListKind::Ebook => lists.0.push(entry),
                ListKind::Audiobook => lists.1.push(entry),
            }
        }

        let mut by_language: HashMap<String, ProviderPolicy> = HashMap::new();
        let mut generic = ProviderPolicy::default();
        for (language, (ebook, audiobook)) in grouped {
            let policy = ProviderPolicy {
                ebook: ProviderList::new(ebook).map_err(|e| DbError::IncompatibleData {
                    detail: e.to_string(),
                })?,
                audiobook: ProviderList::new(audiobook).map_err(|e| DbError::IncompatibleData {
                    detail: e.to_string(),
                })?,
            };
            if language == "*" {
                generic = policy;
            } else {
                by_language.insert(language, policy);
            }
        }

        Ok(ProviderPolicySnapshot {
            by_language,
            generic,
        })
    }
}
