use crate::{DbError, RemotePathMapping, RemotePathMappingId};

#[trait_variant::make(Send)]
pub trait RemotePathMappingService: Send + Sync {
    async fn get_remote_path_mapping(
        &self,
        id: RemotePathMappingId,
    ) -> Result<RemotePathMapping, DbError>;
    async fn list_remote_path_mappings(&self) -> Result<Vec<RemotePathMapping>, DbError>;
    async fn create_remote_path_mapping(
        &self,
        host: &str,
        remote_path: &str,
        local_path: &str,
    ) -> Result<RemotePathMapping, DbError>;
    async fn update_remote_path_mapping(
        &self,
        id: RemotePathMappingId,
        host: &str,
        remote_path: &str,
        local_path: &str,
    ) -> Result<RemotePathMapping, DbError>;
    async fn delete_remote_path_mapping(&self, id: RemotePathMappingId) -> Result<(), DbError>;
}
