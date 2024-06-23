/// Common data provider code
pub mod data_provider;

/// Data providers and extractors that use reqwest HTTP client to load data from remote source
#[cfg(feature = "http")]
pub mod http;
