// audio/transcription/external_stt.rs
//
// External HTTP speech-to-text provider.
//
// Talks to a locally running STT service (GigaAM, whisper.cpp server, any
// OpenAI-compatible `/v1/audio/transcriptions` endpoint) over HTTP. Audio is
// sent as a 16 kHz mono PCM16 WAV; the answer is plain text or JSON, and the
// field holding the text is configured with a dot path so unusual services can
// be adapted without code changes.

use super::provider::{TranscriptionError, TranscriptionProvider, TranscriptResult};
use async_trait::async_trait;
use log::{info, warn};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

/// Minimum audio length worth sending over the network (0.2 s at 16 kHz).
const MIN_SAMPLES: usize = 3200;

/// How the audio is put into the request body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExternalSttMode {
    /// `multipart/form-data` with a file part (OpenAI-compatible services).
    Multipart,
    /// The raw WAV bytes as the request body (`Content-Type: audio/wav`).
    Raw,
}

impl Default for ExternalSttMode {
    fn default() -> Self {
        Self::Multipart
    }
}

fn default_file_field() -> String {
    "file".to_string()
}

fn default_language_field() -> String {
    "language".to_string()
}

fn default_response_path() -> String {
    "text".to_string()
}

fn default_timeout_ms() -> u64 {
    30_000
}

fn default_max_retries() -> u32 {
    2
}

fn default_send_language() -> bool {
    true
}

/// Everything the provider needs to reach the service. Persisted as JSON in
/// `transcript_settings.externalSttConfig`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalSttConfig {
    /// Full endpoint URL, e.g. `http://127.0.0.1:8080/v1/audio/transcriptions`.
    pub url: String,
    #[serde(default)]
    pub mode: ExternalSttMode,
    /// Multipart field name carrying the WAV file.
    #[serde(default = "default_file_field")]
    pub file_field: String,
    /// Optional `model` form field expected by the service.
    #[serde(default)]
    pub model: Option<String>,
    /// Multipart field name carrying the language hint.
    #[serde(default = "default_language_field")]
    pub language_field: String,
    #[serde(default = "default_send_language")]
    pub send_language: bool,
    /// Dot path to the text inside a JSON answer, e.g. `text` or `result.0.text`.
    #[serde(default = "default_response_path")]
    pub response_path: String,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    /// Retries after a network error / timeout / 5xx, with exponential backoff.
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    /// Optional bearer token for services that require one.
    #[serde(default)]
    pub auth_token: Option<String>,
}

impl Default for ExternalSttConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            mode: ExternalSttMode::default(),
            file_field: default_file_field(),
            model: None,
            language_field: default_language_field(),
            send_language: default_send_language(),
            response_path: default_response_path(),
            timeout_ms: default_timeout_ms(),
            max_retries: default_max_retries(),
            auth_token: None,
        }
    }
}

impl ExternalSttConfig {
    /// Reject configurations that cannot possibly work before we hit the network.
    pub fn validate(&self) -> Result<(), String> {
        let url = self.url.trim();
        if url.is_empty() {
            return Err("Endpoint URL is not set".to_string());
        }
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err("Endpoint URL must start with http:// or https://".to_string());
        }
        if self.timeout_ms < 1000 {
            return Err("Timeout must be at least 1000 ms".to_string());
        }
        Ok(())
    }

    /// Host:port shown in the UI as the "model" of this provider.
    pub fn display_name(&self) -> String {
        self.url
            .trim()
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .split('/')
            .next()
            .unwrap_or("")
            .to_string()
    }
}

// ============================================================================
// WAV ENCODING
// ============================================================================

/// Encode f32 samples as a 16-bit PCM mono WAV file in memory.
pub fn encode_wav_pcm16(samples: &[f32], sample_rate: u32) -> Vec<u8> {
    let data_len = samples.len() * 2;
    let mut out = Vec::with_capacity(44 + data_len);

    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVE");

    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // PCM header size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM format
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample

    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_len as u32).to_le_bytes());
    for sample in samples {
        let clamped = sample.clamp(-1.0, 1.0);
        let value = (clamped * i16::MAX as f32).round() as i16;
        out.extend_from_slice(&value.to_le_bytes());
    }

    out
}

// ============================================================================
// RESPONSE PARSING
// ============================================================================

/// Follow a dot path (`result.0.text`) through a JSON value.
fn value_at_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let path = path.trim();
    if path.is_empty() {
        return Some(value);
    }

    let mut current = value;
    for segment in path.split('.') {
        current = match current {
            Value::Object(map) => map.get(segment)?,
            Value::Array(items) => items.get(segment.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(current)
}

/// Extract the transcript out of whatever the service answered.
pub fn extract_text(body: &str, response_path: &str) -> Result<String, String> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Ok(String::new());
    }

    // Plain-text services: no JSON to parse, the body is the transcript.
    let parsed: Value = match serde_json::from_str(trimmed) {
        Ok(value) => value,
        Err(_) => return Ok(trimmed.to_string()),
    };

    let found = value_at_path(&parsed, response_path).ok_or_else(|| {
        format!(
            "Response has no field '{}' (got: {})",
            response_path,
            snippet(trimmed)
        )
    })?;

    match found {
        Value::String(text) => Ok(text.clone()),
        Value::Null => Ok(String::new()),
        // Some services answer with an array of segments at the configured path.
        Value::Array(items) => Ok(items
            .iter()
            .filter_map(|item| item.as_str())
            .collect::<Vec<_>>()
            .join(" ")),
        other => Ok(other.to_string()),
    }
}

/// Shorten a body for error messages so logs stay readable.
fn snippet(body: &str) -> String {
    let cleaned = body.replace(['\n', '\r'], " ");
    if cleaned.chars().count() > 200 {
        let short: String = cleaned.chars().take(200).collect();
        format!("{}...", short)
    } else {
        cleaned
    }
}

// ============================================================================
// PROVIDER
// ============================================================================

pub struct ExternalSttProvider {
    config: ExternalSttConfig,
    client: reqwest::Client,
}

impl ExternalSttProvider {
    pub fn new(config: ExternalSttConfig) -> Result<Self, String> {
        config.validate()?;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(config.timeout_ms))
            .build()
            .map_err(|e| format!("Failed to create HTTP client: {}", e))?;
        Ok(Self { config, client })
    }

    pub fn config(&self) -> &ExternalSttConfig {
        &self.config
    }

    /// One request. The bool in the error marks a failure worth retrying.
    async fn request_once(
        &self,
        wav: &[u8],
        language: Option<&str>,
    ) -> Result<String, (String, bool)> {
        // Not retried: the setting will not change between attempts.
        crate::network_policy::check(self.config.url.trim()).map_err(|error| (error, false))?;
        let mut request = self.client.post(self.config.url.trim());

        if let Some(token) = self
            .config
            .auth_token
            .as_deref()
            .map(str::trim)
            .filter(|token| !token.is_empty())
        {
            request = request.bearer_auth(token);
        }

        request = match self.config.mode {
            ExternalSttMode::Raw => request
                .header(reqwest::header::CONTENT_TYPE, "audio/wav")
                .body(wav.to_vec()),
            ExternalSttMode::Multipart => {
                let part = reqwest::multipart::Part::bytes(wav.to_vec())
                    .file_name("audio.wav")
                    .mime_str("audio/wav")
                    .map_err(|e| (format!("Failed to build request body: {}", e), false))?;
                let mut form =
                    reqwest::multipart::Form::new().part(self.config.file_field.clone(), part);
                if let Some(model) = self
                    .config
                    .model
                    .as_deref()
                    .map(str::trim)
                    .filter(|model| !model.is_empty())
                {
                    form = form.text("model", model.to_string());
                }
                if self.config.send_language {
                    if let Some(language) = language.filter(|value| !value.is_empty()) {
                        form = form
                            .text(self.config.language_field.clone(), language.to_string());
                    }
                }
                request.multipart(form)
            }
        };

        let response = request
            .send()
            .await
            .map_err(|e| (describe_request_error(&e), is_retryable(&e)))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| (format!("Failed to read the answer: {}", e), true))?;

        if !status.is_success() {
            let retryable = status.is_server_error();
            return Err((
                format!("Service answered {}: {}", status.as_u16(), snippet(&body)),
                retryable,
            ));
        }

        extract_text(&body, &self.config.response_path).map_err(|e| (e, false))
    }

    /// Send the audio, retrying transient failures with exponential backoff.
    pub async fn transcribe_wav(
        &self,
        wav: Vec<u8>,
        language: Option<&str>,
    ) -> Result<String, String> {
        // "auto" is our own marker for "let the engine decide" - never send it on
        let language = language
            .map(str::trim)
            .filter(|value| !value.is_empty() && *value != "auto" && *value != "auto-translate");
        let mut attempt = 0;
        let mut backoff = Duration::from_millis(400);

        loop {
            match self.request_once(&wav, language).await {
                Ok(text) => return Ok(text),
                Err((message, retryable)) => {
                    if !retryable || attempt >= self.config.max_retries {
                        return Err(message);
                    }
                    attempt += 1;
                    warn!(
                        "External STT request failed ({}), retry {}/{} in {} ms",
                        message,
                        attempt,
                        self.config.max_retries,
                        backoff.as_millis()
                    );
                    tokio::time::sleep(backoff).await;
                    backoff *= 3;
                }
            }
        }
    }
}

/// Network hiccups and timeouts are worth another attempt; bad requests are not.
fn is_retryable(error: &reqwest::Error) -> bool {
    error.is_timeout() || error.is_connect() || error.is_request()
}

/// Turn a reqwest failure into something the owner can act on.
fn describe_request_error(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        "The speech service did not answer in time".to_string()
    } else if error.is_connect() {
        "Cannot reach the speech service - is it running?".to_string()
    } else {
        format!("Request to the speech service failed: {}", error)
    }
}

#[async_trait]
impl TranscriptionProvider for ExternalSttProvider {
    async fn transcribe(
        &self,
        audio: Vec<f32>,
        language: Option<String>,
    ) -> std::result::Result<TranscriptResult, TranscriptionError> {
        if audio.len() < MIN_SAMPLES {
            return Err(TranscriptionError::AudioTooShort {
                samples: audio.len(),
                minimum: MIN_SAMPLES,
            });
        }

        // The worker hands us 16 kHz mono audio already.
        let wav = encode_wav_pcm16(&audio, 16_000);

        match self.transcribe_wav(wav, language.as_deref()).await {
            Ok(text) => Ok(TranscriptResult {
                text: text.trim().to_string(),
                confidence: None, // Services rarely report one, and the format varies
                is_partial: false,
            }),
            Err(message) => Err(TranscriptionError::EngineFailed(message)),
        }
    }

    async fn is_model_loaded(&self) -> bool {
        // The model lives in the external service; a valid endpoint is all we need.
        self.config.validate().is_ok()
    }

    async fn get_current_model(&self) -> Option<String> {
        self.config
            .model
            .clone()
            .filter(|model| !model.trim().is_empty())
            .or_else(|| Some(self.config.display_name()))
    }

    fn provider_name(&self) -> &'static str {
        "External STT"
    }
}

// ============================================================================
// CONNECTION TEST
// ============================================================================

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalSttTestResult {
    pub ok: bool,
    pub latency_ms: u64,
    /// Text the service returned for the probe (usually empty for silence).
    pub text: String,
}

/// Send one second of silence and report whether the service answered.
pub async fn test_connection(config: ExternalSttConfig) -> Result<ExternalSttTestResult, String> {
    let provider = ExternalSttProvider::new(config)?;
    let wav = encode_wav_pcm16(&vec![0.0f32; 16_000], 16_000);

    let started = std::time::Instant::now();
    let text = provider.transcribe_wav(wav, None).await?;
    let latency_ms = started.elapsed().as_millis() as u64;

    info!("External STT connection test succeeded in {} ms", latency_ms);

    Ok(ExternalSttTestResult {
        ok: true,
        latency_ms,
        text: text.trim().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_header_describes_the_samples() {
        let wav = encode_wav_pcm16(&[0.0, 1.0, -1.0], 16_000);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(wav.len(), 44 + 6);
        // Third sample is full-scale negative.
        assert_eq!(i16::from_le_bytes([wav[48], wav[49]]), -i16::MAX);
    }

    #[test]
    fn text_is_read_from_the_configured_path() {
        assert_eq!(
            extract_text(r#"{"text":"привет"}"#, "text").unwrap(),
            "привет"
        );
        assert_eq!(
            extract_text(r#"{"result":[{"text":"привет"}]}"#, "result.0.text").unwrap(),
            "привет"
        );
        assert_eq!(extract_text("plain answer", "text").unwrap(), "plain answer");
        assert!(extract_text(r#"{"other":1}"#, "text").is_err());
    }

    #[test]
    fn segment_arrays_are_joined() {
        assert_eq!(
            extract_text(r#"{"segments":["раз","два"]}"#, "segments").unwrap(),
            "раз два"
        );
    }

    /// Minimal HTTP/1.1 server that answers each request from `answers` in
    /// order, so the request path can be exercised without a real service.
    async fn spawn_stub(
        answers: Vec<(u16, &'static str)>,
    ) -> (String, tokio::task::JoinHandle<Vec<u8>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/asr", listener.local_addr().unwrap());

        let handle = tokio::spawn(async move {
            let mut first_request = Vec::new();
            for (index, (status, body)) in answers.into_iter().enumerate() {
                let (mut socket, _) = listener.accept().await.unwrap();

                let mut request = Vec::new();
                let mut buffer = [0u8; 4096];
                loop {
                    let read = socket.read(&mut buffer).await.unwrap();
                    request.extend_from_slice(&buffer[..read]);
                    if read == 0 || request_is_complete(&request) {
                        break;
                    }
                }
                if index == 0 {
                    first_request = request;
                }

                let response = format!(
                    "HTTP/1.1 {} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    status,
                    body.len(),
                    body
                );
                socket.write_all(response.as_bytes()).await.unwrap();
                let _ = socket.shutdown().await;
            }
            first_request
        });

        (url, handle)
    }

    /// Headers received and the body is as long as Content-Length promised.
    fn request_is_complete(request: &[u8]) -> bool {
        let text = String::from_utf8_lossy(request);
        let Some(header_end) = text.find("\r\n\r\n") else {
            return false;
        };
        let content_length = text[..header_end]
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())?
            })
            .unwrap_or(0);
        request.len() >= header_end + 4 + content_length
    }

    #[tokio::test]
    async fn audio_is_posted_and_the_answer_is_read_back() {
        let (url, served) = spawn_stub(vec![(200, r#"{"text":"это тест"}"#)]).await;

        let config = ExternalSttConfig {
            url,
            ..ExternalSttConfig::default()
        };
        let provider = ExternalSttProvider::new(config).unwrap();
        let result = provider
            .transcribe(vec![0.1f32; 16_000], Some("ru".to_string()))
            .await
            .unwrap();

        assert_eq!(result.text, "это тест");
        let request = served.await.unwrap();
        let request = String::from_utf8_lossy(&request);
        assert!(request.starts_with("POST /asr "));
        assert!(request.contains("multipart/form-data"));
        assert!(request.contains("name=\"file\""));
        assert!(request.contains("name=\"language\""));
        assert!(request.contains("RIFF"));
    }

    #[tokio::test]
    async fn a_failing_service_is_retried_and_then_succeeds() {
        let (url, _served) =
            spawn_stub(vec![(503, "busy"), (200, r#"{"text":"после сбоя"}"#)]).await;

        let config = ExternalSttConfig {
            url,
            max_retries: 2,
            ..ExternalSttConfig::default()
        };
        let provider = ExternalSttProvider::new(config).unwrap();
        let text = provider
            .transcribe_wav(encode_wav_pcm16(&[0.1f32; 16_000], 16_000), None)
            .await
            .unwrap();

        assert_eq!(text, "после сбоя");
    }

    #[tokio::test]
    async fn a_client_error_is_reported_without_retrying() {
        let (url, _served) = spawn_stub(vec![(400, "bad request")]).await;

        let config = ExternalSttConfig {
            url,
            ..ExternalSttConfig::default()
        };
        let provider = ExternalSttProvider::new(config).unwrap();
        let error = provider
            .transcribe_wav(encode_wav_pcm16(&[0.1f32; 16_000], 16_000), None)
            .await
            .unwrap_err();

        assert!(error.contains("400"), "unexpected message: {}", error);
    }

    #[test]
    fn empty_and_non_http_urls_are_rejected() {
        let mut config = ExternalSttConfig::default();
        assert!(config.validate().is_err());
        config.url = "127.0.0.1:8080".to_string();
        assert!(config.validate().is_err());
        config.url = "http://127.0.0.1:8080/asr".to_string();
        assert!(config.validate().is_ok());
        assert_eq!(config.display_name(), "127.0.0.1:8080");
    }
}
