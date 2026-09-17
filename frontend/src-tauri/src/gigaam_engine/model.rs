// gigaam_engine/model.rs
//
// GigaAM v3 RNN-T inference: three ONNX Runtime sessions (encoder, prediction
// network, joint) driven by a greedy transducer loop.
//
// The graphs come from istupakov/gigaam-v3-onnx (MIT), the same publisher as
// the Parakeet models this app already uses. Their shapes are fixed:
//   encoder: audio_signal [1,64,T] f32, length [1] i64 -> encoded [1,768,T'],
//            encoded_len [1] i32
//   decoder: x [1,1] i64, h.1/c.1 [1,1,320] f32 -> dec, h, c
//   joint:   enc [1,768,1], dec [1,320,1] -> joint [1,1,1,1025]

use ndarray::{Array1, Array2, Array3, ArrayD, Axis};
use once_cell::sync::Lazy;
use ort::execution_providers::CPUExecutionProvider;
use ort::inputs;
use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;
use ort::value::TensorRef;
use regex::Regex;
use std::fs;
use std::path::Path;

use super::features::{FeatureExtractor, N_MELS};
use crate::audio::word_timing::{words_from_pieces, TimedPiece, WordTiming};

/// Frames the encoder collapses into one output step.
const SUBSAMPLING_FACTOR: usize = 4;
/// Feature frame length in seconds, for timestamps.
const WINDOW_STEP: f32 = 0.01;
/// Emissions allowed on a single encoder frame before moving on.
const MAX_TOKENS_PER_STEP: usize = 3;
const PRED_HIDDEN: usize = 320;
const ENCODER_DIM: usize = 768;
/// Fewer feature frames than this and the encoder has nothing to subsample.
const MIN_FRAMES: usize = 16;

static DECODE_SPACE_RE: Lazy<Result<Regex, regex::Error>> =
    Lazy::new(|| Regex::new(r"\A\s|\s\B|(\s)\b"));

#[derive(thiserror::Error, Debug)]
pub enum GigaamError {
    #[error("ONNX Runtime error: {0}")]
    Ort(#[from] ort::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("ndarray shape error: {0}")]
    Shape(#[from] ndarray::ShapeError),
    #[error("Model output not found: {0}")]
    OutputNotFound(String),
    #[error("Model file not found: {0}")]
    ModelFileNotFound(String),
    #[error("Vocabulary is missing the <blk> token")]
    MissingBlank,
}

#[derive(Debug, Clone)]
pub struct TimestampedResult {
    pub text: String,
    pub timestamps: Vec<f32>,
    pub tokens: Vec<String>,
}

impl TimestampedResult {
    /// The tokens joined into words, each with when it was said.
    ///
    /// A token carries only the frame it was emitted on, so it is taken to
    /// last until the next token begins; the last one gets one encoder frame.
    pub fn words(&self) -> Vec<WordTiming> {
        let frame = (WINDOW_STEP * SUBSAMPLING_FACTOR as f32) as f64;
        let pieces = self.tokens.iter().enumerate().map(|(index, token)| {
            let start = self.timestamps.get(index).copied().unwrap_or(0.0) as f64;
            let end = self
                .timestamps
                .get(index + 1)
                .map(|&next| next as f64)
                .unwrap_or(start + frame);
            TimedPiece {
                bytes: token.as_bytes().to_vec(),
                start,
                end,
            }
        });
        words_from_pieces(pieces)
    }
}

/// Names of the files a GigaAM model directory must contain, quantized first.
pub const MODEL_FILES: [&str; 4] = [
    "v3_e2e_rnnt_encoder",
    "v3_e2e_rnnt_decoder",
    "v3_e2e_rnnt_joint",
    "v3_e2e_rnnt_vocab.txt",
];

pub struct GigaamModel {
    encoder: Session,
    decoder: Session,
    joint: Session,
    features: FeatureExtractor,
    vocab: Vec<String>,
    blank_idx: i64,
}

/// The prediction network's state plus the cached output for the current token
/// history - the decoder only has to run again once a token is emitted.
struct Predictor {
    h: Array3<f32>,
    c: Array3<f32>,
    cached: Option<(Array3<f32>, Array3<f32>, Array3<f32>)>,
}

impl Predictor {
    fn new() -> Self {
        Self {
            h: Array3::zeros((1, 1, PRED_HIDDEN)),
            c: Array3::zeros((1, 1, PRED_HIDDEN)),
            cached: None,
        }
    }
}

impl GigaamModel {
    pub fn new<P: AsRef<Path>>(model_dir: P) -> Result<Self, GigaamError> {
        let dir = model_dir.as_ref();
        let encoder = Self::init_session(dir, MODEL_FILES[0])?;
        let decoder = Self::init_session(dir, MODEL_FILES[1])?;
        let joint = Self::init_session(dir, MODEL_FILES[2])?;
        let (vocab, blank_idx) = Self::load_vocab(dir)?;

        log::info!(
            "Loaded GigaAM vocabulary with {} tokens, blank_idx={}",
            vocab.len(),
            blank_idx
        );

        Ok(Self {
            encoder,
            decoder,
            joint,
            features: FeatureExtractor::new(),
            vocab,
            blank_idx,
        })
    }

    /// Prefer the int8 graph; fall back to the full-precision one.
    fn init_session(model_dir: &Path, stem: &str) -> Result<Session, GigaamError> {
        let quantized = model_dir.join(format!("{}.int8.onnx", stem));
        let full = model_dir.join(format!("{}.onnx", stem));
        let path = if quantized.exists() {
            quantized
        } else if full.exists() {
            full
        } else {
            return Err(GigaamError::ModelFileNotFound(format!(
                "{}.onnx",
                stem
            )));
        };

        log::info!("Loading GigaAM graph {}", path.display());
        let session = Session::builder()?
            .with_optimization_level(GraphOptimizationLevel::Level3)?
            .with_execution_providers(vec![CPUExecutionProvider::default().build()])?
            .with_parallel_execution(true)?
            .commit_from_file(path)?;

        Ok(session)
    }

    /// `token id` per line, with the sentencepiece marker turned into a space.
    fn load_vocab(model_dir: &Path) -> Result<(Vec<String>, i64), GigaamError> {
        let path = model_dir.join(MODEL_FILES[3]);
        let content = fs::read_to_string(&path).map_err(|_| {
            GigaamError::ModelFileNotFound(MODEL_FILES[3].to_string())
        })?;

        let mut entries: Vec<(String, usize)> = Vec::new();
        let mut blank_idx: Option<usize> = None;
        let mut max_id = 0usize;

        for line in content.lines() {
            let line = line.trim_end_matches(['\r', '\n']);
            let Some((token, id)) = line.rsplit_once(' ') else {
                continue;
            };
            let Ok(id) = id.parse::<usize>() else {
                continue;
            };
            if token == "<blk>" {
                blank_idx = Some(id);
            }
            max_id = max_id.max(id);
            entries.push((token.to_string(), id));
        }

        let mut vocab = vec![String::new(); max_id + 1];
        for (token, id) in entries {
            vocab[id] = token.replace('\u{2581}', " ");
        }

        Ok((vocab, blank_idx.ok_or(GigaamError::MissingBlank)? as i64))
    }

    pub fn vocab_size(&self) -> usize {
        self.vocab.len()
    }

    /// Run the encoder over 16 kHz mono samples.
    /// Returns the encoded sequence as `[frames][ENCODER_DIM]` plus its length.
    fn encode(&mut self, samples: &[f32]) -> Result<(ArrayD<f32>, usize), GigaamError> {
        let (features, frames) = self.features.compute(samples);
        // The encoder subsamples by 4 and convolves over 5 frames; anything
        // shorter than that carries no speech worth decoding anyway.
        if frames < MIN_FRAMES {
            return Ok((ArrayD::zeros(ndarray::IxDyn(&[0, ENCODER_DIM])), 0));
        }

        let audio_signal = Array3::from_shape_vec((1, N_MELS, frames), features)?;
        let length = Array1::from_vec(vec![frames as i64]);

        let outputs = self.encoder.run(inputs![
            "audio_signal" => TensorRef::from_array_view(audio_signal.view())?,
            "length" => TensorRef::from_array_view(length.view())?,
        ])?;

        let encoded = outputs
            .get("encoded")
            .ok_or_else(|| GigaamError::OutputNotFound("encoded".to_string()))?
            .try_extract_array::<f32>()?
            .to_owned();
        let encoded_len = outputs
            .get("encoded_len")
            .ok_or_else(|| GigaamError::OutputNotFound("encoded_len".to_string()))?
            .try_extract_array::<i32>()?
            .iter()
            .next()
            .copied()
            .unwrap_or(0)
            .max(0) as usize;

        // [1, D, T] -> [T, D]
        let encoded = encoded
            .permuted_axes(ndarray::IxDyn(&[0, 2, 1]))
            .remove_axis(Axis(0))
            .to_owned();
        let available = encoded.shape()[0];

        Ok((encoded, encoded_len.min(available)))
    }

    /// One step of the prediction network, cached until a token is emitted.
    fn predict(
        &mut self,
        predictor: &mut Predictor,
        last_token: i64,
    ) -> Result<(), GigaamError> {
        if predictor.cached.is_some() {
            return Ok(());
        }

        let x = Array2::from_shape_vec((1, 1), vec![last_token])?;
        let outputs = self.decoder.run(inputs![
            "x" => TensorRef::from_array_view(x.view())?,
            "h.1" => TensorRef::from_array_view(predictor.h.view())?,
            "c.1" => TensorRef::from_array_view(predictor.c.view())?,
        ])?;

        let take = |name: &str| -> Result<Array3<f32>, GigaamError> {
            let value = outputs
                .get(name)
                .ok_or_else(|| GigaamError::OutputNotFound(name.to_string()))?
                .try_extract_array::<f32>()?
                .to_owned();
            Ok(value.into_dimensionality::<ndarray::Ix3>()?)
        };

        predictor.cached = Some((take("dec")?, take("h")?, take("c")?));
        Ok(())
    }

    /// Joint network for one encoder frame; returns the vocabulary logits.
    fn join(
        &mut self,
        encoder_frame: &[f32],
        decoder_out: &Array3<f32>,
    ) -> Result<Vec<f32>, GigaamError> {
        let enc = Array3::from_shape_vec((1, ENCODER_DIM, 1), encoder_frame.to_vec())?;
        // [1, 1, 320] -> [1, 320, 1]
        let dec = decoder_out.clone().permuted_axes([0, 2, 1]);
        let dec = dec.as_standard_layout().to_owned();

        let outputs = self.joint.run(inputs![
            "enc" => TensorRef::from_array_view(enc.view())?,
            "dec" => TensorRef::from_array_view(dec.view())?,
        ])?;

        let joint = outputs
            .get("joint")
            .ok_or_else(|| GigaamError::OutputNotFound("joint".to_string()))?
            .try_extract_array::<f32>()?
            .iter()
            .copied()
            .collect::<Vec<f32>>();

        Ok(joint)
    }

    /// Greedy RNN-T decoding, matching the reference implementation in onnx-asr.
    fn decode(
        &mut self,
        encoded: &ArrayD<f32>,
        encoded_len: usize,
    ) -> Result<(Vec<usize>, Vec<usize>), GigaamError> {
        let mut predictor = Predictor::new();
        let mut tokens: Vec<usize> = Vec::new();
        let mut timestamps: Vec<usize> = Vec::new();

        let mut t = 0usize;
        let mut emitted = 0usize;

        while t < encoded_len {
            let frame = encoded.index_axis(Axis(0), t);
            let frame: Vec<f32> = frame.iter().copied().collect();

            let last_token = tokens.last().map(|&id| id as i64).unwrap_or(self.blank_idx);
            self.predict(&mut predictor, last_token)?;
            let (dec, h, c) = predictor
                .cached
                .clone()
                .expect("predict() fills the cache before returning");

            let logits = self.join(&frame, &dec)?;
            let token = logits
                .iter()
                .enumerate()
                .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
                .map(|(idx, _)| idx)
                .unwrap_or(self.blank_idx as usize);

            if token as i64 != self.blank_idx {
                tokens.push(token);
                timestamps.push(t);
                emitted += 1;
                // A new token means the prediction network has to run again
                predictor.h = h;
                predictor.c = c;
                predictor.cached = None;
            }

            if token as i64 == self.blank_idx || emitted == MAX_TOKENS_PER_STEP {
                t += 1;
                emitted = 0;
            }
        }

        Ok((tokens, timestamps))
    }

    /// Join tokens into text the way the reference decoder does.
    fn detokenize(&self, ids: &[usize], timestamps: &[usize]) -> TimestampedResult {
        let tokens: Vec<String> = ids
            .iter()
            .filter_map(|&id| self.vocab.get(id).cloned())
            .collect();

        let joined = tokens.concat();
        let text = match &*DECODE_SPACE_RE {
            Ok(regex) => regex
                .replace_all(&joined, |caps: &regex::Captures| {
                    if caps.get(1).is_some() {
                        " "
                    } else {
                        ""
                    }
                })
                .to_string(),
            Err(_) => joined,
        };

        TimestampedResult {
            text,
            timestamps: timestamps
                .iter()
                .map(|&t| WINDOW_STEP * SUBSAMPLING_FACTOR as f32 * t as f32)
                .collect(),
            tokens,
        }
    }

    /// Transcribe 16 kHz mono samples.
    pub fn transcribe_samples(
        &mut self,
        samples: &[f32],
    ) -> Result<TimestampedResult, GigaamError> {
        let (encoded, encoded_len) = self.encode(samples)?;
        if encoded_len == 0 {
            return Ok(TimestampedResult {
                text: String::new(),
                timestamps: Vec::new(),
                tokens: Vec::new(),
            });
        }

        let (tokens, timestamps) = self.decode(&encoded, encoded_len)?;
        if tokens.is_empty() {
            log::debug!(
                "GigaAM decoded only blanks over {} encoder frames",
                encoded_len
            );
        }

        Ok(self.detokenize(&tokens, &timestamps))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// End-to-end check against the reference implementation. Needs the model
    /// on disk, so it is opt-in:
    ///   GIGAAM_MODEL_DIR=<dir> GIGAAM_TEST_WAV=<file.wav> \
    ///     cargo test --lib gigaam -- --ignored --nocapture
    #[test]
    #[ignore = "needs GIGAAM_MODEL_DIR and GIGAAM_TEST_WAV"]
    fn transcribes_a_local_wav() {
        let model_dir = std::env::var("GIGAAM_MODEL_DIR").expect("GIGAAM_MODEL_DIR");
        let wav = std::env::var("GIGAAM_TEST_WAV").expect("GIGAAM_TEST_WAV");

        let (samples, sample_rate) =
            crate::diarization::dsp::read_wav(std::path::Path::new(&wav)).expect("read wav");
        assert_eq!(sample_rate, 16_000, "the test wav must be 16 kHz mono");

        let mut model = GigaamModel::new(&model_dir).expect("load model");
        let started = std::time::Instant::now();
        let result = model.transcribe_samples(&samples).expect("transcribe");
        let elapsed = started.elapsed().as_secs_f32();
        let duration = samples.len() as f32 / sample_rate as f32;

        println!("text: {}", result.text);
        println!(
            "audio {:.2}s, transcription {:.2}s, realtime factor {:.2}",
            duration,
            elapsed,
            elapsed / duration
        );
        assert!(!result.text.trim().is_empty(), "expected some text");
    }
}
