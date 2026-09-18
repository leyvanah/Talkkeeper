// gigaam_engine/engine.rs
//
// Model lifecycle for the native GigaAM engine: where the files live, whether
// they are complete, downloading them with resume support, and loading or
// unloading the ONNX sessions.
//
// GigaAM ships as a single model here (v3 end-to-end RNN-T, int8), so this is
// deliberately simpler than the Parakeet engine's multi-model catalogue.

use anyhow::{anyhow, Result};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::{Duration, Instant};
use tokio::fs;
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::sync::RwLock;
use tokio::time::timeout;

use super::model::GigaamModel;

/// The one model this engine serves.
pub const MODEL_NAME: &str = "gigaam-v3-e2e-rnnt-int8";
pub const DOWNLOAD_CANCELLED_MESSAGE: &str = "Download cancelled by user";

/// A pinned commit of istupakov/gigaam-v3-onnx; see `crate::model_integrity`.
const BASE_URL: &str = crate::model_integrity::GIGAAM_BASE;

/// Exact published sizes - a short answer or a truncated transfer is caught
/// before the model is ever loaded.
pub const MODEL_FILES: [(&str, u64); 4] = [
    ("v3_e2e_rnnt_encoder.int8.onnx", 224_570_477),
    ("v3_e2e_rnnt_decoder.int8.onnx", 1_159_170),
    ("v3_e2e_rnnt_joint.int8.onnx", 687_791),
    ("v3_e2e_rnnt_vocab.txt", 13_354),
];

pub fn total_download_bytes() -> u64 {
    MODEL_FILES.iter().map(|(_, size)| size).sum()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgress {
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub downloaded_mb: f64,
    pub total_mb: f64,
    pub speed_mbps: f64,
    pub percent: u8,
}

impl DownloadProgress {
    fn new(downloaded: u64, total: u64, speed_mbps: f64) -> Self {
        Self {
            downloaded_bytes: downloaded,
            total_bytes: total,
            downloaded_mb: downloaded as f64 / 1_048_576.0,
            total_mb: total as f64 / 1_048_576.0,
            speed_mbps,
            percent: if total > 0 {
                ((downloaded as f64 / total as f64) * 100.0).min(100.0) as u8
            } else {
                0
            },
        }
    }
}

/// What the settings screen needs to know about the model on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GigaamModelStatus {
    pub name: String,
    /// All files present with the expected sizes.
    pub installed: bool,
    /// Some files are present but incomplete - a download can resume.
    pub partial: bool,
    pub downloading: bool,
    pub progress: u8,
    pub loaded: bool,
    pub size_mb: u32,
    pub path: String,
}

pub struct GigaamEngine {
    model_dir: PathBuf,
    model: RwLock<Option<GigaamModel>>,
    downloading: AtomicBool,
    cancel_download: AtomicBool,
    progress: AtomicU8,
}

impl GigaamEngine {
    pub fn new_with_models_dir(models_dir: Option<PathBuf>) -> Result<Self> {
        let model_dir = models_dir
            .unwrap_or_else(crate::paths::models_dir)
            .join("gigaam");

        if !model_dir.exists() {
            std::fs::create_dir_all(&model_dir)?;
        }
        log::info!("GigaAM model directory: {}", model_dir.display());

        Ok(Self {
            model_dir,
            model: RwLock::new(None),
            downloading: AtomicBool::new(false),
            cancel_download: AtomicBool::new(false),
            progress: AtomicU8::new(0),
        })
    }

    pub fn model_dir(&self) -> &PathBuf {
        &self.model_dir
    }

    /// Bytes already on disk, counting only files no larger than expected.
    async fn downloaded_bytes(&self) -> (u64, bool, bool) {
        let mut downloaded = 0u64;
        let mut complete = true;
        let mut any = false;

        for (name, expected) in MODEL_FILES {
            match fs::metadata(self.model_dir.join(name)).await {
                Ok(metadata) => {
                    any = true;
                    let size = metadata.len();
                    if size == expected {
                        downloaded += expected;
                    } else {
                        complete = false;
                        if size < expected {
                            downloaded += size;
                        }
                    }
                }
                Err(_) => complete = false,
            }
        }

        (downloaded, complete, any)
    }

    pub async fn status(&self) -> GigaamModelStatus {
        let (downloaded, complete, any) = self.downloaded_bytes().await;
        let downloading = self.downloading.load(Ordering::Relaxed);
        let total = total_download_bytes();

        GigaamModelStatus {
            name: MODEL_NAME.to_string(),
            installed: complete,
            partial: any && !complete,
            downloading,
            progress: if downloading {
                self.progress.load(Ordering::Relaxed)
            } else if total > 0 {
                ((downloaded as f64 / total as f64) * 100.0) as u8
            } else {
                0
            },
            loaded: self.model.read().await.is_some(),
            size_mb: (total / 1_048_576) as u32,
            path: self.model_dir.display().to_string(),
        }
    }

    pub async fn is_model_loaded(&self) -> bool {
        self.model.read().await.is_some()
    }

    pub async fn get_current_model(&self) -> Option<String> {
        self.model
            .read()
            .await
            .as_ref()
            .map(|_| MODEL_NAME.to_string())
    }

    pub async fn load_model(&self) -> Result<()> {
        if self.is_model_loaded().await {
            return Ok(());
        }

        let (_, complete, _) = self.downloaded_bytes().await;
        if !complete {
            return Err(anyhow!(
                "GigaAM model is not downloaded yet. Download it in the settings first."
            ));
        }

        log::info!("Loading GigaAM model from {}", self.model_dir.display());
        let model_dir = self.model_dir.clone();
        // Session creation reads ~226 MB from disk; keep it off the async runtime
        let model = tokio::task::spawn_blocking(move || GigaamModel::new(&model_dir))
            .await
            .map_err(|e| anyhow!("GigaAM model loading task failed: {}", e))?
            .map_err(|e| anyhow!("Failed to load GigaAM model: {}", e))?;

        *self.model.write().await = Some(model);
        log::info!("GigaAM model loaded");
        Ok(())
    }

    pub async fn unload_model(&self) -> bool {
        let unloaded = self.model.write().await.take().is_some();
        if unloaded {
            log::info!("GigaAM model unloaded");
        }
        unloaded
    }

    /// Transcribe 16 kHz mono samples with the loaded model.
    pub async fn transcribe_audio(&self, audio_data: Vec<f32>) -> Result<String> {
        Ok(self.transcribe_audio_with_words(audio_data).await?.0)
    }

    /// As [`Self::transcribe_audio`], plus when each word was said, in seconds
    /// from the start of `audio_data`. The transducer places every token on an
    /// encoder frame, so the times are good to 40 ms. `None` when there are no
    /// words or they do not spell the text.
    pub async fn transcribe_audio_with_words(
        &self,
        audio_data: Vec<f32>,
    ) -> Result<(String, Option<Vec<crate::audio::word_timing::WordTiming>>)> {
        let mut guard = self.model.write().await;
        let model = guard
            .as_mut()
            .ok_or_else(|| anyhow!("No GigaAM model loaded. Please load the model first."))?;

        let result = model
            .transcribe_samples(&audio_data)
            .map_err(|e| anyhow!("GigaAM transcription failed: {}", e))?;

        let words = result.words();
        let words = (!words.is_empty() && crate::audio::word_timing::spell(&words, &result.text))
            .then_some(words);
        Ok((result.text, words))
    }

    pub fn cancel_download(&self) {
        self.cancel_download.store(true, Ordering::Relaxed);
    }

    pub async fn delete_model(&self) -> Result<()> {
        self.unload_model().await;
        for (name, _) in MODEL_FILES {
            let path = self.model_dir.join(name);
            if path.exists() {
                fs::remove_file(&path)
                    .await
                    .map_err(|e| anyhow!("Failed to delete {}: {}", name, e))?;
            }
        }
        log::info!("GigaAM model files deleted");
        Ok(())
    }

    /// Download the model, resuming any partial files, reporting progress.
    pub async fn download_model(
        &self,
        progress_callback: Option<Box<dyn Fn(DownloadProgress) + Send + Sync>>,
    ) -> Result<()> {
        if self
            .downloading
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(anyhow!("A GigaAM download is already running"));
        }
        self.cancel_download.store(false, Ordering::Relaxed);

        let result = self.download_files(progress_callback).await;

        self.downloading.store(false, Ordering::SeqCst);
        self.progress.store(0, Ordering::Relaxed);
        result
    }

    async fn download_files(
        &self,
        progress_callback: Option<Box<dyn Fn(DownloadProgress) + Send + Sync>>,
    ) -> Result<()> {
        if !self.model_dir.exists() {
            fs::create_dir_all(&self.model_dir).await?;
        }

        let client = reqwest::Client::builder()
            .tcp_nodelay(true)
            .pool_max_idle_per_host(1)
            .timeout(Duration::from_secs(3600))
            .connect_timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| anyhow!("Failed to create HTTP client: {}", e))?;

        let total_bytes = total_download_bytes();
        let (mut downloaded_total, _, _) = self.downloaded_bytes().await;
        let started = Instant::now();
        let mut last_report = Instant::now();

        let report = |downloaded: u64, elapsed: f64| {
            if let Some(callback) = progress_callback.as_ref() {
                let speed = if elapsed > 0.0 {
                    (downloaded as f64 / 1_048_576.0) / elapsed
                } else {
                    0.0
                };
                callback(DownloadProgress::new(downloaded, total_bytes, speed));
            }
        };

        for (name, expected) in MODEL_FILES {
            let path = self.model_dir.join(name);
            let mut existing = fs::metadata(&path).await.map(|m| m.len()).unwrap_or(0);

            if existing == expected {
                continue;
            }
            if existing > expected {
                fs::remove_file(&path)
                    .await
                    .map_err(|e| anyhow!("Failed to remove oversized {}: {}", name, e))?;
                existing = 0;
            }
            if self.cancel_download.load(Ordering::Relaxed) {
                return Err(anyhow!(DOWNLOAD_CANCELLED_MESSAGE));
            }

            log::info!(
                "Downloading GigaAM file {} (resuming from {} of {} bytes)",
                name,
                existing,
                expected
            );

            let mut request = client.get(format!("{}/{}", BASE_URL, name));
            if existing > 0 {
                request = request.header("Range", format!("bytes={}-", existing));
            }

            let response = timeout(Duration::from_secs(30), request.send())
                .await
                .map_err(|_| anyhow!("{} timed out waiting for the server", name))?
                .map_err(|e| anyhow!("Failed to request {}: {}", name, e))?;

            if !response.status().is_success() {
                return Err(anyhow!(
                    "Download of {} failed: server answered {}",
                    name,
                    response.status()
                ));
            }

            // A server that ignores Range answers 200 with the whole file
            let resuming = response.status() == reqwest::StatusCode::PARTIAL_CONTENT;
            let announced = response.content_length().unwrap_or(0);
            let effective_start = if resuming { existing } else { 0 };
            if announced > 0 && effective_start + announced != expected {
                return Err(anyhow!(
                    "Download of {} failed: server reports {} bytes, expected {}",
                    name,
                    effective_start + announced,
                    expected
                ));
            }

            let file = if resuming {
                fs::OpenOptions::new()
                    .append(true)
                    .open(&path)
                    .await
                    .map_err(|e| anyhow!("Failed to open {} for resume: {}", name, e))?
            } else {
                downloaded_total = downloaded_total.saturating_sub(existing);
                fs::File::create(&path)
                    .await
                    .map_err(|e| anyhow!("Failed to create {}: {}", name, e))?
            };

            let mut writer = BufWriter::with_capacity(8 * 1024 * 1024, file);
            let mut stream = response.bytes_stream();

            loop {
                if self.cancel_download.load(Ordering::Relaxed) {
                    let _ = writer.flush().await;
                    return Err(anyhow!(DOWNLOAD_CANCELLED_MESSAGE));
                }

                let chunk = match timeout(Duration::from_secs(30), stream.next()).await {
                    Err(_) => return Err(anyhow!("{}: no data received for 30 seconds", name)),
                    Ok(None) => break,
                    Ok(Some(Err(error))) => {
                        return Err(anyhow!("{} transfer failed: {}", name, error))
                    }
                    Ok(Some(Ok(chunk))) => chunk,
                };

                writer
                    .write_all(&chunk)
                    .await
                    .map_err(|e| anyhow!("Failed to write {}: {}", name, e))?;
                downloaded_total += chunk.len() as u64;

                if last_report.elapsed() >= Duration::from_millis(500) {
                    last_report = Instant::now();
                    let percent = if total_bytes > 0 {
                        ((downloaded_total as f64 / total_bytes as f64) * 100.0).min(99.0) as u8
                    } else {
                        0
                    };
                    self.progress.store(percent, Ordering::Relaxed);
                    report(downloaded_total, started.elapsed().as_secs_f64());
                }
            }

            writer
                .flush()
                .await
                .map_err(|e| anyhow!("Failed to flush {}: {}", name, e))?;

            let final_size = fs::metadata(&path).await.map(|m| m.len()).unwrap_or(0);
            if final_size != expected {
                return Err(anyhow!(
                    "Download of {} is incomplete: {} of {} bytes",
                    name,
                    final_size,
                    expected
                ));
            }
        }

        // Complete by size is not the same as the right file. Resumed files
        // only streamed their tail, so each is read once here.
        for (name, _) in MODEL_FILES {
            let pinned = crate::model_integrity::find(crate::model_integrity::GIGAAM, name)
                .ok_or_else(|| anyhow!("{} has no pinned checksum", name))?;
            crate::model_integrity::verify_file(pinned, &self.model_dir.join(name)).await?;
        }

        self.progress.store(100, Ordering::Relaxed);
        report(total_bytes, started.elapsed().as_secs_f64());
        log::info!("GigaAM model download complete");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn published_sizes_add_up_to_the_advertised_download() {
        assert_eq!(total_download_bytes(), 226_430_792);
        assert_eq!((total_download_bytes() / 1_048_576) as u32, 215);
    }

    #[tokio::test]
    async fn an_empty_directory_reports_nothing_installed() {
        let temp = tempfile::tempdir().expect("temp dir");
        let engine =
            GigaamEngine::new_with_models_dir(Some(temp.path().to_path_buf())).expect("engine");

        let status = engine.status().await;
        assert!(!status.installed);
        assert!(!status.partial);
        assert!(!status.loaded);
        assert_eq!(status.progress, 0);
    }

    /// Checks the real endpoint: seeds a directory from an existing copy of the
    /// model, truncates one file and lets the downloader finish it with a range
    /// request. Opt-in because it needs the model on disk and network access:
    ///   GIGAAM_MODEL_DIR=<dir> cargo test --lib gigaam -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "needs GIGAAM_MODEL_DIR and network access"]
    async fn a_partial_file_is_resumed_from_the_published_source() {
        let source = std::path::PathBuf::from(
            std::env::var("GIGAAM_MODEL_DIR").expect("GIGAAM_MODEL_DIR"),
        );
        let temp = tempfile::tempdir().expect("temp dir");
        let engine =
            GigaamEngine::new_with_models_dir(Some(temp.path().to_path_buf())).expect("engine");

        for (name, _) in MODEL_FILES {
            std::fs::copy(source.join(name), engine.model_dir().join(name)).expect("seed file");
        }

        // Cut the vocabulary short so only those few kilobytes are fetched again
        let vocab = engine.model_dir().join(MODEL_FILES[3].0);
        let content = std::fs::read(&vocab).expect("read vocab");
        std::fs::write(&vocab, &content[..5000]).expect("truncate vocab");

        assert!(engine.status().await.partial);
        engine.download_model(None).await.expect("resume download");

        let status = engine.status().await;
        assert!(status.installed, "download did not complete: {:?}", status);
        assert_eq!(std::fs::read(&vocab).expect("read vocab"), content);
    }

    /// The path the app actually takes: point the engine at a model directory,
    /// load it and transcribe. Opt-in, needs the model and a 16 kHz wav:
    ///   GIGAAM_MODEL_DIR=<dir> GIGAAM_TEST_WAV=<file.wav> \
    ///     cargo test --lib gigaam -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "needs GIGAAM_MODEL_DIR and GIGAAM_TEST_WAV"]
    async fn the_engine_loads_the_model_and_transcribes() {
        let source = std::path::PathBuf::from(
            std::env::var("GIGAAM_MODEL_DIR").expect("GIGAAM_MODEL_DIR"),
        );
        let wav = std::env::var("GIGAAM_TEST_WAV").expect("GIGAAM_TEST_WAV");

        // The engine appends "gigaam" to the models directory, so seed it there
        let temp = tempfile::tempdir().expect("temp dir");
        let engine =
            GigaamEngine::new_with_models_dir(Some(temp.path().to_path_buf())).expect("engine");
        for (name, _) in MODEL_FILES {
            std::fs::copy(source.join(name), engine.model_dir().join(name)).expect("seed file");
        }

        assert!(engine.status().await.installed);
        engine.load_model().await.expect("load model");
        assert!(engine.is_model_loaded().await);
        assert_eq!(engine.get_current_model().await.as_deref(), Some(MODEL_NAME));

        let (samples, sample_rate) =
            crate::diarization::dsp::read_wav(std::path::Path::new(&wav)).expect("read wav");
        assert_eq!(sample_rate, 16_000, "the test wav must be 16 kHz mono");

        let text = engine.transcribe_audio(samples).await.expect("transcribe");
        println!("engine text: {}", text);
        assert!(!text.trim().is_empty(), "expected some text");

        assert!(engine.unload_model().await);
        assert!(!engine.is_model_loaded().await);
    }

    #[tokio::test]
    async fn a_truncated_file_is_reported_as_partial() {
        let temp = tempfile::tempdir().expect("temp dir");
        let engine =
            GigaamEngine::new_with_models_dir(Some(temp.path().to_path_buf())).expect("engine");
        std::fs::write(engine.model_dir().join(MODEL_FILES[3].0), b"short").expect("write");

        let status = engine.status().await;
        assert!(!status.installed);
        assert!(status.partial);
        assert!(engine.load_model().await.is_err());
    }
}
