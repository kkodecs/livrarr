use sqlx::Row;

use crate::sqlite::SqliteDb;
use crate::sqlite_common::map_db_err;
use crate::{
    ConfigDb, DbError, LlmProvider, MediaManagementConfig, MetadataConfig, NamingConfig,
    ProwlarrConfig, UpdateMediaManagementConfigRequest, UpdateMetadataConfigRequest,
    UpdateProwlarrConfigRequest,
};

fn parse_llm_provider(s: &str) -> Result<LlmProvider, DbError> {
    match s {
        "groq" => Ok(LlmProvider::Groq),
        "gemini" => Ok(LlmProvider::Gemini),
        "openai" => Ok(LlmProvider::Openai),
        "custom" => Ok(LlmProvider::Custom),
        _ => Err(DbError::IncompatibleData {
            detail: format!("unknown LLM provider: {s}"),
        }),
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

fn parse_languages(s: &str) -> Result<Vec<String>, DbError> {
    serde_json::from_str(s).map_err(|e| DbError::IncompatibleData {
        detail: format!("invalid JSON in metadata_config.languages: {e}"),
    })
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
        let current = self.get_prowlarr_config().await?;

        let url = req.url.or(current.url);
        let api_key = req.api_key.or(current.api_key);
        let enabled = req.enabled.unwrap_or(current.enabled);

        sqlx::query("UPDATE prowlarr_config SET url = ?, api_key = ?, enabled = ? WHERE id = 1")
            .bind(&url)
            .bind(&api_key)
            .bind(enabled)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?;

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
            llm_provider: llm_provider_str
                .map(|s| parse_llm_provider(&s))
                .transpose()?,
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
            languages: parse_languages(&languages_str)?,
        })
    }

    async fn update_metadata_config(
        &self,
        req: UpdateMetadataConfigRequest,
    ) -> Result<MetadataConfig, DbError> {
        let current = self.get_metadata_config().await?;

        let hardcover_enabled = req.hardcover_enabled.unwrap_or(current.hardcover_enabled);
        let hardcover_api_token = req.hardcover_api_token.or(current.hardcover_api_token);
        let llm_enabled = req.llm_enabled.unwrap_or(current.llm_enabled);
        let llm_provider = req.llm_provider.or(current.llm_provider);
        let llm_provider_val = llm_provider.as_ref().map(llm_provider_str);
        let llm_endpoint = req.llm_endpoint.or(current.llm_endpoint);
        let llm_api_key = req.llm_api_key.or(current.llm_api_key);
        let llm_model = req.llm_model.or(current.llm_model);
        let audnexus_url = req.audnexus_url.unwrap_or(current.audnexus_url);
        let languages = req.languages.unwrap_or(current.languages);
        let languages_json =
            serde_json::to_string(&languages).map_err(|e| DbError::Io(Box::new(e)))?;

        sqlx::query(
            "UPDATE metadata_config SET \
             hardcover_enabled = ?, hardcover_api_token = ?, \
             llm_enabled = ?, llm_provider = ?, llm_endpoint = ?, \
             llm_api_key = ?, llm_model = ?, audnexus_url = ?, languages = ? \
             WHERE id = 1",
        )
        .bind(hardcover_enabled)
        .bind(&hardcover_api_token)
        .bind(llm_enabled)
        .bind(llm_provider_val)
        .bind(&llm_endpoint)
        .bind(&llm_api_key)
        .bind(&llm_model)
        .bind(&audnexus_url)
        .bind(&languages_json)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;

        self.get_metadata_config().await
    }
}
