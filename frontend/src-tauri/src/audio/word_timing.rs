//! When each word of a transcript was said.
//!
//! The recognizers already know this, roughly: Whisper keeps a time for every
//! token, GigaAM for every emission. Kept, it lets the transcript put a
//! sentence where it was spoken rather than where an even pace would put it,
//! and lets playback mark the word being heard.
//!
//! Recognizers emit sub-word pieces, and the piece that starts a word starts
//! with a space — both Whisper's byte-level tokens and GigaAM's vocabulary
//! follow that convention. Whisper's pieces are raw bytes: a Cyrillic letter
//! is two bytes and can be split across two tokens, so pieces are joined as
//! bytes and only a whole word is decoded.
//!
//! Times are seconds. The recognizers give them from the start of the audio
//! they were handed; [`shift`] moves them onto the recording's clock.

use serde::{Deserialize, Serialize};

/// One word and when it was said.
///
/// Stored as JSON with one-letter keys: an hour of speech is some ten thousand
/// of these, and the key names would otherwise be most of the column.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WordTiming {
    #[serde(rename = "w")]
    pub text: String,
    #[serde(rename = "s")]
    pub start: f64,
    #[serde(rename = "e")]
    pub end: f64,
}

/// A piece of a word as a recognizer reports it.
#[derive(Debug, Clone)]
pub struct TimedPiece {
    pub bytes: Vec<u8>,
    pub start: f64,
    pub end: f64,
}

/// Join pieces into words.
///
/// A piece beginning with whitespace starts a new word; any other piece —
/// the rest of a word, or punctuation — belongs to the word before it.
/// Times are kept in order: a word never starts before the one it follows,
/// and never ends before it starts.
pub fn words_from_pieces(pieces: impl IntoIterator<Item = TimedPiece>) -> Vec<WordTiming> {
    let mut words = Vec::new();
    let mut bytes: Vec<u8> = Vec::new();
    let mut start = 0.0f64;
    let mut end = 0.0f64;

    fn finish(bytes: &mut Vec<u8>, start: f64, end: f64, words: &mut Vec<WordTiming>) {
        let text = String::from_utf8_lossy(bytes).trim().to_string();
        bytes.clear();
        if text.is_empty() {
            return;
        }
        let floor = words.last().map(|w: &WordTiming| w.start).unwrap_or(0.0);
        let start = start.max(floor);
        words.push(WordTiming {
            text,
            start,
            end: end.max(start),
        });
    }

    for piece in pieces {
        let begins_word = piece
            .bytes
            .first()
            .map(|b| b.is_ascii_whitespace())
            .unwrap_or(false);
        if begins_word && !bytes.is_empty() {
            finish(&mut bytes, start, end, &mut words);
        }
        if bytes.is_empty() {
            start = piece.start;
            end = piece.end;
        }
        bytes.extend_from_slice(&piece.bytes);
        end = end.max(piece.end);
    }
    if !bytes.is_empty() {
        finish(&mut bytes, start, end, &mut words);
    }
    words
}

/// Move word times by `seconds`, onto the clock of the whole recording.
pub fn shift(words: &mut [WordTiming], seconds: f64) {
    for word in words {
        word.start += seconds;
        word.end += seconds;
    }
}

/// Whether the words spell out `text`, ignoring spacing.
///
/// Anything that edits the text after recognition — a repetition cleaned
/// up, a hallucination removed — leaves the words describing text that is no
/// longer there. Such timings are dropped rather than shown against the wrong
/// words.
pub fn spell(words: &[WordTiming], text: &str) -> bool {
    let joined: String = words
        .iter()
        .flat_map(|word| word.text.chars())
        .filter(|c| !c.is_whitespace())
        .collect();
    let expected: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    !joined.is_empty() && joined == expected
}

pub fn to_json(words: &[WordTiming]) -> String {
    serde_json::to_string(words).unwrap_or_else(|_| "[]".to_string())
}

/// `None` for anything that is not a list of words: the timings are an extra,
/// and a damaged one should cost the extra, not the transcript.
pub fn from_json(json: &str) -> Option<Vec<WordTiming>> {
    serde_json::from_str(json).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn piece(bytes: &[u8], start: f64, end: f64) -> TimedPiece {
        TimedPiece {
            bytes: bytes.to_vec(),
            start,
            end,
        }
    }

    fn texts(words: &[WordTiming]) -> Vec<&str> {
        words.iter().map(|w| w.text.as_str()).collect()
    }

    #[test]
    fn a_letter_split_across_tokens_comes_back_whole() {
        // " привет" with the "р" (0xD1 0x80) cut in half between two tokens.
        let word = " привет".as_bytes();
        let cut = word.iter().position(|&b| b == 0xD1).unwrap() + 1;
        let words = words_from_pieces([
            piece(&word[..cut], 0.0, 0.2),
            piece(&word[cut..], 0.2, 0.5),
            piece(" мир".as_bytes(), 0.6, 0.9),
        ]);
        assert_eq!(texts(&words), ["привет", "мир"]);
        assert_eq!((words[0].start, words[0].end), (0.0, 0.5));
        assert_eq!((words[1].start, words[1].end), (0.6, 0.9));
    }

    #[test]
    fn punctuation_stays_with_its_word() {
        let words = words_from_pieces([
            piece(b" Hello", 0.0, 0.3),
            piece(b",", 0.3, 0.35),
            piece(b" world", 0.4, 0.8),
            piece(b".", 0.8, 0.85),
        ]);
        assert_eq!(texts(&words), ["Hello,", "world."]);
        assert_eq!(words[1].end, 0.85);
    }

    #[test]
    fn the_first_piece_need_not_start_with_a_space() {
        let words = words_from_pieces([piece(b"Da", 0.0, 0.1), piece(b" net", 0.2, 0.3)]);
        assert_eq!(texts(&words), ["Da", "net"]);
    }

    #[test]
    fn times_never_run_backwards() {
        let words = words_from_pieces([
            piece(b" one", 1.0, 1.5),
            piece(b" two", 0.8, 0.7),
        ]);
        assert_eq!(words[1].start, 1.0, "a word does not start before the one it follows");
        assert!(words[1].end >= words[1].start);
    }

    #[test]
    fn nothing_but_spaces_is_no_word() {
        assert!(words_from_pieces([piece(b" ", 0.0, 0.1)]).is_empty());
        assert!(words_from_pieces(Vec::new()).is_empty());
    }

    #[test]
    fn shifting_moves_every_word() {
        let mut words = words_from_pieces([piece(b" a", 0.5, 1.0)]);
        shift(&mut words, 10.0);
        assert_eq!((words[0].start, words[0].end), (10.5, 11.0));
    }

    #[test]
    fn words_that_no_longer_spell_the_text_are_recognised() {
        let words = words_from_pieces([piece(" да да".as_bytes(), 0.0, 1.0)]);
        assert!(spell(&words, "да да"));
        assert!(spell(&words, "да  да"), "spacing does not matter");
        assert!(!spell(&words, "да"), "an edit shows");
        assert!(!spell(&[], ""), "no words spell nothing useful");
    }

    #[test]
    fn stored_form_round_trips_and_is_compact() {
        let words = vec![WordTiming {
            text: "слово".into(),
            start: 1.25,
            end: 1.5,
        }];
        let json = to_json(&words);
        assert_eq!(json, r#"[{"w":"слово","s":1.25,"e":1.5}]"#);
        assert_eq!(from_json(&json), Some(words));
        assert_eq!(from_json("not json"), None);
    }
}

/// Both recognizers on real speech, side by side.
///
/// There is no ground truth for when a synthesized word was said, so the
/// check is the next best thing: two unrelated models — a transducer that
/// places each token on a 40 ms frame, and Whisper estimating times from its
/// decoder — should agree on when the words they both heard began.
///
///   cargo test --lib word_timing_bench -- --ignored --nocapture
///
/// Needs the models under `target/debug/data/models` and the bench speech
/// from `scripts/make-bench-speech.ps1`.
#[cfg(test)]
mod word_timing_bench {
    use super::*;
    use std::path::{Path, PathBuf};

    fn repo_path(relative: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(relative)
    }

    fn speech_16k() -> Vec<f32> {
        let (samples, rate) =
            crate::diarization::dsp::read_wav(&repo_path("target/audio-bench/speaker-ru.wav"))
                .expect("bench speech");
        crate::audio::audio_processing::resample_audio(&samples, rate, 16_000)
    }

    fn show(label: &str, words: &[WordTiming]) {
        println!("\n=== {label}: {} words ===", words.len());
        for word in words {
            println!("  {:6.2}–{:6.2}  {}", word.start, word.end, word.text);
        }
    }

    fn normal(word: &str) -> String {
        word.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .map(|c| if c == 'ё' { 'е' } else { c })
            .collect()
    }

    #[tokio::test]
    #[ignore = "needs the models and the bench speech"]
    async fn word_timing_bench_whisper_and_gigaam_agree() {
        let models = repo_path("target/debug/data/models");
        let samples = speech_16k();
        let duration = samples.len() as f64 / 16_000.0;

        let gigaam = crate::gigaam_engine::GigaamEngine::new_with_models_dir(Some(models.clone()))
            .expect("gigaam engine");
        gigaam.load_model().await.expect("gigaam model");
        let (giga_text, giga_words) = gigaam
            .transcribe_audio_with_words(samples.clone())
            .await
            .expect("gigaam");
        let giga_words = giga_words.expect("gigaam gave word timings that spell its text");
        println!("GigaAM text: {giga_text}");
        show("GigaAM", &giga_words);

        let whisper = crate::whisper_engine::WhisperEngine::new_with_models_dir(Some(models))
            .expect("whisper engine");
        whisper.discover_models().await.expect("discover");
        whisper.load_model("large-v3-turbo").await.expect("whisper model");
        let (whisper_text, _, _, whisper_words) = whisper
            .transcribe_audio_with_words(samples, Some("ru".to_string()), None)
            .await
            .expect("whisper");
        let whisper_words = whisper_words.expect("whisper gave word timings that spell its text");
        println!("Whisper text: {whisper_text}");
        show("Whisper", &whisper_words);

        for words in [&giga_words, &whisper_words] {
            assert!(words.windows(2).all(|w| w[1].start >= w[0].start), "times run forward");
            assert!(words.iter().all(|w| w.end <= duration + 0.5), "times stay inside the audio");
        }

        // Pair the words both heard, in order, and compare when they began.
        let mut differences = Vec::new();
        let mut from = 0;
        for word in &giga_words {
            let key = normal(&word.text);
            if key.chars().count() < 4 {
                continue;
            }
            if let Some(offset) = whisper_words[from..]
                .iter()
                .take(6)
                .position(|other| normal(&other.text) == key)
            {
                let other = &whisper_words[from + offset];
                differences.push((word.start - other.start).abs());
                from += offset + 1;
            }
        }
        differences.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let median = differences[differences.len() / 2];
        let p90 = differences[differences.len() * 9 / 10];
        println!(
            "\npaired {} words; start difference median {:.2}s, 90th percentile {:.2}s",
            differences.len(),
            median,
            p90
        );
        assert!(differences.len() >= 10, "too few words in common to judge");
        // Measured with alignment heads: median 0.20 s, 90th percentile 0.30 s.
        // Without them Whisper drifted to a 3 s median, which is what this is
        // here to catch.
        assert!(median < 0.35, "the two models disagree by {median:.2}s on a typical word");
        assert!(p90 < 0.6, "one word in ten is off by {p90:.2}s or more");
    }
}
