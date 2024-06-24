use std::error::Error;
use std::fmt::{Debug, Display, Formatter};
use std::future::Future;
use std::marker::PhantomData;
use std::ops::Deref;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use arc_swap::{ArcSwap, AsRaw, Guard};
use tokio::spawn;
use tokio::sync::Mutex;
use crate::data_providers::data_provider::{DataLoadResult, DataProvider};

#[cfg(feature = "tracing")] use tracing::{warn, error};

#[derive(Debug)]
struct Revalidator <Data: Send + Sync, Provider: DataProvider<Data> + Send> {
    data_provider: Provider,
    // Arc for easy thread safety
    revalidation_error: Option<Arc<DataProviderError>>,
    data_type: PhantomData<Data>
}

/// Remote configuration data.
/// Data is pulled from specified data provider automatically.
/// # Examples
/// ```
/// use std::collections::HashMap;
/// use std::time::{Duration, SystemTime};
/// use arc_swap::access::Access;
/// use reqwest::{Client, Url};
/// use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderValue};
/// use serde::de::Unexpected::Str;
/// use tokio::sync::OnceCell;
/// use std::string::String;
/// use remote_config::config::RemoteConfig;
/// use remote_config::data_providers::http::HttpDataProvider;
/// use remote_config::data_providers::http::serde_extractor::SerdeDataExtractor;
///
/// type Data = HashMap<String, String>;
/// async fn init_config() -> RemoteConfig<Data, HttpDataProvider<Data, SerdeDataExtractor<Data>>> {
///     // Configure http client
///     let mut headers = HeaderMap::with_capacity(1);
///     headers.insert(AUTHORIZATION, HeaderValue::from_str("Bearer: very_secret_token").unwrap());
///     headers.insert(ACCEPT, HeaderValue::from_str("application/json").unwrap());
///     let client = Client::builder().default_headers(headers).connect_timeout(Duration::from_secs(5)).build().unwrap();
///
///     let data_provider = HttpDataProvider::new(client, Url::parse("https://example.com").unwrap(), SerdeDataExtractor::new());
///
///     return RemoteConfig::new("Example named config".to_owned(), data_provider, Duration::from_secs(5)).await.unwrap();
/// }
/// // Note, that async OnceCell is used. You can use blocking OnceCell by changing init_config() to sync and using block_on() to wait for data load
/// static CONFIG: OnceCell<RemoteConfig<Data, HttpDataProvider<Data, SerdeDataExtractor<Data>>>> = OnceCell::const_new();
/// pub async fn do_some_things() {
///     // You may want to handle errors properly instead of panicking
///     let cfg = CONFIG.get_or_init(init_config).await.load().await.unwrap();
///
///     let val = cfg.get("key").unwrap();
/// }
/// ```
/// # Thread safety
/// [`DataProvider`] must be [`Send`] (to safely perform revalidation in background task),
/// but may not be [`Sync`] (only one thread can perform revalidation to avoid spamming unnecessary request).
///
/// `Data` must be both [`Send`] and [`Sync`]
#[derive(Debug)]
pub struct RemoteConfig<Data: Send + Sync, Provider: DataProvider<Data> + Send> {
    /// Config name to include in tracing messages
    #[cfg(feature = "tracing")] name: String,
    /// Minimal amount of time between data loading attempts in case of error
    retry_interval: Duration,
    /// Cached config, loaded from remote source
    cached_response: ArcSwap<DataLoadResult<Data>>,
    /// Used for revalidation
    revalidator: Mutex<Revalidator<Data, Provider>>
}

/// Wrapper around error that is returned by data provider
#[derive(Debug)]
pub struct DataProviderError {
    source: Option<Box<dyn Error>>,
    timestamp: SystemTime
}
// Data provider errors are always wrapped in Arc and additionally guarded by mutex.
// And when they are returned in results, they are immutable.

unsafe impl Send for DataProviderError {}
unsafe impl Sync for DataProviderError {}

impl Display for DataProviderError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "data provider error")
    }
}

impl Error for DataProviderError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source.as_deref()
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

/// Convenient wrapper around pointer to load result that dereferences to data
#[derive(Debug)]
pub struct CachedData<Data>(Guard<Arc<DataLoadResult<Data>>>);

impl <Data> Deref for CachedData<Data> {
    type Target = Data;

    fn deref(&self) -> &Self::Target {
        &self.0.data
    }
}
type LoadResult<Data> = Result<CachedData<Data>, Arc<DataProviderError>>;

impl <Data: Send + Sync, Provider: DataProvider<Data> + Send> RemoteConfig<Data, Provider> {
    /// Constructs new remote config instance.
    /// If `tracing` feature is activated, name should be assigned to config instance.
    /// # Errors
    /// Returns error if initial data load failed.
    pub async fn new(
        #[cfg(feature = "tracing")] name: String,
        data_provider: Provider,
        retry_interval: Duration
    ) -> Result<Self, DataProviderError> {
        let data = data_provider.load_data().await.map_err(DataProviderError::from)?;
        let revalidator = Revalidator{
            data_provider,
            revalidation_error: None,
            data_type: PhantomData
        };
        Ok(Self {
            #[cfg(feature = "tracing")] name,
            retry_interval,
            cached_response: ArcSwap::new(Arc::new(data)),
            revalidator: Mutex::new(revalidator)
        })
    }

    /// Loads current config.
    /// If cached data is still valid, it is returned.
    /// If not, but `must_revalidate` is false, cached data is returned, and revalidation is started in background if necessary.
    /// If stale data must be revalidated, this method returns only after revalidation attempt is finished.
    /// ## Why static?
    /// When data is stale, but `!must_revalidate`, revalidation task is spawned to perform revalidation in background. Lifetime must be `'static` to spawn this task.
    /// Enable feature `non_static` to support usage of [`Arc`] instead of static reference.
    /// # Errors
    /// If stale data must be revalidated and last revalidation attempt failed
    /// # Panics
    /// If underlying data provider panics.
    pub async fn load_with_time(&'static self, time: SystemTime) -> LoadResult<Data> {
        let curr = self.cached_response.load();

        if curr.valid_until < time {
            return match self.revalidator.try_lock() {
                // Revalidation is in progress
                Err(_) => {
                    if curr.must_revalidate {
                        // Wait for revalidation to finish
                        let guard = self.revalidator.lock().await;

                        if let Some(ref error) = guard.revalidation_error {
                            // Revalidation failed
                            // Error is wrapped in arc for thread safety
                            Err(error.clone())
                        } else {
                            // Revalidation was successful, so we can use data without additional checks
                            Ok(CachedData(self.cached_response.load()))
                        }
                    } else {
                        #[cfg(feature = "tracing")] {
                            warn!("Stale configuration data is being used for config '{cfg_name}'", cfg_name = self.name)
                        }
                        Ok(CachedData(curr))
                    }
                },
                // Revalidation should be started
                Ok(mut guard) => {

                    // Quick return if it is too early to retry after error
                    if let Some(ref err) = guard.revalidation_error {
                        if time < err.timestamp + self.retry_interval {
                            return if curr.must_revalidate {
                                Err(err.clone())
                            } else {
                                Ok(CachedData(curr))
                            }
                        }
                    }

                    let handle = spawn(async move {
                        return match guard.data_provider.load_data().await {
                            Ok(load_result) => {
                                self.cached_response.store(Arc::new(load_result));
                                guard.revalidation_error = None;
                                Ok(CachedData(self.cached_response.load()))
                            },
                            Err(err) => {
                                #[cfg(feature = "tracing")] {
                                    if let Some(source) = err.source() {
                                        error!("Failed to load data for config {cfg_name}. Error: {error}", cfg_name = self.name, error = source);
                                    } else {
                                        error!("Failed to load data for config {cfg_name}. No source error provided", cfg_name = self.name)
                                    }
                                }
                                let dp_err = Arc::new(DataProviderError::from(err));
                                guard.revalidation_error = Some(dp_err.clone());
                                Err(dp_err)
                            }
                        }
                    });

                    if curr.must_revalidate {
                        // Wait for validation attempt to finish
                        handle.await.unwrap()
                    } else {
                        // Return immediately
                        Ok(CachedData(curr))
                    }
                }
            }
        }

        // Return valid data
        Ok(CachedData(curr))
    }

    /// See [`RemoteConfig::load_with_time`] docs
    pub async fn load(&'static self) -> LoadResult<Data> {
        self.load_with_time(SystemTime::now()).await
    }
}

#[cfg(feature = "non_static")]
pub trait NonStaticRemoteConfig <Data: Send + Sync>
where Self: Send + Sync + Clone
{
    fn load_with_time(&self, time: SystemTime) -> impl Future<Output = LoadResult<Data>> + Send;
    fn load(&self) -> impl Future<Output = LoadResult<Data>> + Send;
    
}

#[cfg(feature = "non_static")]
impl <Data: Send + Sync + 'static, Provider: DataProvider<Data> + Send + 'static> NonStaticRemoteConfig<Data> for Arc<RemoteConfig<Data, Provider>> {
    async fn load_with_time(&self, time: SystemTime) -> LoadResult<Data> {
        let curr = self.cached_response.load();

        // Self is cloned and moved into spawned task, so reference validity is guaranteed
        let self_static: &'static RemoteConfig<Data, Provider> = unsafe{&*self.as_raw()};
        
        if curr.valid_until < time {
            return match self_static.revalidator.try_lock() {
                // Revalidation is in progress
                Err(_) => {
                    if curr.must_revalidate {
                        // Wait for revalidation to finish
                        let guard = self_static.revalidator.lock().await;

                        if let Some(ref error) = guard.revalidation_error {
                            // Revalidation failed
                            // Error is wrapped in arc for thread safety
                            Err(error.clone())
                        } else {
                            // Revalidation was successful, so we can use data without additional checks
                            Ok(CachedData(self_static.cached_response.load()))
                        }
                    } else {
                        #[cfg(feature = "tracing")] {
                            warn!("Stale configuration data is being used for config '{cfg_name}'", cfg_name = self_static.name)
                        }
                        Ok(CachedData(curr))
                    }
                },
                // Revalidation should be started
                Ok(mut guard) => {

                    // Quick return if it is too early to retry after error
                    if let Some(ref err) = guard.revalidation_error {
                        if time < err.timestamp + self_static.retry_interval {
                            return if curr.must_revalidate {
                                Err(err.clone())
                            } else {
                                Ok(CachedData(curr))
                            }
                        }
                    }

                    // We clone and move self to uphold 'static lifetime guarantee
                    let cloned = self.clone();
                    
                    let handle = spawn(async move {
                        return match guard.data_provider.load_data().await {
                            Ok(load_result) => {
                                cloned.cached_response.store(Arc::new(load_result));
                                guard.revalidation_error = None;
                                Ok(CachedData(cloned.cached_response.load()))
                            },
                            Err(err) => {
                                #[cfg(feature = "tracing")] {
                                    if let Some(source) = err.source() {
                                        error!("Failed to load data for config {cfg_name}. Error: {error}", cfg_name = cloned.name, error = source);
                                    } else {
                                        error!("Failed to load data for config {cfg_name}. No source error provided", cfg_name = cloned.name)
                                    }
                                }
                                let dp_err = Arc::new(DataProviderError::from(err));
                                guard.revalidation_error = Some(dp_err.clone());
                                Err(dp_err)
                            }
                        }
                    });

                    if curr.must_revalidate {
                        // Wait for validation attempt to finish
                        handle.await.unwrap()
                    } else {
                        // Return immediately
                        Ok(CachedData(curr))
                    }
                }
            }
        }

        // Return valid data
        Ok(CachedData(curr))
    }

    async fn load(&self) -> LoadResult<Data> {
        self.load_with_time(SystemTime::now()).await
    }
}
