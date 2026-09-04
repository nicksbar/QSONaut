use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use reqwest::{
    blocking::{multipart, Client},
    Url,
};
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
    pub(super) vision_model: String,
    pub(super) image_model: String,
    pub(super) edit_model: String,
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
            vision_model: String::new(),
            image_model: String::new(),
            edit_model: String::new(),
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
        if settings.image_model.is_empty() && !settings.model.is_empty() {
            settings.image_model = settings.model.clone();
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LocalModelRole {
    Vision,
    Image,
    Edit,
}

impl LocalModelRole {
    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Vision => "vision/context",
            Self::Image => "image generation",
            Self::Edit => "image editing",
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct LocalModelCapabilities {
    pub(super) vision: bool,
    pub(super) image: bool,
    pub(super) edit: bool,
    pub(super) chat: bool,
    pub(super) metadata_available: bool,
}

impl LocalModelCapabilities {
    fn supports(&self, role: LocalModelRole) -> bool {
        match role {
            LocalModelRole::Vision => self.vision && self.chat,
            LocalModelRole::Image => self.image,
            LocalModelRole::Edit => self.image && self.edit,
        }
    }

    pub(super) fn summary(&self) -> String {
        let mut parts = Vec::new();
        if self.vision {
            parts.push("vision");
        }
        if self.chat {
            parts.push("chat");
        }
        if self.image {
            parts.push("image");
        }
        if self.edit {
            parts.push("edit");
        }
        if parts.is_empty() {
            if self.metadata_available {
                "no role capabilities advertised".to_string()
            } else {
                "capability metadata unavailable".to_string()
            }
        } else {
            parts.join(" + ")
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct LocalModelInfo {
    pub(super) id: String,
    pub(super) provider: LocalImageProvider,
    pub(super) recipe: Option<String>,
    pub(super) labels: Vec<String>,
    pub(super) capabilities: LocalModelCapabilities,
    pub(super) downloaded: bool,
    pub(super) size_gb: Option<f64>,
    pub(super) parameter_size: Option<String>,
}

impl LocalModelInfo {
    pub(super) fn supports(&self, role: LocalModelRole) -> bool {
        self.downloaded && self.capabilities.supports(role)
    }

    pub(super) fn role_unavailable_reason(&self, role: LocalModelRole) -> Option<String> {
        if !self.downloaded {
            return Some("not downloaded for local execution".to_string());
        }
        if self.supports(role) {
            return None;
        }
        if !self.capabilities.metadata_available {
            return Some(format!(
                "{} capability unavailable: provider did not advertise role metadata",
                role.label()
            ));
        }
        Some(format!(
            "{} capability unavailable: advertised capabilities are {}",
            role.label(),
            self.capabilities.summary()
        ))
    }

    pub(super) fn detail(&self) -> String {
        let mut detail = format!(
            "{}; provider {}",
            self.capabilities.summary(),
            self.provider.label()
        );
        if let Some(recipe) = &self.recipe {
            detail.push_str(&format!("; recipe {recipe}"));
        }
        if !self.labels.is_empty() {
            detail.push_str(&format!("; labels {}", self.labels.join(", ")));
        }
        if let Some(parameters) = &self.parameter_size {
            detail.push_str(&format!("; {parameters}"));
        }
        if let Some(size_gb) = self.size_gb {
            detail.push_str(&format!("; approx {size_gb:.2} GB reported by provider"));
        }
        detail
    }
}

#[derive(Debug)]
pub(super) enum LocalImageEvent {
    Models(Result<Vec<LocalModelInfo>, String>),
    Vision(Result<(String, String), String>),
    Generated(Result<Vec<u8>, String>),
    Edited(Result<Vec<u8>, String>),
}

pub(super) fn list_models(settings: &LocalImageSettings) -> Result<Vec<LocalModelInfo>> {
    let base = validate_loopback_endpoint(settings.endpoint())?;
    let client = client()?;
    let endpoint = match settings.provider {
        LocalImageProvider::Ollama => base.join("/api/tags")?,
        LocalImageProvider::Lemonade => openai_endpoint(&base, "models")?,
    };
    tracing::info!(provider = %settings.provider.label(), endpoint = %endpoint, "requesting local AI model list");
    let value = read_json(
        client
            .get(endpoint)
            .send()
            .context("local model server is unavailable")?,
    )?;
    let models: Vec<LocalModelInfo> = match settings.provider {
        LocalImageProvider::Ollama => value
            .get("models")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("Ollama response did not contain a models array"))?
            .iter()
            .filter_map(|model| {
                let id = model.get("name").and_then(Value::as_str)?.to_string();
                let parameter_size = model
                    .pointer("/details/parameter_size")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                Some(LocalModelInfo {
                    id,
                    provider: settings.provider,
                    recipe: None,
                    labels: Vec::new(),
                    capabilities: LocalModelCapabilities {
                        vision: false,
                        image: false,
                        edit: false,
                        chat: false,
                        metadata_available: false,
                    },
                    downloaded: true,
                    size_gb: model
                        .get("size")
                        .and_then(Value::as_u64)
                        .map(|bytes| bytes as f64 / 1_073_741_824.0),
                    parameter_size,
                })
            })
            .collect(),
        LocalImageProvider::Lemonade => value
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("Lemonade response did not contain a data array"))?
            .iter()
            .filter_map(|model| parse_lemonade_model(model, settings.provider))
            .filter(|model| model.downloaded)
            .collect(),
    };
    for model in &models {
        tracing::debug!(
            provider = %model.provider.label(),
            model = %model.id,
            labels = ?model.labels,
            capabilities = %model.capabilities.summary(),
            vision_role = model.supports(LocalModelRole::Vision),
            image_role = model.supports(LocalModelRole::Image),
            edit_role = model.supports(LocalModelRole::Edit),
            "local AI model capability filtering decision"
        );
    }
    tracing::info!(provider = %settings.provider.label(), model_count = models.len(), "local AI model list received");
    Ok(models)
}

fn parse_lemonade_model(model: &Value, provider: LocalImageProvider) -> Option<LocalModelInfo> {
    let id = model.get("id").and_then(Value::as_str)?.to_string();
    let downloaded = model
        .get("downloaded")
        .or_else(|| model.get("is_downloaded"))
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let labels = collect_capability_tokens(model);
    let has_metadata = !labels.is_empty()
        || model.get("capabilities").is_some()
        || model.get("labels").is_some()
        || model.get("supports").is_some();
    let has = |needles: &[&str]| labels.iter().any(|label| needles.contains(&label.as_str()));
    let vision = has(&["vision", "image_input", "image-input", "multimodal"]);
    let image = has(&[
        "image",
        "image_generation",
        "image-generation",
        "text-to-image",
    ]);
    let edit = has(&[
        "edit",
        "image_edit",
        "image-edit",
        "image_editing",
        "image-editing",
        "images/edits",
    ]);
    let explicit_chat = has(&[
        "chat",
        "completion",
        "completions",
        "chat_completions",
        "multimodal",
    ]);
    // Lemonade's model inventory uses `vision` for multimodal LLMs served by
    // /chat/completions; current catalog entries do not also carry `chat`.
    let capabilities = LocalModelCapabilities {
        vision,
        image,
        edit,
        chat: explicit_chat || vision,
        metadata_available: has_metadata,
    };
    Some(LocalModelInfo {
        id,
        provider,
        recipe: model
            .get("recipe")
            .and_then(Value::as_str)
            .or_else(|| model.pointer("/metadata/recipe").and_then(Value::as_str))
            .map(str::to_string),
        labels,
        capabilities,
        downloaded,
        size_gb: model.get("size").and_then(Value::as_f64).or_else(|| {
            model
                .get("size_bytes")
                .or_else(|| model.get("file_size"))
                .or_else(|| model.get("file_size_bytes"))
                .and_then(Value::as_u64)
                .map(|bytes| bytes as f64 / 1_073_741_824.0)
        }),
        parameter_size: model
            .get("parameter_size")
            .or_else(|| model.get("parameters"))
            .or_else(|| model.pointer("/details/parameter_size"))
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

fn collect_capability_tokens(model: &Value) -> Vec<String> {
    let mut labels = Vec::new();
    for key in ["labels", "capabilities", "supports"] {
        collect_tokens_from_value(model.get(key), &mut labels);
    }
    labels.sort();
    labels.dedup();
    labels
}

fn collect_tokens_from_value(value: Option<&Value>, labels: &mut Vec<String>) {
    match value {
        Some(Value::Array(values)) => {
            for value in values {
                collect_tokens_from_value(Some(value), labels);
            }
        }
        Some(Value::Object(map)) => {
            for (key, value) in map {
                match value {
                    Value::Bool(true) => labels.push(normalize_capability_token(key)),
                    Value::String(text) => {
                        labels.push(normalize_capability_token(key));
                        labels.push(normalize_capability_token(text));
                    }
                    Value::Array(_) | Value::Object(_) => {
                        labels.push(normalize_capability_token(key));
                        collect_tokens_from_value(Some(value), labels);
                    }
                    _ => {}
                }
            }
        }
        Some(Value::String(text)) => labels.push(normalize_capability_token(text)),
        _ => {}
    }
}

fn normalize_capability_token(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace(' ', "_")
}

pub(super) fn model_for_role<'a>(
    models: &'a [LocalModelInfo],
    selected: &str,
    role: LocalModelRole,
) -> Result<&'a LocalModelInfo> {
    let selected = selected.trim();
    if selected.is_empty() {
        bail!("select a {} model first", role.label());
    }
    let model = models
        .iter()
        .find(|model| model.id == selected)
        .ok_or_else(|| {
            anyhow!(
                "selected {} model '{selected}' is not in the current provider inventory; refresh models or choose another compatible model",
                role.label()
            )
        })?;
    if let Some(reason) = model.role_unavailable_reason(role) {
        bail!("{reason}");
    }
    Ok(model)
}

pub(super) fn analyze_image(
    settings: &LocalImageSettings,
    model_id: &str,
    image_bytes: &[u8],
    instruction: &str,
) -> Result<String> {
    if model_id.trim().is_empty() {
        bail!("select a vision/context model first");
    }
    if image_bytes.is_empty() {
        bail!("select a received SSTV image first");
    }
    let base = validate_loopback_endpoint(settings.endpoint())?;
    let endpoint = match settings.provider {
        LocalImageProvider::Lemonade => openai_endpoint(&base, "chat/completions")?,
        LocalImageProvider::Ollama => {
            bail!("Ollama vision is unavailable because this integration has no role capability metadata")
        }
    };
    let request = build_vision_request(model_id, image_bytes, instruction);
    tracing::info!(provider = %settings.provider.label(), endpoint = %endpoint, role = "vision", selected_model = %model_id, "requesting local vision analysis");
    let value = read_json(
        client()?
            .post(endpoint)
            .json(&request)
            .send()
            .context("local vision server is unavailable")?,
    )
    .map_err(add_model_loading_guidance)?;
    value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(parse_assistant_content)
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| anyhow!("local vision response did not contain assistant text"))
}

fn build_vision_request(model_id: &str, image_bytes: &[u8], instruction: &str) -> Value {
    let encoded = base64::engine::general_purpose::STANDARD.encode(image_bytes);
    json!({
        "model": model_id.trim(),
        "messages": [{
            "role": "user",
            "content": [
                {"type": "text", "text": instruction.trim()},
                {"type": "image_url", "image_url": {"url": format!("data:image/png;base64,{encoded}")}}
            ]
        }],
        "stream": false
    })
}

fn parse_assistant_content(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return Some(text.to_string());
    }
    let parts = value.as_array()?;
    let text = parts
        .iter()
        .filter_map(|part| {
            part.get("text")
                .and_then(Value::as_str)
                .or_else(|| part.get("content").and_then(Value::as_str))
        })
        .collect::<Vec<_>>()
        .join("\n");
    Some(text)
}

pub(super) fn generate_with_model(
    settings: &LocalImageSettings,
    model_id: &str,
    prompt: &str,
) -> Result<Vec<u8>> {
    if model_id.trim().is_empty() {
        bail!("select an image-generation model first");
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
                "model": model_id.trim(),
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
                "model": model_id.trim(),
                "prompt": prompt.trim(),
                "size": format!("{width}x{height}"),
                "steps": steps,
                "n": 1,
                "response_format": "b64_json"
            }),
        ),
    };
    tracing::info!(
        provider = %settings.provider.label(),
        endpoint = %endpoint,
        role = "image",
        selected_model = %model_id,
        width,
        height,
        steps,
        "requesting local image generation"
    );
    let response = client
        .post(endpoint)
        .json(&request)
        .send()
        .context("local image server is unavailable")?;
    parse_image_response(settings.provider, response).map_err(add_model_loading_guidance)
}

pub(super) fn edit_image(
    settings: &LocalImageSettings,
    model_id: &str,
    prompt: &str,
    source_png: Vec<u8>,
) -> Result<Vec<u8>> {
    if settings.provider != LocalImageProvider::Lemonade {
        bail!(
            "received-image reinterpretation requires a provider with image-edit endpoint support"
        );
    }
    if model_id.trim().is_empty() {
        bail!("select an image-editing model first");
    }
    if prompt.trim().is_empty() {
        bail!("enter an image reinterpretation prompt first");
    }
    if source_png.is_empty() {
        bail!("select a received SSTV image first");
    }
    let width = settings.width.clamp(256, 2048);
    let height = settings.height.clamp(256, 2048);
    let steps = settings.steps.clamp(1, 100);
    let base = validate_loopback_endpoint(settings.endpoint())?;
    let endpoint = openai_endpoint(&base, "images/edits")?;
    tracing::info!(
        provider = %settings.provider.label(),
        endpoint = %endpoint,
        role = "edit",
        selected_model = %model_id,
        width,
        height,
        steps,
        "requesting local received-image reinterpretation"
    );
    let form = multipart::Form::new()
        .text("model", model_id.trim().to_string())
        .text("prompt", prompt.trim().to_string())
        .text("size", format!("{width}x{height}"))
        .text("n", "1")
        .text("response_format", "b64_json")
        .text("steps", steps.to_string())
        .part(
            "image",
            multipart::Part::bytes(source_png)
                .file_name("received-sstv.png")
                .mime_str("image/png")
                .context("failed to prepare received image upload")?,
        );
    let response = client()?
        .post(endpoint)
        .multipart(form)
        .send()
        .context("local image edit server is unavailable")?;
    parse_image_response(settings.provider, response).map_err(add_model_loading_guidance)
}

fn add_model_loading_guidance(error: anyhow::Error) -> anyhow::Error {
    let detail = format!("{error:#}");
    let lower = detail.to_ascii_lowercase();
    if lower.contains("load model")
        || lower.contains("loading model")
        || lower.contains("out of memory")
        || lower.contains("vram")
        || lower.contains("allocation failed")
    {
        anyhow!(
            "{detail}. The provider may need fewer loaded models or a different compatible model; your selection was kept."
        )
    } else {
        error
    }
}

fn parse_image_response(
    provider: LocalImageProvider,
    response: reqwest::blocking::Response,
) -> Result<Vec<u8>> {
    let bytes = read_response(response)?;
    let value = parse_json_or_last_ndjson(&bytes)?;
    let encoded = match provider {
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
    let base_path = normalized.path().trim_matches('/');
    if base_path.is_empty() {
        normalized.set_path("/v1/");
    } else if !normalized.path().ends_with('/') {
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
        let base = Url::parse("http://localhost:13305/v1").unwrap();
        assert_eq!(
            openai_endpoint(&base, "images/generations")
                .unwrap()
                .as_str(),
            "http://localhost:13305/v1/images/generations"
        );
        let base = Url::parse("http://localhost:13305").unwrap();
        assert_eq!(
            openai_endpoint(&base, "models").unwrap().as_str(),
            "http://localhost:13305/v1/models"
        );
    }

    #[test]
    fn parses_lemonade_role_labels_from_current_inventory_shape() {
        let vision = parse_lemonade_model(
            &json!({
                "id": "Qwen3-VL-8B-Instruct-GGUF",
                "downloaded": true,
                "labels": ["vision", "tool-calling"],
                "recipe": "llamacpp",
                "size": 5.76
            }),
            LocalImageProvider::Lemonade,
        )
        .unwrap();
        assert!(vision.supports(LocalModelRole::Vision));
        assert!(!vision.supports(LocalModelRole::Image));
        assert!(!vision.supports(LocalModelRole::Edit));
        assert_eq!(vision.size_gb, Some(5.76));

        let editor = parse_lemonade_model(
            &json!({
                "id": "Flux-2-Klein-4B",
                "downloaded": true,
                "labels": ["image", "edit"],
                "recipe": "sd-cpp",
                "size": 15.0
            }),
            LocalImageProvider::Lemonade,
        )
        .unwrap();
        assert!(!editor.supports(LocalModelRole::Vision));
        assert!(editor.supports(LocalModelRole::Image));
        assert!(editor.supports(LocalModelRole::Edit));

        let generator = parse_lemonade_model(
            &json!({
                "id": "SDXL-Turbo",
                "downloaded": true,
                "labels": ["image"]
            }),
            LocalImageProvider::Lemonade,
        )
        .unwrap();
        assert!(generator.supports(LocalModelRole::Image));
        assert!(!generator.supports(LocalModelRole::Edit));
    }

    #[test]
    fn does_not_infer_roles_without_provider_metadata() {
        let model = parse_lemonade_model(
            &json!({"id": "looks-like-a-vision-image-model", "downloaded": true}),
            LocalImageProvider::Lemonade,
        )
        .unwrap();
        assert!(!model.supports(LocalModelRole::Vision));
        assert!(!model.supports(LocalModelRole::Image));
        assert!(!model.supports(LocalModelRole::Edit));
        assert!(!model.capabilities.metadata_available);
    }

    #[test]
    fn builds_openai_multimodal_vision_request_with_png_data_url() {
        let request = build_vision_request(" vision-model ", &[0, 1, 2, 3], " inspect ");
        assert_eq!(request["model"], "vision-model");
        assert_eq!(request["stream"], false);
        assert_eq!(request["messages"][0]["content"][0]["type"], "text");
        assert_eq!(request["messages"][0]["content"][0]["text"], "inspect");
        assert_eq!(
            request["messages"][0]["content"][1]["image_url"]["url"],
            "data:image/png;base64,AAECAw=="
        );
    }

    #[test]
    fn parses_string_and_part_array_assistant_content() {
        assert_eq!(
            parse_assistant_content(&json!("plain response")).as_deref(),
            Some("plain response")
        );
        assert_eq!(
            parse_assistant_content(&json!([
                {"type": "text", "text": "first"},
                {"type": "text", "text": "second"}
            ]))
            .as_deref(),
            Some("first\nsecond")
        );
    }

    #[test]
    fn local_provider_and_role_labels_are_stable() {
        assert_eq!(LocalImageProvider::Ollama.label(), "Ollama");
        assert_eq!(LocalImageProvider::Lemonade.label(), "Lemonade");
        assert_eq!(LocalModelRole::Vision.label(), "vision/context");
        assert_eq!(LocalModelRole::Image.label(), "image generation");
        assert_eq!(LocalModelRole::Edit.label(), "image editing");
        assert_eq!(
            LocalImageSettings::default().endpoint(),
            "http://127.0.0.1:11434"
        );
        let settings = LocalImageSettings {
            provider: LocalImageProvider::Lemonade,
            ..LocalImageSettings::default()
        };
        assert_eq!(settings.endpoint(), "http://localhost:13305/api/v1");
    }

    #[test]
    fn model_capabilities_report_supported_roles_and_empty_metadata() {
        let no_metadata = LocalModelCapabilities {
            vision: false,
            image: false,
            edit: false,
            chat: false,
            metadata_available: false,
        };
        assert_eq!(no_metadata.summary(), "capability metadata unavailable");
        let advertised = LocalModelCapabilities {
            vision: true,
            image: true,
            edit: true,
            chat: true,
            metadata_available: true,
        };
        assert_eq!(advertised.summary(), "vision + chat + image + edit");
        assert!(advertised.supports(LocalModelRole::Vision));
        assert!(advertised.supports(LocalModelRole::Image));
        assert!(advertised.supports(LocalModelRole::Edit));
        let vision_without_chat = LocalModelCapabilities {
            chat: false,
            ..advertised
        };
        assert!(!vision_without_chat.supports(LocalModelRole::Vision));
    }

    #[test]
    fn model_selection_explains_empty_unknown_undownloaded_and_incompatible_models() {
        let model = LocalModelInfo {
            id: "vision".to_string(),
            provider: LocalImageProvider::Lemonade,
            recipe: Some("llamacpp".to_string()),
            labels: vec!["vision".to_string()],
            capabilities: LocalModelCapabilities {
                vision: true,
                image: false,
                edit: false,
                chat: true,
                metadata_available: true,
            },
            downloaded: true,
            size_gb: Some(2.5),
            parameter_size: Some("8B".to_string()),
        };
        let models = [model.clone()];
        assert!(model_for_role(&models, "", LocalModelRole::Vision).is_err());
        assert!(model_for_role(&models, "missing", LocalModelRole::Vision).is_err());
        assert!(model_for_role(&models, "vision", LocalModelRole::Vision).is_ok());
        assert!(model_for_role(&models, "vision", LocalModelRole::Image).is_err());
        assert!(model.detail().contains("recipe llamacpp"));
        assert!(model.detail().contains("approx 2.50 GB"));

        let undownloaded = LocalModelInfo {
            downloaded: false,
            ..model.clone()
        };
        assert!(undownloaded
            .role_unavailable_reason(LocalModelRole::Vision)
            .unwrap()
            .contains("not downloaded"));
        let unavailable_metadata = LocalModelInfo {
            capabilities: LocalModelCapabilities {
                metadata_available: false,
                ..model.capabilities.clone()
            },
            ..model
        };
        assert!(unavailable_metadata
            .role_unavailable_reason(LocalModelRole::Image)
            .unwrap()
            .contains("provider did not advertise"));
    }

    #[test]
    fn capability_tokens_collect_nested_and_normalize_variants() {
        let model = json!({
            "labels": ["Vision", {"image generation": true}],
            "capabilities": {"edit": "image-editing", "ignored": false},
            "supports": {"nested": ["Chat", {"multimodal": true}]}
        });
        let labels = collect_capability_tokens(&model);
        assert!(labels.contains(&"vision".to_string()));
        assert!(labels.contains(&"image_generation".to_string()));
        assert!(labels.contains(&"image-editing".to_string()));
        assert!(labels.contains(&"chat".to_string()));
        assert!(labels.contains(&"multimodal".to_string()));
        assert_eq!(
            normalize_capability_token("  Image Generation "),
            "image_generation"
        );
    }

    #[test]
    fn lemonade_model_parser_accepts_compatibility_aliases_and_defaults() {
        let model = parse_lemonade_model(
            &json!({
                "id": "editor",
                "is_downloaded": false,
                "supports": {"image-editing": true, "completions": true},
                "metadata": {"recipe": "recipe-name"},
                "size_bytes": 1_073_741_824u64,
                "parameters": "4B"
            }),
            LocalImageProvider::Lemonade,
        )
        .expect("model id");
        assert!(!model.downloaded);
        assert_eq!(model.recipe.as_deref(), Some("recipe-name"));
        assert_eq!(model.size_gb, Some(1.0));
        assert_eq!(model.parameter_size.as_deref(), Some("4B"));
        assert!(model.capabilities.edit);
        assert!(model.capabilities.chat);
        assert!(
            parse_lemonade_model(&json!({"downloaded": true}), LocalImageProvider::Lemonade)
                .is_none()
        );
    }

    #[test]
    fn loopback_validation_rejects_credentials_missing_hosts_and_non_http_urls() {
        for url in [
            "http://user@localhost:11434",
            "http://localhost:pw@11434",
            "http:///api/v1",
            "file:///tmp/model",
            "http://[2001:db8::1]:11434",
        ] {
            assert!(validate_loopback_endpoint(url).is_err(), "accepted {url}");
        }
    }

    #[test]
    fn assistant_content_and_ndjson_parser_handle_empty_or_malformed_values() {
        assert_eq!(parse_assistant_content(&json!([])).as_deref(), Some(""));
        assert_eq!(
            parse_assistant_content(&json!([{"content":"part"}])).as_deref(),
            Some("part")
        );
        assert_eq!(parse_assistant_content(&json!(42)), None);
        assert!(parse_json_or_last_ndjson(b"\n  \n").is_err());
        assert!(parse_json_or_last_ndjson(b"not-json\n").is_err());
    }
}
