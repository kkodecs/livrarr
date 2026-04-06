use sqlx::Row;

use crate::sqlite::SqliteDb;
use crate::sqlite_common::map_db_err;
use crate::{
    ConfigDb, DbError, LlmProvider, MediaManagementConfig, MetadataConfig, NamingConfig,
    ProwlarrConfig, UpdateMediaManagementConfigRequest, UpdateMetadataConfigRequest,
    UpdateProwlarrConfigRequest,
};

fn parse_llm_provider(s: &str) -> Option<LlmProvider> {
    match s {
        "groq" => Some(LlmProvider::Groq),
        "gemini" => Some(LlmProvider::Gemini),
        "openai" => Some(LlmProvider::Openai),
        "custom" => Some(LlmProvider::Custom),
        _ => None,
    }
}

fn llm_provider_str(p: &LlmProvider) -> &'static str {
    match p {
        LlmProvider::Groq => "groq",
        LlmProvider::Gemini => "gemini",
        LlmProvider::Openai => "openai",
        LlmProvider::Custom => "custom",
    }
}

fn parse_languages(s: &str) -> Vec<String> {
    serde_json::from_str(s).unwrap_or_else(|_| vec!["en".to_string()])
}

impl ConfigDb for SqliteDb {
    async fn get_naming_config(&self) -> Result<NamingConfig, DbError> {
        let row = sqlx::query("SELECT * FROM naming_config WHERE id = 1")
            .fetch_one(self.pool())
            .await
            .map_err(map_db_err)?;

        Ok(NamingConfig {
            author_folder_format: row
                .try_get("author_folder_format")
                .map_err(|e| DbError::Io(Box::new(e)))?,
            book_folder_format: row
                .try_get("book_folder_format")
                .map_err(|e| DbError::Io(Box::new(e)))?,
            rename_files: row
                .try_get::<bool, _>("rename_files")
                .map_err(|e| DbError::Io(Box::new(e)))?,
            replace_illegal_chars: row
                .try_get::<bool, _>("replace_illegal_chars")
                .map_err(|e| DbError::Io(Box::new(e)))?,
        })
    }

    async fn get_media_management_config(&self) -> Result<MediaManagementConfig, DbError> {
        let row = sqlx::query("SELECT * FROM media_management_config WHERE id = 1")
            .fetch_one(self.pool())
            .await
            .map_err(map_db_err)?;

        let ebook_json: String = row
            .try_get("preferred_ebook_formats")
            .map_err(|e| DbError::Io(Box::new(e)))?;
        let audiobook_json: String = row
            .try_get("preferred_audiobook_formats")
            .map_err(|e| DbError::Io(Box::new(e)))?;

        Ok(MediaManagementConfig {
            cwa_ingest_path: row
                .try_get("cwa_ingest_path")
                .map_err(|e| DbError::Io(Box::new(e)))?,
            preferred_ebook_formats: serde_json::from_str(&ebook_json)
                .map_err(|e| DbError::Io(Box::new(e)))?,
            preferred_audiobook_formats: serde_json::from_str(&audiobook_json)
                .map_err(|e| DbError::Io(Box::new(e)))?,
        })
    }

    async fn update_media_management_config(
        &self,
        req: UpdateMediaManagementConfigRequest,
    ) -> Result<MediaManagementConfig, DbError> {
        let ebook_json = serde_json::to_string(&req.preferred_ebook_formats)
            .map_err(|e| DbError::Io(Box::new(e)))?;
        let audiobook_json = serde_json::to_string(&req.preferred_audiobook_formats)
            .map_err(|e| DbError::Io(Box::new(e)))?;
        sqlx::query(
            "UPDATE media_management_config SET cwa_ingest_path = ?, \
             preferred_ebook_formats = ?, preferred_audiobook_formats = ? WHERE id = 1",
        )
        .bind(&req.cwa_ingest_path)
        .bind(&ebook_json)
        .bind(&audiobook_json)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;

        self.get_media_management_config().await
    }

    async fn get_prowlarr_config(&self) -> Result<ProwlarrConfig, DbError> {
        let row = sqlx::query("SELECT * FROM prowlarr_config WHERE id = 1")
            .fetch_one(self.pool())
            .await
            .map_err(map_db_err)?;

        Ok(ProwlarrConfig {
            url: row.try_get("url").map_err(|e| DbError::Io(Box::new(e)))?,
            api_key: row
                .try_get("api_key")
                .map_err(|e| DbError::Io(Box::new(e)))?,
            enabled: row
                .try_get::<bool, _>("enabled")
                .map_err(|e| DbError::Io(Box::new(e)))?,
        })
    }

    async fn update_prowlarr_config(
        &self,
        req: UpdateProwlarrConfigRequest,
    ) -> Result<ProwlarrConfig, DbError> {
        if let Some(url) = &req.url {
            sqlx::query("UPDATE prowlarr_config SET url = ? WHERE id = 1")
                .bind(url)
                .execute(self.pool())
                .await
                .map_err(map_db_err)?;
        }
        if let Some(api_key) = &req.api_key {
            sqlx::query("UPDATE prowlarr_config SET api_key = ? WHERE id = 1")
                .bind(api_key)
                .execute(self.pool())
                .await
                .map_err(map_db_err)?;
        }
        if let Some(enabled) = req.enabled {
            sqlx::query("UPDATE prowlarr_config SET enabled = ? WHERE id = 1")
                .bind(enabled)
                .execute(self.pool())
                .await
                .map_err(map_db_err)?;
        }

        self.get_prowlarr_config().await
    }

    async fn get_metadata_config(&self) -> Result<MetadataConfig, DbError> {
        let row = sqlx::query("SELECT * FROM metadata_config WHERE id = 1")
            .fetch_one(self.pool())
            .await
            .map_err(map_db_err)?;

        let llm_provider_str: Option<String> = row
            .try_get("llm_provider")
            .map_err(|e| DbError::Io(Box::new(e)))?;
        let languages_str: String = row
            .try_get("languages")
            .map_err(|e| DbError::Io(Box::new(e)))?;

        Ok(MetadataConfig {
            hardcover_enabled: row.try_get::<bool, _>("hardcover_enabled").unwrap_or(true),
            hardcover_api_token: row
                .try_get("hardcover_api_token")
                .map_err(|e| DbError::Io(Box::new(e)))?,
            llm_enabled: row.try_get::<bool, _>("llm_enabled").unwrap_or(true),
            llm_provider: llm_provider_str.and_then(|s| parse_llm_provider(&s)),
            llm_endpoint: row
                .try_get("llm_endpoint")
                .map_err(|e| DbError::Io(Box::new(e)))?,
            llm_api_key: row
                .try_get("llm_api_key")
                .map_err(|e| DbError::Io(Box::new(e)))?,
            llm_model: row
                .try_get("llm_model")
                .map_err(|e| DbError::Io(Box::new(e)))?,
            audnexus_url: row
                .try_get("audnexus_url")
                .map_err(|e| DbError::Io(Box::new(e)))?,
            languages: parse_languages(&languages_str),
        })
    }

    async fn update_metadata_config(
        &self,
        req: UpdateMetadataConfigRequest,
    ) -> Result<MetadataConfig, DbError> {
        if let Some(enabled) = req.hardcover_enabled {
            sqlx::query("UPDATE metadata_config SET hardcover_enabled = ? WHERE id = 1")
                .bind(enabled)
                .execute(self.pool())
                .await
                .map_err(map_db_err)?;
        }
        if let Some(token) = &req.hardcover_api_token {
            sqlx::query("UPDATE metadata_config SET hardcover_api_token = ? WHERE id = 1")
                .bind(token)
                .execute(self.pool())
                .await
                .map_err(map_db_err)?;
        }
        if let Some(enabled) = req.llm_enabled {
            sqlx::query("UPDATE metadata_config SET llm_enabled = ? WHERE id = 1")
                .bind(enabled)
                .execute(self.pool())
                .await
                .map_err(map_db_err)?;
        }
        if let Some(provider) = &req.llm_provider {
            sqlx::query("UPDATE metadata_config SET llm_provider = ? WHERE id = 1")
                .bind(llm_provider_str(provider))
                .execute(self.pool())
                .await
                .map_err(map_db_err)?;
        }
        if let Some(endpoint) = &req.llm_endpoint {
            sqlx::query("UPDATE metadata_config SET llm_endpoint = ? WHERE id = 1")
                .bind(endpoint)
                .execute(self.pool())
                .await
                .map_err(map_db_err)?;
        }
        if let Some(key) = &req.llm_api_key {
            sqlx::query("UPDATE metadata_config SET llm_api_key = ? WHERE id = 1")
                .bind(key)
                .execute(self.pool())
                .await
                .map_err(map_db_err)?;
        }
        if let Some(model) = &req.llm_model {
            sqlx::query("UPDATE metadata_config SET llm_model = ? WHERE id = 1")
                .bind(model)
                .execute(self.pool())
                .await
                .map_err(map_db_err)?;
        }
        if let Some(url) = &req.audnexus_url {
            sqlx::query("UPDATE metadata_config SET audnexus_url = ? WHERE id = 1")
                .bind(url)
                .execute(self.pool())
                .await
                .map_err(map_db_err)?;
        }
        if let Some(languages) = &req.languages {
            let json = serde_json::to_string(languages).map_err(|e| DbError::Io(Box::new(e)))?;
            sqlx::query("UPDATE metadata_config SET languages = ? WHERE id = 1")
                .bind(&json)
                .execute(self.pool())
                .await
                .map_err(map_db_err)?;
        }

        self.get_metadata_config().await
    }
}
