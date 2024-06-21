use std::error::Error;
use std::fmt::{Display, Formatter};
use cache_control::CacheControl;
use reqwest::header::{CACHE_CONTROL, HeaderName, HeaderValue};
use reqwest::Url;
use crate::data_providers::data_provider::{DataLoadResult, DataProvider};
use crate::data_providers::http::DataExtractionError::HeaderParseError;

pub trait HttpDataExtractor<Data> {
    async fn extract(response: reqwest::Response) -> Result<DataLoadResult<Data>, Box<dyn Error>>;
}

pub struct HttpDataProvider<Data, Extractor: HttpDataExtractor<Data>> {
    extractor: Extractor,
    client: reqwest::Client,
    url: Url
}

impl <Data, Extractor: HttpDataExtractor<Data>> DataProvider<Data> for HttpDataProvider<Data, Extractor> {
    async fn load_data<Data>(&self) -> Result<DataLoadResult<Data>, Box<dyn Error>> {
        return self.extractor.extract(self.client.get(&self.url).send().await?).await;
    }
}

#[derive(Debug)]
pub enum DataExtractionError {
    HeaderNotFound(HeaderName),
    HeaderParseError(HeaderName, String),
    UnsupportedContentType(String, Option<&'static str>), // Optional feature name can be provided
    ContentParseError(String, Box<dyn Error>)
}

impl Display for DataExtractionError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HeaderNotFound(name) => write!(f, "header '{name}' is not present in response, but is required to correctly extract data"),
            Self::UnsupportedContentType(t, f) => {
                match f {
                    Some(feature) => {
                        write!(f, "content type '{t}' is supported only with feature '{feature}', which is disabled")
                    },
                    None => {
                        write!(f, "unsupported content type: {t}")
                    }
                }
            },
            HeaderParseError(name, value) => write!(f, "header {name}: {value} could could not be parsed"),
            Self::ContentParseError(content_type, _) => write!(f, "failed to parse response body with Content-Type: {content_type}")
        }
    }
}

impl Error for DataExtractionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            DataExtractionError::ContentParseError(_, inner) => inner.downcast_ref(),
            _ => None
        }
    }
}
pub fn parse_cache_control(h: &HeaderValue) -> Result<CacheControl, HeaderParseError>{
    let s = h.to_str().map_err(|e| HeaderParseError(CACHE_CONTROL, "<NON_ASCII_DATA>".to_string()))?;
    return CacheControl::from_value(s).ok_or(HeaderParseError(CACHE_CONTROL, s.to_string()));
}

#[cfg(feature = "serde")]
pub mod serde_extractor {
    use std::error::Error;
    use std::fmt::{Display};
    use std::time::Duration;
    use reqwest::header::{CACHE_CONTROL, CONTENT_TYPE};
    use reqwest::Response;
    use serde::de::DeserializeOwned;
    use crate::data_providers::data_provider::DataLoadResult;
    use crate::data_providers::http::{HttpDataExtractor, parse_cache_control};
    use crate::data_providers::http::DataExtractionError::{ContentParseError, HeaderNotFound, UnsupportedContentType};

    pub struct SerdeDataExtractor<Data: DeserializeOwned>;

    impl <Data: DeserializeOwned> HttpDataExtractor<Data> for SerdeDataExtractor<Data> {
        async fn extract(response: Response) -> Result<DataLoadResult<Data>, Box<dyn Error>> {
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
                    #[cfg(not (feature = "toml"))] return Err(UnsupportedContentType("application/toml".to_string(), Some("toml"))).into();

                    #[cfg(feature = "toml")] {
                        let txt = response.text().await.map_err(|e| ContentParseError("application/toml".to_string(), Box::new(e)))?;
                        toml::de::<Data>::from_str(&txt).map_err(|e| ContentParseError("application/toml".to_string(), Box::new(e)))?
                    }
                },
                "application/yaml" => {
                    #[cfg(not (feature = "yaml"))] return Err(UnsupportedContentType("application/yaml".to_string(), Some("yaml"))).into();

                    #[cfg(feature = "yaml")] {
                        let bytes = response.bytes().await.map_err(|e| ContentParseError("application/yaml".to_owned(), Box::new(e)))?;
                        serde_yaml::<Data>::from_slice(&bytes).map_err(|e| ContentParseError("application/yaml".to_owned(), Box::new(e)))?
                    }
                },
                other => {
                    return Err(UnsupportedContentType(other.to_string(), None)).into()
                }
            };
            return Ok(DataLoadResult {
                data,
                must_revalidate: cache_control.must_revalidate,
                max_age: cache_control.max_age.unwrap_or(Duration::default()),
            })
        }
    }
}