//! Embeddings over a pluggable, OpenAI-compatible HTTP endpoint.
//!
//! No model is baked into the binary. Point `endpoint` at a local Ollama
//! (`http://localhost:11434/v1`, private — nothing leaves the network), an
//! internal service, or a hosted API. The same model is used to embed both
//! documents (at build time) and queries (at search time); the model name and
//! dimensionality are recorded in the index so `recall` can refuse a mismatch.

use crate::config::EmbeddingConfig;
use crate::error::{Error, Result};
use serde_json::json;

pub struct Embedder {
    endpoint: String,
    model: String,
    api_key: Option<String>,
    pub dims: usize,
}

impl Embedder {
    pub fn from_config(cfg: &EmbeddingConfig) -> Result<Self> {
        let api_key = match &cfg.api_key_env {
            Some(var) => match std::env::var(var) {
                Ok(v) if !v.is_empty() => Some(v),
                _ => {
                    return Err(Error::Embed(format!(
                        "api_key_env `{var}` is set in config but the env var is empty/unset"
                    )))
                }
            },
            None => None,
        };
        Ok(Self {
            endpoint: cfg.endpoint.trim_end_matches('/').to_string(),
            model: cfg.model.clone(),
            api_key,
            dims: cfg.dims,
        })
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Embed a batch of inputs. Returns one vector per input, in order.
    pub fn embed_batch(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>> {
        if inputs.is_empty() {
            return Ok(Vec::new());
        }
        let url = format!("{}/embeddings", self.endpoint);
        let mut req = ureq::post(&url).set("content-type", "application/json");
        if let Some(key) = &self.api_key {
            req = req.set("authorization", &format!("Bearer {key}"));
        }

        let resp = req
            .send_json(json!({ "model": self.model, "input": inputs }))
            .map_err(|e| map_ureq(e, &url))?;

        let body: serde_json::Value = resp
            .into_json()
            .map_err(|e| Error::Embed(format!("invalid JSON from {url}: {e}")))?;

        let data = body
            .get("data")
            .and_then(|d| d.as_array())
            .ok_or_else(|| Error::Embed(format!("no `data` array in response from {url}")))?;

        if data.len() != inputs.len() {
            return Err(Error::Embed(format!(
                "endpoint returned {} embeddings for {} inputs",
                data.len(),
                inputs.len()
            )));
        }

        let mut out = Vec::with_capacity(data.len());
        for (i, item) in data.iter().enumerate() {
            let arr = item
                .get("embedding")
                .and_then(|e| e.as_array())
                .ok_or_else(|| Error::Embed(format!("item {i} has no `embedding` array")))?;
            let vec: Vec<f32> = arr
                .iter()
                .map(|v| v.as_f64().map(|f| f as f32))
                .collect::<Option<_>>()
                .ok_or_else(|| Error::Embed(format!("item {i} embedding is not all numbers")))?;
            if vec.len() != self.dims {
                return Err(Error::Embed(format!(
                    "model `{}` returned dim {} but config says dims = {} — fix `embedding.dims`",
                    self.model,
                    vec.len(),
                    self.dims
                )));
            }
            out.push(vec);
        }
        Ok(out)
    }

    pub fn embed_one(&self, input: &str) -> Result<Vec<f32>> {
        Ok(self.embed_batch(&[input.to_string()])?.swap_remove(0))
    }
}

fn map_ureq(e: ureq::Error, url: &str) -> Error {
    match e {
        ureq::Error::Status(code, resp) => {
            let detail = resp.into_string().unwrap_or_default();
            let hint = if code == 404 {
                "  (is the endpoint OpenAI-compatible? Ollama needs the /v1 base)"
            } else if code == 401 || code == 403 {
                "  (auth — check api_key_env)"
            } else {
                ""
            };
            Error::Embed(format!("{url} returned {code}: {}{hint}", detail.trim()))
        }
        ureq::Error::Transport(t) => Error::Embed(format!(
            "cannot reach {url}: {t}  (is the embedding server running?)"
        )),
    }
}
