use std::ops::Deref;
use std::sync::Arc;
use std::time::Duration;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use tokio::sync::OnceCell;
use tokio::time::sleep;
use remote_config::config::RemoteConfig;
use remote_config::data_providers::http::HttpDataProvider;
use remote_config::data_providers::http::serde_extractor::SerdeDataExtractor;

#[derive(Debug, Serialize, Deserialize, Eq, PartialEq)]
struct MockData {
    test_number: u32
}

impl Default for MockData {
    fn default() -> Self {
        MockData {
            test_number: 49842
        }
    }
}

async fn init_config(url : &str) -> RemoteConfig<MockData, HttpDataProvider<MockData, SerdeDataExtractor<MockData>>> {
    let client = reqwest::Client::default();
    let data_provider = HttpDataProvider::new(client, Url::parse(url).unwrap(), SerdeDataExtractor::default());
    RemoteConfig::new("Test config".to_string(), data_provider, Duration::from_secs(1)).await.unwrap()
}

type RConfTest = RemoteConfig<MockData, HttpDataProvider<MockData, SerdeDataExtractor<MockData>>>;

async fn test_static_with_cache_control(conf: &'static OnceCell<RConfTest>, must_revalidate: bool, ttl: Duration) {
    static MOCK_DATA: MockData = MockData{test_number: 999};

    let mut server = mockito::Server::new_async().await;

    let mut cache_control_val = format!("private, max-age={secs}", secs = ttl.as_secs());
    if must_revalidate {
        cache_control_val += ", must-revalidate"
    }
    
    let mock = server
        .mock("GET", "/mock")
        .with_header("Content-Type", "application/json")
        .with_header("Cache-Control", &cache_control_val)
        .with_body(serde_json::to_string(&MOCK_DATA).unwrap())
        .expect(2)
        .create_async()
        .await;
    
    let url = Arc::new(server.url() + "/mock");

    // Test initial load
    let mut handles = Vec::with_capacity(10);
    
    for _ in 0..10{
        let uc = url.clone();
        handles.push(tokio::spawn(async move {
            // Expect successful load
            assert_eq!(conf.get_or_init(|| init_config(uc.as_str())).await.load().await.unwrap().deref(), &MOCK_DATA);
        }));
    }
    for handle in handles {
        handle.await.unwrap();
    }
    
    // Wait for data to expire
    sleep(ttl).await;

    // Revalidation should succeed
    let mut handles = Vec::with_capacity(10);
    for _ in 0..10{
        let uc = url.clone();
        handles.push(tokio::spawn(async move {
            // Expect successful load
            assert_eq!(conf.get_or_init(|| init_config(uc.as_str())).await.load().await.unwrap().deref(), &MOCK_DATA);
        }));
    }
    for handle in handles {
        handle.await.unwrap();
    }
    
    // In case background request is in progress
    sleep(Duration::from_millis(100)).await;
    
    // Only two requests expected
    mock.assert_async().await;
}

#[tokio::test]
async fn test_with_must_revalidate() {
    static CONF: OnceCell<RConfTest> = OnceCell::const_new();
    
    test_static_with_cache_control(&CONF, true, Duration::from_secs(1)).await;
}

#[tokio::test]
async fn test_without_must_revalidate() {
    static CONF: OnceCell<RConfTest> = OnceCell::const_new();
    test_static_with_cache_control(&CONF, false, Duration::from_secs(1)).await;
}