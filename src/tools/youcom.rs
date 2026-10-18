//! You.com keyless search backend — free web search, no API key, answers from any IP.
//!
//! Uses the keyless Agents API (`GET https://api.you.com/v1/agents/search`)
//! which is rate-limited to 100 searches/day per IP and needs no key. Results
//! are nested under `results.web`. A keyed upgrade is possible later by sending
//! `X-API-Key` (read from `YDC_API_KEY`) against the same endpoint; this backend
//! stays keyless to fit the unkeyed fallback chain.

use super::backend::SearchBackend;
use crate::types::{
    ContentType, DaedraError, DaedraResult, ResultMetadata, SearchArgs, SearchResponse,
    SearchResult,
};
use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;
use tracing::info;

const YOUCOM_API: &str = "https://api.you.com/v1/agents/search";

/// You.com keyless search backend — general web results via the keyless Agents API.
pub struct YoucomBackend {
    client: Client,
}

/// Top-level response: results nested under `web`.
#[derive(Deserialize)]
struct YoucomResponse {
    #[serde(default)]
    results: YoucomResults,
}

#[derive(Default, Deserialize)]
struct YoucomResults {
    #[serde(default)]
    web: Vec<YoucomItem>,
}

#[derive(Deserialize)]
struct YoucomItem {
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    description: String,
}

impl YoucomBackend {
    /// Create a new You.com keyless backend instance.
    pub fn new() -> Self {
        // The host UA (`daedra/1.0`) is appended with the You.com integration
        // attribution token; this client only calls api.you.com.
        let client = Client::builder()
            .user_agent("daedra/1.0 youdotcom-integration/dirmacs-daedra")
            .timeout(Duration::from_secs(10))
            .build()
            .expect("HTTP client");
        Self { client }
    }
}

impl Default for YoucomBackend {
    fn default() -> Self {
        Self::new()
    }
}

/// Map a You.com result item to a `SearchResult`, dropping entries with no URL
/// (the API response is a trust boundary — never surface a result without one).
fn item_to_result(item: YoucomItem) -> Option<SearchResult> {
    let url = item.url.trim();
    if url.is_empty() {
        return None;
    }
    Some(SearchResult {
        title: if item.title.is_empty() {
            url.to_string()
        } else {
            item.title
        },
        url: url.to_string(),
        description: item.description,
        metadata: ResultMetadata {
            content_type: ContentType::Article,
            source: "youcom".to_string(),
            favicon: None,
            published_date: None,
        },
    })
}

#[async_trait]
impl SearchBackend for YoucomBackend {
    async fn search(&self, args: &SearchArgs) -> DaedraResult<SearchResponse> {
        let opts = args.options.clone().unwrap_or_default();
        let max_str = opts.num_results.to_string();

        let resp = self
            .client
            .get(YOUCOM_API)
            .query(&[
                ("query", args.query.as_str()),
                ("max_results", max_str.as_str()),
            ])
            .send()
            .await
            .map_err(DaedraError::HttpError)?;

        let data: YoucomResponse = resp.json().await.map_err(DaedraError::HttpError)?;

        let results: Vec<SearchResult> = data
            .results
            .web
            .into_iter()
            .take(opts.num_results)
            .filter_map(item_to_result)
            .collect();

        info!(
            backend = "youcom",
            results = results.len(),
            "You.com search complete"
        );
        Ok(SearchResponse::new(args.query.clone(), results, &opts))
    }

    fn name(&self) -> &str {
        "youcom"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_youcom_name() {
        assert_eq!(YoucomBackend::new().name(), "youcom");
    }

    #[test]
    fn test_parse_and_map_sample_response() {
        // Exercises the real deserialization + mapping path against the
        // documented keyless `results.web` shape.
        let json = r#"{
            "results": {
                "web": [
                    {"title": "Rust language", "url": "https://www.rust-lang.org/", "description": "Empowering everyone to build reliable software."},
                    {"title": "No URL", "url": "", "description": "dropped"},
                    {"title": "", "url": "https://example.org/empty-title", "description": "title falls back to url"}
                ]
            }
        }"#;
        let parsed: YoucomResponse = serde_json::from_str(json).unwrap();
        let results: Vec<SearchResult> = parsed
            .results
            .web
            .into_iter()
            .filter_map(item_to_result)
            .collect();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Rust language");
        assert_eq!(results[0].url, "https://www.rust-lang.org/");
        assert_eq!(results[0].metadata.source, "youcom");
        // Empty title falls back to the URL; empty-URL entry is dropped.
        assert_eq!(results[1].title, "https://example.org/empty-title");
    }

    #[test]
    fn test_empty_web_is_no_results() {
        let json = r#"{ "results": { "web": [] } }"#;
        let parsed: YoucomResponse = serde_json::from_str(json).unwrap();
        let results: Vec<SearchResult> = parsed
            .results
            .web
            .into_iter()
            .filter_map(item_to_result)
            .collect();
        assert!(results.is_empty());
    }

    #[test]
    fn test_missing_results_field_defaults_empty() {
        // A malformed response (no `results`) must not panic — it yields no results.
        let json = r#"{ }"#;
        let parsed: YoucomResponse = serde_json::from_str(json).unwrap();
        assert!(parsed.results.web.is_empty());
    }
}
