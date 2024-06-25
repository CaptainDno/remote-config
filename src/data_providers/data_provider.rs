use std::error::Error;
use std::time::SystemTime;
/// Result of successful data load
/// # What if I don't need caching?
/// Just set `valid_until` to some time in the past or current time.
#[derive(Debug)]
pub struct DataLoadResult<T> {
    /// Data in desired format
    pub data: T,
    /// If true, once the data becomes stale, it can't be used until revalidated successfully.
    pub must_revalidate: bool,
    /// Time in the future when `data` becomes stale
    pub valid_until: SystemTime
}
/// Remote data provider trait.
/// Data provider loads data from external sources and returns [`DataLoadResult`]
/// # Errors
/// Any error can be returned by custom implementation.
pub trait DataProvider<Data: Send + Sync> {
    /// Try to load data
    fn load_data(&self) -> impl std::future::Future<Output = Result<DataLoadResult<Data>, Box<dyn Error>>> + Send;
}