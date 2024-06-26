# Remote config

[![crates.io](https://img.shields.io/crates/v/remote_config)](https://crates.io/crates/remote_config)

This crate provides easy way to asynchronously pull configuration files from external source
(e.g. centralized HTTP service).

It was originally developed to load public keys from `https://www.googleapis.com/robot/v1/metadata/x509/securetoken@system.gserviceaccount.com`
and revalidate them periodically.

## Features
+ Supports both `static` and wrapped in `Arc` configs.
+ Flexible `RemoteConfig` struct that uses any custom data provider and automatically revalidates data when it becomes stale.
+ Supports loading configuration in JSON, YAML, XML and TOML via HTTP out of the box (Uses `Cache-Control` and `Content-Type` headers).

## Documentation and examples

Refer to documentation and examples on [docs rs](https://docs.rs).

## State of the project
Project is not guaranteed to actively receive any new features without requests, but is maintained.
Feature requests, bug reports, pull requests, corrections to docs or examples are gladly accepted. 

Code is tested, with unit and integration tests. 

Currently, this project is not used in production environment.

## Contributing
If you found a bug, submit an issue or pull requests.

Any contribution intentionally submitted for inclusion in Tokio by you, **shall be licensed as MIT**, without any additional terms or conditions.

## Licence

This project is licenced under the MIT licence.
