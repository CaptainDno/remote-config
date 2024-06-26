#![cfg_attr(docsrs, feature(doc_auto_cfg))]
#![doc = include_str!("../README.md")]
//! ## Feature flags
//! ### Main crate features
//! This features affect whole crate or `RemoteConfig` implementation directly
//! + `tracing` - enables tracing with tokio 
//! + `non_static` - enables implementation of `RemoteConfig` that uses `&Arc<RemoteConfig>` instead of `&'static RemoteConfig`. 
//!    As the intended use case for this crate is to store `RemoteConfig` in static tokio's `OnceCell`, this feature is not enabled by default.
//! 
//! ### Data providers
//! All built-in data providers and their features can be enabled or disabled using this feature flags.
//! + `http` - enables `HttpDataProvider` that uses reqwest client to load data from remote source (enabled by default)
//!     + `serde` - enables convenient data extractor for http data provider, that automatically parses necessary headers and deserializes data based on content-type (enabled by default)
//!         + `json` - json deserialization support (enabled by default). Deserializer: [serde_json](https://crates.io/crates/serde_json)
//!         + `yaml` - yaml deserialization support. Deserializer: [serde_yaml](https://crates.io/crates/serde_yaml)
//!         + `toml` - toml deserialization support. Deserializer: [toml](https://crates.io/crates/toml)
//!         + `xml` - xml deserialization support. Deserializer: [serde-xml-rs](https://crates.io/crates/serde-xml-rs)
//!
//! # Examples
//! ```
//! use std::collections::HashMap;
//! use std::time::{Duration, SystemTime};
//! use arc_swap::access::Access;
//! use reqwest::{Client, Url};
//! use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderValue};
//! use serde::de::Unexpected::Str;
//! use tokio::sync::OnceCell;
//! use std::string::String;
//! use remote_config::config::RemoteConfig;
//! use remote_config::data_providers::http::HttpDataProvider;
//! use remote_config::data_providers::http::serde_extractor::SerdeDataExtractor;
//!
//! type Data = HashMap<String, String>;
//! async fn init_config() -> RemoteConfig<Data, HttpDataProvider<Data, SerdeDataExtractor<Data>>> {
//!     // Configure http client
//!     let mut headers = HeaderMap::with_capacity(1);
//!     headers.insert(AUTHORIZATION, HeaderValue::from_str("Bearer: very_secret_token").unwrap());
//!     headers.insert(ACCEPT, HeaderValue::from_str("application/json").unwrap());
//!     let client = Client::builder().default_headers(headers).connect_timeout(Duration::from_secs(5)).build().unwrap();
//!
//!     let data_provider = HttpDataProvider::new(client, Url::parse("https://example.com").unwrap(), SerdeDataExtractor::new());
//!
//!     return RemoteConfig::new("Example named config".to_owned(), data_provider, Duration::from_secs(5)).await.unwrap();
//! }
//! // Note, that async OnceCell is used. You can use blocking OnceCell by changing init_config() to sync and using block_on() to wait for data load
//! static CONFIG: OnceCell<RemoteConfig<Data, HttpDataProvider<Data, SerdeDataExtractor<Data>>>> = OnceCell::const_new();
//! pub async fn do_some_things() {
//!     // You may want to handle errors properly instead of panicking
//!     let cfg = CONFIG.get_or_init(init_config).await.load().await.unwrap();
//!
//!     let val = cfg.get("key").unwrap();
//! }
//! ```

/// Remote Config instance and utility types
/// See [`config::RemoteConfig`] struct.
pub mod config;
/// Data providers for RemoteConfig instance.
/// Public traits are included to allow easy use of custom implementations.
pub mod data_providers;
