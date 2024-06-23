use std::error::Error;
use std::fmt::{Debug, Display, Formatter};
use std::ops::Deref;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use arc_swap::{ArcSwap, Guard};
use tokio::sync::Mutex;
use crate::data_providers::data_provider::{DataLoadResult, DataProvider};

#[cfg(feature = "tracing")] use tracing::{warn, info};
use tracing::error;

struct Revalidator <Data, Provider: DataProvider<Data>> {
    data_provider: Provider,
    // Arc for easy thread safety
    revalidation_error: Option<Arc<DataProviderError>>
}

/// Remote configuration data.
/// Data is pulled from specified data provider automatically.
pub struct RemoteConfig<Data, Provider: DataProvider<Data>> {
    /// Config name to include in tracing messages
    #[cfg(feature = "tracing")] name: String,
    /// Minimal amount of time between data loading attempts in case of error
    retry_interval: Duration,
    /// Cached config, loaded from remote source
    cached_response: ArcSwap<DataLoadResult<Data>>,
    /// Used for revalidation
    revalidator: Mutex<Revalidator<Data, Provider>>
}

#[derive(Debug)]
struct DataProviderError {
    source: Option<Box<dyn Error>>,
    timestamp: SystemTime
}

impl Display for DataProviderError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "data provider error")
    }
}

impl Error for DataProviderError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source.as_ref()
    }
}

impl From<Box<dyn Error>> for DataProviderError{
    fn from(value: Box<(dyn Error + 'static)>) -> Self {
        DataProviderError{
            source: Some(value),
            timestamp: SystemTime::now()
        }
    }
}

#[derive(Clone)]
/// Convenient wrapper around pointer to load result that dereferences to data
pub struct CachedData<Data>(Guard<Arc<DataLoadResult<Data>>>);

impl <Data> Deref for CachedData<Data> {
    type Target = Data;

    fn deref(&self) -> &Self::Target {
        &self.0.data
    }
}
type LoadResult<Data> = Result<CachedData<Data>, Arc<DataProviderError>>;

impl <Data, Provider: DataProvider<Data>> RemoteConfig<Data, Provider> {
    /// Loads current config.
    /// If cached data is still valid, it is returned.
    /// If not, but `must_revalidate` is false, cached data is returned, but revalidation is started in background if necessary.
    /// If stale data must be revalidated, this method returns only after revalidation attempt is finished.
    /// # Errors
    /// Error is returned in one of the following cases if:
    /// Stale data must be revalidated and last revalidation attempt failed
    /// # Panics
    /// If underlying data provider panics.
    pub async fn load_with_time(&'static self, time: SystemTime) -> LoadResult<Data> {
        let curr = self.cached_response.load();

        if curr.valid_until < time {
            match self.revalidator.try_lock() {
                // Revalidation is in progress
                Err(_) => {
                    if curr.must_revalidate {
                        // Wait for revalidation to finish
                        let guard = self.revalidator.lock().await;

                        if let Some(ref error) = guard.revalidation_error {
                            // Revalidation failed
                            // Error is wrapped in arc for thread safety
                            return Err(error.clone());
                        } else {
                            // Revalidation was successful, so we can use data without additional checks
                            return Ok(CachedData(self.cached_response.load()));
                        }
                    }
                    else {
                        #[cfg(feature = "tracing")] {
                            warn!("Stale configuration data is being used for config '{cfg_name}'", cfg_name = self.name)
                        }
                        return Ok(CachedData(curr));
                    }
                },
                // Revalidation should be started
                Ok(mut guard) => {

                    // Quick return if it is too early to retry after error
                    if let Some(ref err) = guard.revalidation_error {
                        if time < err.timestamp + self.retry_interval {
                            if curr.must_revalidate {
                                return Err(err.clone());
                            } else {
                                return Ok(CachedData(curr))
                            }
                        }
                    }

                    let handle = tokio::spawn(async move {
                        match guard.load_data() {
                            Ok(load_result) => {
                                self.cached_response.store(Arc::new(load_result));
                                guard.revalidation_error = None;
                                return Ok(CachedData(self.cached_response.load()));
                            },
                            Err(err) => {
                                #[cfg(feature = "tracing")] error!("Failed to load data for config {cfg_name}. Error: {error}", cfg_name = self.name, error = err.source());
                                let dp_err = Arc::new(DataProviderError::from(err));
                                guard.revalidation_error = Some(dp_err.clone());
                                return Err(dp_err);
                            }
                        }
                    });

                    if curr.must_revalidate {
                        // Wait for validation attempt to finish
                        return handle.await.unwrap();
                    } else {
                        // Return immediately
                        return Ok(CachedData(curr));
                    }
                }
            }
        }

        // Return valid data
        return Ok(CachedData(curr));
    }

    /// See [`RemoteConfig::load_with_time`] docs
    pub async fn load(&'static self) -> LoadResult<Data> {
        self.load_with_time(SystemTime::now())
    }
}

#[cfg(feature = "non_static")]
pub trait NonStaticRemoteConfig<S: Send + Sync + Clone, Data> {
    async fn load_with_time(&self: &S, time: SystemTime) -> LoadResult<Data>;
    async fn load(&self: S) -> LoadResult<Data>;
}

#[cfg(feature = "non_static")]
impl <Data, Provider: DataProvider<Data>> NonStaticRemoteConfig<Arc<RemoteConfig<Data, Provider>>, Data>
for Arc<RemoteConfig<Data, Provider>>{
    /// See [`RemoteConfig::load_with_time`] docs
    async fn load_with_time(&self: &Arc<RemoteConfig<Data, Provider>>, time: SystemTime) -> LoadResult<Data> {
        let curr = self.cached_response.load();

        if curr.valid_until < time {
            match self.revalidator.try_lock() {
                // Revalidation is in progress
                Err(_) => {
                    if curr.must_revalidate {
                        // Wait for revalidation to finish
                        let guard = self.revalidator.lock().await;

                        if let Some(ref error) = guard.revalidation_error {
                            // Revalidation failed
                            // Error is wrapped in arc for thread safety
                            return Err(error.clone());
                        } else {
                            // Revalidation was successful, so we can use data without additional checks
                            return Ok(CachedData(self.cached_response.load()));
                        }
                    }
                    else {
                        #[cfg(feature = "tracing")] {
                            warn!("Stale configuration data is being used for config '{cfg_name}'", cfg_name = self.name)
                        }
                        return Ok(CachedData(curr));
                    }
                },
                // Revalidation should be started
                Ok(mut guard) => {

                    // Quick return if it is too early to retry after error
                    if let Some(ref err) = guard.revalidation_error {
                        if time < err.timestamp + self.retry_interval {
                            if curr.must_revalidate {
                                return Err(err.clone());
                            } else {
                                return Ok(CachedData(curr))
                            }
                        }
                    }
                    let cloned = self.clone();
                    let handle = tokio::spawn(async move {
                        match guard.load_data() {
                            Ok(load_result) => {
                                cloned.cached_response.store(Arc::new(load_result));
                                guard.revalidation_error = None;
                                return Ok(CachedData(cloned.cached_response.load()));
                            },
                            Err(err) => {
                                #[cfg(feature = "tracing")] error!("Failed to load data for config {cfg_name}. Error: {error}", cfg_name = self.name, error = err.source());
                                let dp_err = Arc::new(DataProviderError::from(err));
                                guard.revalidation_error = Some(dp_err.clone());
                                return Err(dp_err);
                            }
                        }
                    });

                    if curr.must_revalidate {
                        // Wait for validation attempt to finish
                        return handle.await.unwrap();
                    } else {
                        // Return immediately
                        return Ok(CachedData(curr));
                    }
                }
            }
        }

        // Return valid data
        return Ok(CachedData(curr));
    }

    /// See [`RemoteConfig::load_with_time`] docs
    async fn load(&self: &Arc<RemoteConfig<Data, Provider>>) -> LoadResult<Data> {
        self.load_with_time(SystemTime::now())
    }
}