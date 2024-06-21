use std::error::Error;
use std::fmt::{Debug, Display, Formatter};
use std::marker::PhantomPinned;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::sync::atomic::Ordering::Relaxed;
use std::time::SystemTime;
use tokio::sync::Mutex;
use crate::data_providers::data_provider::{DataLoadResult, DataProvider};

struct CachedResponse <T> {
    must_revalidate: bool,
    valid_until: SystemTime,
    // Arc ensures that when old cached response is dropped, data is not dropped if it is still in use somewhere
    data: T,
}

impl <Data> From<DataLoadResult<Data>> for CachedResponse<Data> {
    fn from(value: DataLoadResult<Data>) -> Self {
        CachedResponse {
            must_revalidate: value.must_revalidate,
            valid_until: SystemTime::now() + value.max_age,
            data: Arc::new(value.data),
            _phantom_pinned: PhantomPinned,
        }
    }
}

struct RemoteConfig<Data, Provider: DataProvider<Data>> {
    data_provider: Provider,
    cached_response: AtomicPtr<Arc<CachedResponse<Data>>>,
    revalidation_lock: Mutex<()>,
    is_revalidating: AtomicBool,
    ordering: Ordering
}
#[derive(Debug)]
struct DataProviderError {
    source: Option<Box<dyn Error>>
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
    fn from(value: Option<Box<dyn Error>>) -> Self {
        DataProviderError{
            source: value
        }
    }
}

impl <Data, Provider: DataProvider<Data>> RemoteConfig<Data, Provider> {

    pub async fn pull(&self) -> Result<CachedResponse<Data>, DataProviderError>{
        return match self.data_provider.load_data() {
            Ok(res) => Ok(CachedResponse::from(res)),
            Err(err) => Err(DataProviderError::from(err))
        }
    }

    pub async fn load_with_time(&self, time: SystemTime) -> Result<Arc<Data>, DataProviderError> {
        // Just use arc swap. I'm really tired of synchronizing this shit

    }

    pub fn load (&self) -> Arc<Data> {
        let ptr = self.cached_response.load(Relaxed);

    }
}
