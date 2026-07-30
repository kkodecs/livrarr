use livrarr_domain::services::HttpFetcher;
use livrarr_domain::{ProviderAuthorRef, RequestPriority};

use crate::provider_client::{GoodreadsClient, HardcoverClient, OpenLibraryClient};
use crate::types::ProviderFetchError;

impl<F: HttpFetcher> OpenLibraryClient<F> {
    pub async fn fetch_work_authors(
        &self,
        work_key: String,
        priority: RequestPriority,
    ) -> Result<Vec<ProviderAuthorRef>, ProviderFetchError> {
        todo!()
    }
}

impl<F: HttpFetcher> GoodreadsClient<F> {
    pub async fn fetch_work_authors(
        &self,
        book_key: String,
        priority: RequestPriority,
    ) -> Result<Vec<ProviderAuthorRef>, ProviderFetchError> {
        todo!()
    }
}

impl<F: HttpFetcher> HardcoverClient<F> {
    pub async fn fetch_work_authors(
        &self,
        book_key: String,
        priority: RequestPriority,
    ) -> Result<Vec<ProviderAuthorRef>, ProviderFetchError> {
        todo!()
    }
}
