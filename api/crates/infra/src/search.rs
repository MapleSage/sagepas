use anyhow::Context;
use serde::{Deserialize, Serialize};

/// Client for Azure AI Search REST API (2023-11-01).
#[derive(Clone, Debug)]
pub struct SearchClient {
    endpoint: String,
    key: String,
    http: reqwest::Client,
}

/// A single document returned from a search query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    /// Relevance score assigned by Azure AI Search.
    pub score: f64,
    /// Main body text of the document chunk.
    pub content: String,
    pub title: Option<String>,
    pub source: Option<String>,
    pub category: Option<String>,
    pub subcategory: Option<String>,
}

/// Wire shape of an individual Azure Search document.
#[derive(Debug, Deserialize)]
struct RawSearchDoc {
    #[serde(rename = "@search.score")]
    search_score: f64,
    #[serde(default)]
    content: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    subcategory: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    value: Vec<RawSearchDoc>,
}

impl SearchClient {
    pub fn new(endpoint: &str, key: &str) -> Self {
        Self {
            endpoint: endpoint.trim_end_matches('/').to_string(),
            key: key.to_string(),
            http: reqwest::Client::new(),
        }
    }

    /// Perform a vector (ANN) search against the named index.
    ///
    /// Uses `vectorQueries` with `kind: "vector"` and the default
    /// `content_vector` field as per the SageInsure Search index schema.
    pub async fn vector_search(
        &self,
        index: &str,
        vector: Vec<f32>,
        top_k: usize,
    ) -> anyhow::Result<Vec<SearchResult>> {
        let url = format!(
            "{}/indexes/{}/docs/search?api-version=2023-11-01",
            self.endpoint, index
        );

        let body = serde_json::json!({
            "vectorQueries": [{
                "kind": "vector",
                "vector": vector,
                "fields": "content_vector",
                "k": top_k
            }],
            "select": "content,title,source,category,subcategory",
            "top": top_k
        });

        self.execute_search(&url, &body, "vector").await
    }

    /// Perform a full-text (BM25) search against the named index.
    pub async fn text_search(
        &self,
        index: &str,
        query: &str,
        top_k: usize,
    ) -> anyhow::Result<Vec<SearchResult>> {
        let url = format!(
            "{}/indexes/{}/docs/search?api-version=2023-11-01",
            self.endpoint, index
        );

        let body = serde_json::json!({
            "search": query,
            "select": "content,title,source,category,subcategory",
            "top": top_k,
            "queryType": "simple"
        });

        self.execute_search(&url, &body, "text").await
    }

    pub async fn search_kb(
        &self,
        index: &str,
        query: &str,
        category: Option<&str>,
        top_k: usize,
    ) -> anyhow::Result<Vec<SearchResult>> {
        let url = format!(
            "{}/indexes/{}/docs/search?api-version=2023-11-01",
            self.endpoint, index
        );

        let mut body = serde_json::json!({
            "search": query,
            "select": "content,title,source,category,subcategory",
            "top": top_k,
            "queryType": "simple"
        });

        if let Some(cat) = category.filter(|c| !c.trim().is_empty()) {
            let escaped = cat.replace('\'', "''");
            body["filter"] = serde_json::Value::String(format!("category eq '{}'", escaped));
        }

        self.execute_search(&url, &body, "kb").await
    }

    async fn execute_search(
        &self,
        url: &str,
        body: &serde_json::Value,
        kind: &str,
    ) -> anyhow::Result<Vec<SearchResult>> {
        let resp = self
            .http
            .post(url)
            .header("api-key", &self.key)
            .header("Content-Type", "application/json")
            .json(body)
            .send()
            .await
            .with_context(|| format!("Azure Search {} request failed", kind))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Azure Search returned {}: {}", status, text);
        }

        let parsed: SearchResponse = resp
            .json()
            .await
            .with_context(|| format!("Failed to parse Azure Search {} response", kind))?;

        Ok(parsed.value.into_iter().map(doc_to_result).collect())
    }
}

fn doc_to_result(doc: RawSearchDoc) -> SearchResult {
    SearchResult {
        score: doc.search_score,
        content: doc.content,
        title: doc.title,
        source: doc.source,
        category: doc.category,
        subcategory: doc.subcategory,
    }
}
