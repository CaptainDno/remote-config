use std::error::Error;
use std::fmt::{Display, Formatter};
use std::marker::PhantomData;
use std::ops::Deref;
use cache_control::CacheControl;
use reqwest::header::{CACHE_CONTROL, HeaderName, HeaderValue};
use reqwest::{StatusCode, Url};
use crate::data_providers::data_provider::{DataLoadResult, DataProvider};
use crate::data_providers::http::DataExtractionError::HeaderParseError;

/// Generic data extractor, that consumes [`reqwest::Response`]
/// Use this trait to create custom data extractors.
pub trait HttpDataExtractor<Data: Send + Sync> {
    /// Extract data from HTTP response
    /// # Errors
    /// Any error can be returned by custom implementation.
    fn extract(&self, response: reqwest::Response) -> impl std::future::Future<Output = Result<DataLoadResult<Data>, Box<dyn Error>>> + Send;
}

/// This data provider uses http client to send GET request to specified URL, then feeds response into specified data extractor
/// # Examples
/// ```
/// use std::collections::HashMap;
/// use reqwest::Url;
/// use remote_config::data_providers::http::HttpDataProvider;
/// use remote_config::data_providers::http::serde_extractor::SerdeDataExtractor;
///
/// // You can configure http client to pass additional header, for example
/// let client = reqwest::Client::default();
/// // Use default extractor from serde_extractor module
/// let extractor = SerdeDataExtractor::<HashMap<String, String>>::new();
/// let data_provider = HttpDataProvider::new(client, Url::parse("https://www.example.com/cfg").unwrap(), extractor);
/// ```
pub struct HttpDataProvider<Data: Send + Sync, Extractor: HttpDataExtractor<Data>> {
    extractor: Extractor,
    client: reqwest::Client,
    url: Url,
    phantom_data: PhantomData<Data>
}

impl <Data: Send + Sync, Extractor: HttpDataExtractor<Data> + Sync> DataProvider<Data> for HttpDataProvider<Data, Extractor> {
    /// Loads data by making GET request to specified URL
    /// # Errors
    /// If either reqwest client or data extractor returns an error.
    async fn load_data(&self) -> Result<DataLoadResult<Data>, Box<dyn Error>> {
        // Clone because trait is not implemented for reference
        self.extractor.extract(self.client.get(self.url.clone()).send().await?).await
    }
}

impl <Data: Send + Sync, Extractor: HttpDataExtractor<Data>> HttpDataProvider<Data, Extractor> {
    /// Construct new [`HttpDataExtractor`] from reqwest client, url and data extractor
    pub fn new(client: reqwest::Client, url: Url, extractor: Extractor) -> Self {
        Self {
            client,
            url,
            extractor,
            phantom_data: PhantomData
        }
    }
}

// Test both serde extractor and http data provider
#[cfg(all(test, feature = "serde"))]
mod tests {
    use std::time::SystemTime;
    use mockito::ServerGuard;
    use reqwest::{Url};
    use serde::{Deserialize, Serialize};
    use serde_json::json;
    use crate::data_providers::data_provider::DataProvider;
    use crate::data_providers::http::{DataExtractionError, HttpDataProvider};
    use crate::data_providers::http::serde_extractor::SerdeDataExtractor;

    #[derive(Deserialize, Serialize, Debug, Eq, PartialEq)]
    struct TestData {
        test_number: i64
    }

    const TEST_DATA: TestData = TestData{
        test_number: 42
    };

    async fn get_server(valid: String, invalid: String, content_type: &str) -> ServerGuard {
        let mut server = mockito::Server::new_async().await;

        server
            .mock("GET", "/valid-allow-stale")
            .with_header("Content-Type", content_type)
            .with_header("Cache-Control", "public, max-age=10")
            .with_body(valid.clone())
            .create_async()
            .await;

        server
            .mock("GET", "/valid-must-revalidate")
            .with_header("Content-Type", content_type)
            .with_header("Cache-Control", "public, max-age=10, must-revalidate")
            .with_body(valid.clone())
            .create_async()
            .await;

        server
            .mock("GET", "/invalid")
            .with_header("Content-Type", content_type)
            .with_header("Cache-Control", "public, max-age=10")
            .with_body(invalid)
            .create_async()
            .await;

        server
            .mock("GET", "/valid-no-cache-control")
            .with_header("Content-Type", content_type)
            .with_body(valid.clone())
            .create_async()
            .await;

        server
            .mock("GET", "/unknown-content-type")
            .with_header("Content-Type", "unknown")
            .with_header("Cache-Control", "public, max-age=10")
            .with_body("unknown content type body")
            .create_async()
            .await;

        server
            .mock("GET", "/404")
            .with_status(404)
            .with_header("Content-Type", "application/json")
            .with_header("Cache-Control", "public, max-age=10, must-revalidate")
            .with_body(json!({"err": "Not found"}).to_string())
            .create_async()
            .await;
        
        server
    }

    fn get_data_provider(url: String) -> HttpDataProvider<TestData, SerdeDataExtractor<TestData>>{
        HttpDataProvider::new(
            reqwest::Client::default(),
            Url::parse(&url).unwrap(),
            SerdeDataExtractor::new()
        )
    }
    
    macro_rules! test_content_type {
        ($serialization_expr:expr, $content_type:literal) => {
            let valid = $serialization_expr;
            let invalid = "invalid string".to_string();

            let server = get_server(valid, invalid, $content_type).await;

            {
                let data_provider = get_data_provider(server.url() + "/valid-allow-stale");
                let data = data_provider.load_data().await.unwrap();
                assert_eq!(data.must_revalidate, false);
                assert_eq!(data.data, TEST_DATA);
                assert!(data.valid_until > SystemTime::now());
            }

            {
                let data_provider = get_data_provider(server.url() + "/valid-must-revalidate");
                let data = data_provider.load_data().await.unwrap();
                assert_eq!(data.must_revalidate, true);
                assert_eq!(data.data, TEST_DATA);
                assert!(data.valid_until > SystemTime::now());
            }

            {
                let data_provider = get_data_provider(server.url() + "/invalid");
                let e = data_provider.load_data().await.expect_err("Expected error on invalid content deserialization attempt").downcast::<DataExtractionError>().unwrap();
                assert!(matches!(*e, DataExtractionError::ContentParseError(_, _)));

            }

            {
                let data_provider = get_data_provider(server.url() + "/valid-no-cache-control");
                let e =  data_provider.load_data().await.expect_err("Expected error: Cache-Control header not present").downcast::<DataExtractionError>().unwrap();
                assert!(matches!(*e, DataExtractionError::HeaderNotFound(reqwest::header::CACHE_CONTROL)));
            }

            {
                let data_provider = get_data_provider(server.url() + "/unknown-content-type");
                let e = data_provider.load_data().await.expect_err("Expected error: content-type is unsupported").downcast::<DataExtractionError>().unwrap();
                assert!(matches!(*e, DataExtractionError::UnsupportedContentType(_, _)));
            }

            {
                let data_provider = get_data_provider(server.url() + "/404");
                let e = data_provider.load_data().await.expect_err("Expected error: content-type is unsupported").downcast::<DataExtractionError>().unwrap();
                assert!(matches!(*e, DataExtractionError::StatusError(_)));
            }
        };
    }

    #[tokio::test]
    #[cfg(feature = "json")]
    async fn deserialize_json() {
        test_content_type!(serde_json::to_string(&TEST_DATA).unwrap(), "application/json");
    }

    #[tokio::test]
    #[cfg(feature = "yaml")]
    async fn deserialize_yaml() {
        test_content_type!(serde_yaml::to_string(&TEST_DATA).unwrap(), "application/yaml");
    }

    #[tokio::test]
    #[cfg(feature = "toml")]
    async fn deserialize_toml() {
        test_content_type!(toml::to_string(&TEST_DATA).unwrap(), "application/toml");
    }

    #[tokio::test]
    #[cfg(feature = "xml")]
    async fn deserialize_xml() {
        test_content_type!(serde_xml_rs::to_string(&TEST_DATA).unwrap(), "application/xml");
    }

    #[tokio::test]
    async fn http_error() {
        {
            let data_provider = get_data_provider("https://localhost".to_string());
            data_provider.load_data().await.expect_err("Expected error when sending reqwest to non existent resource");
        }
    }
}

/// Data extraction errors
#[derive(Debug)]
pub enum DataExtractionError {
    /// Header is required to correctly extract data from response, but is not present in it
    HeaderNotFound(HeaderName),
    /// Header could not be parsed
    HeaderParseError(HeaderName, String),
    /// Content type of response is not supported by extractor.
    /// If there is feature that enables support for this content type, feature name is included
    UnsupportedContentType(String, Option<&'static str>), // Optional feature name can be provided
    /// Response body could not be parsed
    ContentParseError(String, Box<dyn Error>),
    /// Unexpected http status
    StatusError(StatusCode)
}

impl Display for DataExtractionError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HeaderNotFound(name) => write!(f, "header '{name}' is not present in response, but is required to correctly extract data"),
            Self::UnsupportedContentType(t, feat) => {
                match feat {
                    Some(feature) => {
                        write!(f, "content type '{t}' is supported only with feature '{feature}', which is disabled")
                    },
                    None => {
                        write!(f, "unsupported content type: {t}")
                    }
                }
            },
            HeaderParseError(name, value) => write!(f, "header {name}: {value} could could not be parsed"),
            Self::ContentParseError(content_type, _) => write!(f, "failed to parse response body with Content-Type: {content_type}"),
            Self::StatusError(code) => write!(f, "Unexpected response status code: {code}")
        }
    }
}

impl Error for DataExtractionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            DataExtractionError::ContentParseError(_, inner) => Some(inner.deref()),
            _ => None
        }
    }
}
/// Utility function to parse Cache-Control headers.
/// Exported so that it can be used in custom extractors.
pub fn parse_cache_control(h: &HeaderValue) -> Result<CacheControl, DataExtractionError>{
    let s = h.to_str().map_err(|_| HeaderParseError(CACHE_CONTROL, "<NON_ASCII_DATA>".to_string()))?;
    CacheControl::from_value(s).ok_or(HeaderParseError(CACHE_CONTROL, s.to_string()))
}

/// Automatic HTTP response deserialization with serde
#[cfg(feature = "serde")]
pub mod serde_extractor {
    use std::error::Error;
    use std::marker::PhantomData;
    use std::time::{Duration, SystemTime};
    use reqwest::header::{CACHE_CONTROL, CONTENT_TYPE};
    use reqwest::Response;
    use serde::de::DeserializeOwned;
    use crate::data_providers::data_provider::DataLoadResult;
    use crate::data_providers::http::{HttpDataExtractor, parse_cache_control};
    use crate::data_providers::http::DataExtractionError::{ContentParseError, HeaderNotFound, StatusError, UnsupportedContentType};

    /// This data extractor automatically deserializes response if its Content-Type is supported.
    /// Cache-Control header is used to determine max age and revalidation policy.
    /// See list of features and MIME types that they provide support for.
    ///
    /// | Feature | Content-Type            |
    /// |---------|-------------------------|
    /// | json    | application/json        |
    /// | toml    | application/toml[^note] |
    /// | yaml    | application/yaml        |
    /// | xml     | application/xml         |
    ///
    /// [^note]: As of 21.06.2024  there is no official MIME type for TOML, so `application/toml` is used
    pub struct SerdeDataExtractor<Data: DeserializeOwned>{
        phantom_data: PhantomData<Data>
    }

    impl <Data: DeserializeOwned + Sync + Send> HttpDataExtractor<Data> for SerdeDataExtractor<Data> {
        /// Extracts data from provided response.
        /// # Errors
        /// Return an error in one the following cases:
        /// - Cache-Control header is not present or can't be parsed
        /// - Content-Type header is not present
        /// - MIME type specified in Content-Type header is not supported
        /// - Body cannot be deserialized into `Data` struct
        async fn extract(&self, response: Response) -> Result<DataLoadResult<Data>, Box<dyn Error>> {
            if !response.status().is_success() {
                return Err(StatusError(response.status()).into())
            }

            let cache_control = parse_cache_control(response.headers().get(CACHE_CONTROL).ok_or(HeaderNotFound(CACHE_CONTROL))?)?;
            let content_type = response.headers().get(CONTENT_TYPE).ok_or(HeaderNotFound(CACHE_CONTROL))?;

            let data: Data = match content_type.to_str()? {
                "application/json" => {
                    #[cfg(not (feature = "json"))] return Err(UnsupportedContentType("application/json".to_string(), Some("json"))).into();

                    #[cfg(feature = "json")] {
                        let bytes = response.bytes().await.map_err(|e| ContentParseError("application/json".to_owned(), Box::new(e)))?;
                        serde_json::de::from_slice::<Data>(&bytes).map_err(|e| ContentParseError("application/json".to_owned(), Box::new(e)))?
                    }
                },
                // NOTE: as of 21.06.2024 no MIME type for TOML is registered officially
                "application/toml" => {
                    #[cfg(not (feature = "toml"))] return Err(Box::new(UnsupportedContentType("application/toml".to_string(), Some("toml"))));

                    #[cfg(feature = "toml")] {
                        let txt = response.text().await.map_err(|e| ContentParseError("application/toml".to_string(), Box::new(e)))?;
                        toml::from_str::<Data>(&txt).map_err(|e| ContentParseError("application/toml".to_string(), Box::new(e)))?
                    }
                },
                "application/yaml" => {
                    #[cfg(not (feature = "yaml"))] return Err(Box::new(UnsupportedContentType("application/yaml".to_string(), Some("yaml"))));

                    #[cfg(feature = "yaml")] {
                        let bytes = response.bytes().await.map_err(|e| ContentParseError("application/yaml".to_owned(), Box::new(e)))?;
                        serde_yaml::from_slice::<Data>(&bytes).map_err(|e| ContentParseError("application/yaml".to_owned(), Box::new(e)))?
                    }
                },
                "application/xml" => {
                    #[cfg(not (feature = "xml"))] return Err(Box::new(UnsupportedContentType("application/yaml".to_string(), Some("xml"))));

                    #[cfg(feature = "xml")] {
                        let txt = response.text().await.map_err(|e| ContentParseError("application/xml".to_string(), Box::new(e)))?;
                        serde_xml_rs::from_str::<Data>(&txt).map_err(|e| ContentParseError("application/xml".to_string(), Box::new(e)))?
                    }
                }
                other => {
                    return Err(Box::new(UnsupportedContentType(other.to_string(), None)));
                }
            };
            Ok(DataLoadResult {
                data,
                must_revalidate: cache_control.must_revalidate,
                valid_until: SystemTime::now() + cache_control.max_age.unwrap_or(Duration::default())
            })
        }
    }

    impl <Data: DeserializeOwned> SerdeDataExtractor<Data> {
        /// Constructs new extractor instance
        pub fn new() -> Self {
            SerdeDataExtractor{phantom_data: PhantomData}
        }
    }
    
    impl<Data: DeserializeOwned> Default for SerdeDataExtractor<Data>{
        fn default() -> Self {
            SerdeDataExtractor::new()
        }
    }
}