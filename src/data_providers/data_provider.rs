use std::error::Error;
use std::time::{Duration, SystemTime};
/// Result of successful data load
pub struct DataLoadResult<T> {
    /// Data in desired format
    pub data: T,
    /// If true, once the data becomes stale, it can't be used until revalidated successfully.
    pub must_revalidate: bool,
    /// Max data age until it should be revalidated
    pub max_age: Duration
}
/// Remote data provider trait.
/// Data provider is responsible for loading data rom external source and converting it to desired type.
pub trait DataProvider<Data> {
    async fn load_data(&self) -> Result<DataLoadResult<Data>, Box<dyn Error>>;
}