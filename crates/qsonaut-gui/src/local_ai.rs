use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use reqwest::{blocking::Client, Url};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::net::IpAddr;
use std::path::PathBuf;
use std::time::Duration;

const MAX_IMAGE_RESPONSE_BYTES: usize = 96 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub(super) enum LocalImageProvider {
    #[default]
    Ollama,
    Lemonade,
}

impl LocalImageProvider {
    pub(super) const ALL: [Self; 2] = [Self::Ollama, Self::Lemonade];

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Ollama => "Ollama",
            Self::Lemonade => "Lemonade",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct LocalImageSettings {
    pub(super) provider: LocalImageProvider,
    pub(super) ollama_url: String,
    pub(super) lemonade_url: String,
    pub(super) model: String,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) steps: u32,
}

impl Default for LocalImageSettings {
    fn default() -> Self {
        Self {
            provider: LocalImageProvider::Ollama,
            ollama_url: "http://127.0.0.1:11434".to_string(),
            lemonade_url: "http://localhost:13305/api/v1".to_string(),
            model: String::new(),
            width: 512,
            height: 512,
            steps: 4,
        }
    }
}

impl LocalImageSettings {
    pub(super) fn endpoint(&self) -> &str {
        match self.provider {
            LocalImageProvider::Ollama => &self.ollama_url,
            LocalImageProvider::Lemonade => &self.lemonade_url,
        }
    }

    fn path() -> PathBuf {
        qsonaut_log::app_config_dir().join("local-image.json")
    }

    pub(super) fn load() -> Self {
        let mut settings: Self = std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        if settings.lemonade_url == "http://127.0.0.1:13305" {
            settings.lemonade_url = "http://localhost:13305/api/v1".to_string();
        }
        settings
    }

    pub(super) fn save(&self) -> Result<()> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .context("failed to create QSONaut settings directory")?;
        }
        let json = serde_json::to_vec_pretty(self)?;
        std::fs::write(path, json).context("failed to save local image settings")
    }
}

#[derive(Debug)]
pub(super) enum LocalImageEvent {
    Models(Result<Vec<String>, String>),
    Generated(Result<Vec<u8>, String>),
}

pub(super) fn list_models(settings: &LocalImageSettings) -> Result<Vec<String>> {
    let base = validate_loopback_endpoint(settings.endpoint())?;
    let client = client()?;
    let endpoint = match settings.provider {
        LocalImageProvider::Ollama => base.join("/api/tags")?,
        LocalImageProvider::Lemonade => openai_endpoint(&base, "models")?,
    };
    let value = read_json(
        client
            .get(endpoint)
            .send()
            .context("local model server is unavailable")?,
    )?;
    let models = match settings.provider {
        LocalImageProvider::Ollama => value
            .get("models")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|model| model.get("name").and_then(Value::as_str))
            .map(str::to_string)
            .collect(),
        LocalImageProvider::Lemonade => value
            .get("data")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|model| {
                model
                    .get("downloaded")
                    .and_then(Value::as_bool)
                    .unwrap_or(true)
                    && model
                        .get("labels")
                        .and_then(Value::as_array)
                        .is_some_and(|labels| {
                            labels.iter().any(|label| label.as_str() == Some("image"))
                        })
            })
            .filter_map(|model| model.get("id").and_then(Value::as_str))
            .map(str::to_string)
            .collect(),
    };
    Ok(models)
}

pub(super) fn generate(settings: &LocalImageSettings, prompt: &str) -> Result<Vec<u8>> {
    if settings.model.trim().is_empty() {
        bail!("select a local image model first");
    }
    if prompt.trim().is_empty() {
        bail!("enter an image prompt first");
    }
    let width = settings.width.clamp(256, 2048);
    let height = settings.height.clamp(256, 2048);
    let steps = settings.steps.clamp(1, 100);
    let base = validate_loopback_endpoint(settings.endpoint())?;
    let client = client()?;
    let (endpoint, request) = match settings.provider {
        LocalImageProvider::Ollama => (
            base.join("/api/generate")?,
            json!({
                "model": settings.model,
                "prompt": prompt.trim(),
                "stream": false,
                "width": width,
                "height": height,
                "steps": steps
            }),
        ),
        LocalImageProvider::Lemonade => (
            openai_endpoint(&base, "images/generations")?,
            json!({
                "model": settings.model,
                "prompt": prompt.trim(),
                "size": format!("{width}x{height}"),
                "steps": steps,
                "n": 1,
                "response_format": "b64_json"
            }),
        ),
    };
    let response = client
        .post(endpoint)
        .json(&request)
        .send()
        .context("local image server is unavailable")?;
    let bytes = read_response(response)?;
    let value = parse_json_or_last_ndjson(&bytes)?;
    let encoded = match settings.provider {
        LocalImageProvider::Ollama => value.get("image").and_then(Value::as_str),
        LocalImageProvider::Lemonade => value
            .get("data")
            .and_then(Value::as_array)
            .and_then(|data| data.first())
            .and_then(|image| image.get("b64_json"))
            .and_then(Value::as_str),
    }
    .ok_or_else(|| {
        anyhow!(
            "local server returned no image; verify the selected model supports image generation"
        )
    })?;
    base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .context("local server returned invalid base64 image data")
}

fn openai_endpoint(base: &Url, path: &str) -> Result<Url> {
    let mut normalized = base.clone();
    if !normalized.path().ends_with('/') {
        normalized.set_path(&format!("{}/", normalized.path()));
    }
    normalized
        .join(path.trim_start_matches('/'))
        .context("invalid OpenAI-compatible endpoint path")
}

pub(super) fn validate_loopback_endpoint(raw: &str) -> Result<Url> {
    let url = Url::parse(raw.trim()).context("invalid local server URL")?;
    if url.scheme() != "http" {
        bail!("local AI requires an http:// loopback URL");
    }
    if !url.username().is_empty() || url.password().is_some() {
        bail!("credentials are not allowed in the local AI URL");
    }
    let host = url
        .host_str()
        .ok_or_else(|| anyhow!("local AI URL has no host"))?;
    let address_host = host.trim_start_matches('[').trim_end_matches(']');
    let loopback = host.eq_ignore_ascii_case("localhost")
        || address_host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback());
    if !loopback {
        bail!("local-only policy blocked non-loopback host '{host}'");
    }
    Ok(url)
}

fn client() -> Result<Client> {
    Client::builder()
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(15 * 60))
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .context("failed to create local-only HTTP client")
}

fn read_json(response: reqwest::blocking::Response) -> Result<Value> {
    let bytes = read_response(response)?;
    serde_json::from_slice(&bytes).context("local server returned invalid JSON")
}

fn read_response(response: reqwest::blocking::Response) -> Result<Vec<u8>> {
    let status = response.status();
    let bytes = response
        .bytes()
        .context("failed reading local server response")?;
    if bytes.len() > MAX_IMAGE_RESPONSE_BYTES {
        bail!("local server response exceeded 96 MiB safety limit");
    }
    if !status.is_success() {
        let detail = String::from_utf8_lossy(&bytes);
        bail!(
            "local server returned {status}: {}",
            detail.chars().take(500).collect::<String>()
        );
    }
    Ok(bytes.to_vec())
}

fn parse_json_or_last_ndjson(bytes: &[u8]) -> Result<Value> {
    if let Ok(value) = serde_json::from_slice(bytes) {
        return Ok(value);
    }
    bytes
        .split(|byte| *byte == b'\n')
        .rev()
        .find(|line| !line.iter().all(u8::is_ascii_whitespace))
        .ok_or_else(|| anyhow!("local server returned an empty response"))
        .and_then(|line| serde_json::from_slice(line).context("local server returned invalid JSON"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_policy_allows_only_loopback_http() {
        assert!(validate_loopback_endpoint("http://localhost:11434").is_ok());
        assert!(validate_loopback_endpoint("http://127.0.0.1:13305").is_ok());
        assert!(validate_loopback_endpoint("http://[::1]:11434").is_ok());
        assert!(validate_loopback_endpoint("https://localhost:11434").is_err());
        assert!(validate_loopback_endpoint("http://192.168.1.5:11434").is_err());
        assert!(validate_loopback_endpoint("http://example.com").is_err());
    }

    #[test]
    fn parses_ollama_stream_final_record() {
        let value =
            parse_json_or_last_ndjson(b"{\"done\":false}\n{\"image\":\"abc\",\"done\":true}\n")
                .unwrap();
        assert_eq!(value["image"], "abc");
    }

    #[test]
    fn appends_openai_paths_to_lemonade_api_base() {
        let base = Url::parse("http://localhost:13305/api/v1").unwrap();
        assert_eq!(
            openai_endpoint(&base, "models").unwrap().as_str(),
            "http://localhost:13305/api/v1/models"
        );
    }
}
