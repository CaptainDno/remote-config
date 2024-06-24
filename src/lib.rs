#![cfg_attr(docsrs, feature(doc_auto_cfg))]
//! Test crate documentation

/// Remote Config instance and utility types
pub mod config;
/// Data providers for RemoteConfig instance.
/// Public traits are included to allow easy use of custom implementations.
pub mod data_providers;
