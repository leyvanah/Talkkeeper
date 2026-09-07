use std::path::PathBuf;
use anyhow::{Result, anyhow};
use log::{info, warn, error};
use super::encode::{encode_raw_audio, FLAC_OUTPUT_ARGS, MP4_AAC_OUTPUT_ARGS};
use super::recording_state::AudioChunk;
use serde::{Serialize, Deserialize};

use super::ffmpeg::find_ffmpeg_path;

/// Extension of the pieces a recording is written in while it runs.
/// See `FLAC_OUTPUT_ARGS` for why it is not the delivery format.
const CHECKPOINT_EXT: &str = "flac";

/// Extensions a checkpoint directory may hold. Recordings interrupted by an
/// older build left `.mp4` pieces behind, and recovery still has to join them.
const CHECKPOINT_EXTS: [&str; 2] = [CHECKPOINT_EXT, "mp4"];

fn is_checkpoint_file(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| CHECKPOINT_EXTS.contains(&ext))
}

fn ffconcat_file_line(path: &std::path::Path) -> String {
    let escaped = path.to_string_lossy().replace('\'', "'\\''");
    format!("file '{}'\n", escaped)
}

/// Join the checkpoint pieces into one delivery file.
///
/// The pieces are decoded and the result encoded **once**, over the whole
/// recording. Copying the pieces instead would be faster, but it would also
/// carry each piece's codec padding into the middle of the recording; that is
/// the audible hitch every 30 seconds this avoids.
fn concat_checkpoints(files: &[PathBuf], list_file: &PathBuf, output: &PathBuf) -> Result<()> {
    let mut list_content = String::new();
    for path in files {
        let abs_path = path
            .canonicalize()
            .map_err(|e| anyhow!("Failed to canonicalize {}: {e}", path.display()))?;
        list_content.push_str(&ffconcat_file_line(&abs_path));
    }
    std::fs::write(list_file, list_content)?;

    let ffmpeg_path = find_ffmpeg_path()
        .ok_or_else(|| anyhow!("FFmpeg not found. Please install FFmpeg to finalize recordings."))?;
    info!("Using FFmpeg at: {:?}", ffmpeg_path);

    let mut command = std::process::Command::new(ffmpeg_path);
    command
        .args([
            "-f",
            "concat", // Use concat demuxer
            "-safe",
            "0", // Allow absolute paths
            "-i",
            list_file.to_str().ok_or_else(|| anyhow!("Invalid checkpoint list path"))?,
        ])
        .args(MP4_AAC_OUTPUT_ARGS)
        .args([
            "-f",
            "mp4",
            "-y", // Overwrite output file
            output.to_str().ok_or_else(|| anyhow!("Invalid output path"))?,
        ]);

    // Hide console window on Windows to prevent CMD popup during finalization
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let ffmpeg_output = command.output()?;
    if !ffmpeg_output.status.success() {
        let stderr = String::from_utf8_lossy(&ffmpeg_output.stderr);
        error!("FFmpeg merge failed: {}", stderr);
        return Err(anyhow!("FFmpeg concat failed: {}", stderr));
    }
    if !output.exists() {
        return Err(anyhow!("Merged audio file was not created: {}", output.display()));
    }
    Ok(())
}

/// Audio data for one track
#[derive(Clone)]
struct AudioData {
    data: Vec<f32>,
}

/// Incremental audio saver that writes checkpoints every 30 seconds
/// to minimize memory usage and enable crash recovery.
///
/// `track` selects the output basename (`audio`, `mic`, `system`) so a meeting
/// can keep separate source tracks for offline diarization plus a mixed file
/// for playback.
pub struct IncrementalAudioSaver {
    checkpoint_buffer: Vec<AudioData>,
    checkpoint_interval_samples: usize,  // 30s at 48kHz = 1,440,000 samples
    checkpoint_count: u32,
    checkpoints_dir: PathBuf,
    meeting_folder: PathBuf,
    sample_rate: u32,
    /// Basename without extension: "audio" | "mic" | "system"
    track: String,
}

impl IncrementalAudioSaver {
    /// Create a new incremental saver for the mixed playback track (`audio.mp4`).
    pub fn new(meeting_folder: PathBuf, sample_rate: u32) -> Result<Self> {
        Self::new_track(meeting_folder, sample_rate, "audio")
    }

    /// Create a saver for a named track (`audio`, `mic`, or `system`).
    pub fn new_track(meeting_folder: PathBuf, sample_rate: u32, track: &str) -> Result<Self> {
        let track = track.trim().to_ascii_lowercase();
        if track.is_empty() {
            return Err(anyhow!("track name must not be empty"));
        }
        let checkpoints_dir = meeting_folder.join(".checkpoints").join(&track);

        std::fs::create_dir_all(&checkpoints_dir).map_err(|e| {
            anyhow!(
                "Failed to create checkpoints dir {}: {}",
                checkpoints_dir.display(),
                e
            )
        })?;

        Ok(Self {
            checkpoint_buffer: Vec::new(),
            checkpoint_interval_samples: sample_rate as usize * 30, // 30 seconds
            checkpoint_count: 0,
            checkpoints_dir,
            meeting_folder,
            sample_rate,
            track,
        })
    }

    /// Add an audio chunk to the buffer
    /// Automatically saves a checkpoint when buffer reaches 30 seconds
    pub fn add_chunk(&mut self, chunk: AudioChunk) -> Result<()> {
        let audio_data = AudioData {
            data: chunk.data,
            // sample_rate: chunk.sample_rate,
        };

        self.checkpoint_buffer.push(audio_data);

        // Calculate total samples in buffer
        let total_samples: usize = self.checkpoint_buffer
            .iter()
            .map(|c| c.data.len())
            .sum();

        // Save checkpoint when buffer reaches threshold (30 seconds)
        if total_samples >= self.checkpoint_interval_samples {
            self.save_checkpoint()?;
            self.checkpoint_buffer.clear();
        }

        Ok(())
    }

    /// Save current buffer as a checkpoint file
    fn save_checkpoint(&mut self) -> Result<()> {
        // Concatenate all chunks in buffer
        let audio_data: Vec<f32> = self.checkpoint_buffer
            .iter()
            .flat_map(|c| &c.data)
            .cloned()
            .collect();

        if audio_data.is_empty() {
            warn!("Attempted to save empty checkpoint, skipping");
            return Ok(());
        }

        // Generate checkpoint filename
        let checkpoint_path = self.checkpoints_dir.join(format!(
            "{}_chunk_{:03}.{CHECKPOINT_EXT}",
            self.track, self.checkpoint_count
        ));

        // Encode and save checkpoint
        encode_raw_audio(
            bytemuck::cast_slice(&audio_data),
            self.sample_rate,
            1,  // mono
            &FLAC_OUTPUT_ARGS,
            CHECKPOINT_EXT,
            &checkpoint_path
        )?;

        let duration_seconds = audio_data.len() as f32 / self.sample_rate as f32;
        self.checkpoint_count += 1;

        info!(
            "Saved {} checkpoint {}: {:.2}s ({} samples)",
            self.track,
            self.checkpoint_count,
            duration_seconds,
            audio_data.len()
        );

        Ok(())
    }

    /// Finalize the recording: save final checkpoint, merge all checkpoints, cleanup
    ///
    /// Returns the path to the final merged `{track}.mp4` file
    pub async fn finalize(&mut self) -> Result<PathBuf> {
        info!("Finalizing incremental recording track '{}'...", self.track);

        // Save final buffer if not empty
        if !self.checkpoint_buffer.is_empty() {
            info!(
                "Saving final {} checkpoint with remaining {} chunks",
                self.track,
                self.checkpoint_buffer.len()
            );
            self.save_checkpoint()?;
            self.checkpoint_buffer.clear();
        }

        if self.checkpoint_count == 0 {
            return Err(anyhow!(
                "No audio checkpoints to merge for track '{}' - recording may have failed",
                self.track
            ));
        }

        // Merge all checkpoints using FFmpeg concat
        let final_audio_path = self.meeting_folder.join(format!("{}.mp4", self.track));
        self.merge_checkpoints(&final_audio_path).await?;

        // Clean up this track's checkpoints directory
        info!(
            "Cleaning up {} {} checkpoint files",
            self.checkpoint_count, self.track
        );
        if let Err(e) = std::fs::remove_dir_all(&self.checkpoints_dir) {
            warn!("Failed to clean up checkpoints directory: {}", e);
            // Non-fatal - user can manually delete
        }
        // Removes only when all sibling tracks have also finished.
        if let Some(parent) = self.checkpoints_dir.parent() {
            let _ = std::fs::remove_dir(parent);
        }

        info!("Finalized track: {}", final_audio_path.display());

        Ok(final_audio_path)
    }

    /// Merge all checkpoint files into the final `{track}.mp4`
    async fn merge_checkpoints(&self, output: &PathBuf) -> Result<()> {
        info!("Merging {} checkpoints into final audio file...", self.checkpoint_count);

        let mut checkpoint_files = Vec::with_capacity(self.checkpoint_count as usize);
        for i in 0..self.checkpoint_count {
            let checkpoint_path = self.checkpoints_dir.join(format!(
                "{}_chunk_{:03}.{CHECKPOINT_EXT}",
                self.track, i
            ));

            // Verify checkpoint exists
            if !checkpoint_path.exists() {
                return Err(anyhow!("Checkpoint file missing: {}", checkpoint_path.display()));
            }
            checkpoint_files.push(checkpoint_path);
        }

        let list_file = self.checkpoints_dir.join("concat_list.txt");
        // Encoding a whole recording keeps a core busy for a while; off the
        // runtime's threads so the other tracks can be merged at the same time.
        let output_path = output.clone();
        tokio::task::spawn_blocking(move || {
            concat_checkpoints(&checkpoint_files, &list_file, &output_path)
        })
        .await
        .map_err(|e| anyhow!("Merge task failed: {e}"))??;

        info!("Successfully merged {} checkpoints → {}",
              self.checkpoint_count, output.display());

        Ok(())
    }

    /// Get the meeting folder path
    pub fn get_meeting_folder(&self) -> &PathBuf {
        &self.meeting_folder
    }

    /// Get current checkpoint count
    pub fn get_checkpoint_count(&self) -> u32 {
        self.checkpoint_count
    }
}

/// Whether a recording left tracks that were never given their final name.
fn has_unfinished_tracks(folder: &std::path::Path) -> bool {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let path = entry.path();
        path.extension().and_then(|ext| ext.to_str()) == Some(super::streaming_encoder::PARTIAL_EXT)
            && std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) > 0
    })
}

/// Audio recovery status for transcript recovery feature
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioRecoveryStatus {
    pub status: String, // "success" | "partial" | "failed" | "none"
    pub chunk_count: u32,
    pub estimated_duration_seconds: f64,
    pub audio_file_path: Option<String>,
    pub message: String,
}

/// Recover audio from checkpoint files
/// This is called by the transcript recovery system to merge audio chunks after a crash
fn recover_checkpoint_track(
    checkpoints_dir: &PathBuf,
    output_path: &PathBuf,
    expected_count: Option<u32>,
) -> Result<u32, String> {
    let mut checkpoint_files: Vec<PathBuf> = std::fs::read_dir(checkpoints_dir)
        .map_err(|e| format!("Failed to read {}: {e}", checkpoints_dir.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| is_checkpoint_file(path))
        .collect();
    checkpoint_files.sort();
    if checkpoint_files.is_empty() {
        return Err(format!("No checkpoints found in {}", checkpoints_dir.display()));
    }
    if let Some(expected) = expected_count {
        if checkpoint_files.len() < expected as usize {
            return Err(format!(
                "{} has {} checkpoints; expected at least {expected}",
                checkpoints_dir.display(),
                checkpoint_files.len()
            ));
        }
        checkpoint_files.truncate(expected as usize);
    }

    let concat_file = checkpoints_dir.join("concat_list.txt");
    let temp_output = output_path.with_extension("mp4.recovering");
    let _ = std::fs::remove_file(&temp_output);
    if let Err(error) = concat_checkpoints(&checkpoint_files, &concat_file, &temp_output) {
        let _ = std::fs::remove_file(&temp_output);
        return Err(format!("Failed to merge {}: {error}", output_path.display()));
    }
    if output_path.exists() {
        std::fs::remove_file(output_path)
            .map_err(|e| format!("Failed to replace {}: {e}", output_path.display()))?;
    }
    std::fs::rename(&temp_output, output_path)
        .map_err(|e| format!("Failed to promote {}: {e}", output_path.display()))?;
    std::fs::write(checkpoints_dir.join(".recovered"), b"ok")
        .map_err(|e| format!("Failed to mark recovered track: {e}"))?;
    let _ = std::fs::remove_file(concat_file);
    Ok(checkpoint_files.len() as u32)
}

#[tauri::command]
pub async fn recover_audio_from_checkpoints(
    meeting_folder: String,
    _sample_rate: u32
) -> Result<AudioRecoveryStatus, String> {
    info!("Starting audio recovery for folder: {}", meeting_folder);

    let folder_path = PathBuf::from(&meeting_folder);
    let checkpoints_root = folder_path.join(".checkpoints");

    // Getting here means the recording never closed, so its working tracks are
    // half-written and unreadable. Recovery restores the delivery tracks, which
    // is what the later passes will fall back to.
    super::working_track::discard_partial_tracks(&folder_path);

    // A recording made by this build is already encoded: it was written as it
    // went and only ever lacked its final name.
    let streamed = super::streaming_encoder::recover_partial_tracks(&folder_path);
    if !streamed.is_empty() {
        let mixed = folder_path.join("audio.mp4");
        let recovered_mixed = streamed.iter().any(|path| *path == mixed);
        let seconds = recovered_mixed
            .then(|| super::decoder::decode_audio_file(&mixed).ok())
            .flatten()
            .map(|audio| audio.duration_seconds)
            .unwrap_or(0.0);
        return Ok(AudioRecoveryStatus {
            status: if recovered_mixed { "success" } else { "partial" }.to_string(),
            chunk_count: streamed.len() as u32,
            estimated_duration_seconds: seconds,
            audio_file_path: recovered_mixed.then(|| mixed.to_string_lossy().into_owned()),
            message: format!("Recovered {} interrupted track(s)", streamed.len()),
        });
    }

    // Older recordings were written in pieces, each track in its own subfolder.
    // Recover mixed playback exactly as legacy recovery did; mic/system remain
    // optional diagnostics.
    let checkpoints_dir = if checkpoints_root.join("audio").is_dir() {
        checkpoints_root.join("audio")
    } else {
        checkpoints_root.clone()
    };

    if !checkpoints_dir.exists() {
        info!("No checkpoints directory found at: {}", checkpoints_dir.display());
        return Ok(AudioRecoveryStatus {
            status: "none".to_string(),
            chunk_count: 0,
            estimated_duration_seconds: 0.0,
            audio_file_path: None,
            message: "No audio checkpoints found".to_string(),
        });
    }

    let output_path = folder_path.join("audio.mp4");
    let chunk_count = match recover_checkpoint_track(&checkpoints_dir, &output_path, None) {
        Ok(count) => count,
        Err(error) if error.starts_with("No checkpoints") => {
            return Ok(AudioRecoveryStatus {
                status: "none".to_string(),
                chunk_count: 0,
                estimated_duration_seconds: 0.0,
                audio_file_path: None,
                message: error,
            });
        }
        Err(error) => {
            error!("Mixed audio recovery failed: {error}");
            return Ok(AudioRecoveryStatus {
                status: "failed".to_string(),
                chunk_count: 0,
                estimated_duration_seconds: 0.0,
                audio_file_path: None,
                message: error,
            });
        }
    };

    let mut source_failures = Vec::new();
    for track in ["mic", "system"] {
        let track_dir = checkpoints_root.join(track);
        if track_dir.is_dir() {
            if let Err(error) = recover_checkpoint_track(
                &track_dir,
                &folder_path.join(format!("{track}.mp4")),
                Some(chunk_count),
            ) {
                warn!("Could not recover {track} track: {error}");
                source_failures.push(format!("{track}: {error}"));
            }
        }
    }

    let status = if source_failures.is_empty() { "success" } else { "partial" };
    let message = if source_failures.is_empty() {
        format!("Successfully recovered {chunk_count} audio chunks and available source tracks")
    } else {
        format!("Recovered mixed audio; source-track recovery failed ({})", source_failures.join("; "))
    };
    Ok(AudioRecoveryStatus {
        status: status.to_string(),
        chunk_count,
        estimated_duration_seconds: chunk_count as f64 * 30.0,
        audio_file_path: Some(output_path.to_string_lossy().into_owned()),
        message,
    })
}

/// Clean up checkpoint files after successful recording or recovery
/// This command is called by the frontend after successful save to clean up checkpoint files
#[tauri::command]
pub async fn cleanup_checkpoints(meeting_folder: String) -> Result<(), String> {
    info!("Cleaning up checkpoints for folder: {}", meeting_folder);

    let folder_path = PathBuf::from(&meeting_folder);
    let checkpoints_root = folder_path.join(".checkpoints");

    if checkpoints_root.exists() {
        if checkpoints_root.join("audio").is_dir() {
            for track in ["audio", "mic", "system"] {
                let track_dir = checkpoints_root.join(track);
                if folder_path.join(format!("{track}.mp4")).is_file()
                    && track_dir.join(".recovered").is_file()
                {
                    std::fs::remove_dir_all(&track_dir)
                        .map_err(|e| format!("Failed to remove {}: {e}", track_dir.display()))?;
                }
            }
            let _ = std::fs::remove_dir(&checkpoints_root);
        } else if folder_path.join("audio.mp4").is_file() {
            std::fs::remove_dir_all(&checkpoints_root)
                .map_err(|e| format!("Failed to remove checkpoints directory: {e}"))?;
        }
        info!("Cleaned checkpoints for successfully recovered tracks");
    } else {
        info!("No checkpoints directory to clean up");
    }

    Ok(())
}

/// Check if a meeting folder has audio checkpoint files
/// Returns true if .checkpoints/ directory exists and contains checkpoint pieces
#[tauri::command]
pub async fn has_audio_checkpoints(meeting_folder: String) -> Result<bool, String> {
    let folder_path = PathBuf::from(&meeting_folder);

    // A recording made by this build leaves its tracks under a temporary name
    // instead of checkpoint pieces; either way there is audio to recover.
    if has_unfinished_tracks(&folder_path) {
        return Ok(true);
    }

    let checkpoints_root = folder_path.join(".checkpoints");
    let checkpoints_dir = if checkpoints_root.join("audio").is_dir() {
        checkpoints_root.join("audio")
    } else {
        checkpoints_root
    };

    // Check if checkpoints directory exists
    if !checkpoints_dir.exists() {
        return Ok(false);
    }

    // Scan for checkpoint pieces
    let has_checkpoint_files = std::fs::read_dir(&checkpoints_dir)
        .map_err(|e| format!("Failed to read checkpoints directory: {}", e))?
        .filter_map(|entry| entry.ok())
        .any(|entry| is_checkpoint_file(&entry.path()));

    Ok(has_checkpoint_files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use super::super::recording_state::DeviceType;

    #[test]
    fn concat_paths_escape_apostrophes() {
        let line = ffconcat_file_line(std::path::Path::new("/Users/O'Brien/audio.mp4"));
        assert_eq!(line, "file '/Users/O'\\''Brien/audio.mp4'\n");
    }

    #[tokio::test]
    async fn test_checkpoint_creation() {
        // Create temp meeting folder
        let temp_dir = tempdir().unwrap();
        let meeting_folder = temp_dir.path().join("Test_Meeting");
        std::fs::create_dir_all(&meeting_folder).unwrap();
        std::fs::create_dir_all(meeting_folder.join(".checkpoints")).unwrap();

        let mut saver = IncrementalAudioSaver::new(
            meeting_folder.clone(),
            48000
        ).unwrap();

        // Add 60 seconds worth of audio (should create 2 checkpoints)
        for i in 0..120 {  // 120 chunks of 0.5s each
            let chunk = AudioChunk {
                data: vec![0.5f32; 24000],  // 0.5s at 48kHz
                sample_rate: 48000,
                timestamp: i as f64 * 0.5,  // timestamp in seconds
                chunk_id: i as u64,
                device_type: DeviceType::Microphone,
            };
            saver.add_chunk(chunk).unwrap();
        }

        // Verify 2 checkpoints created
        assert_eq!(saver.checkpoint_count, 2);

        // Finalize and verify merge
        let final_path = saver.finalize().await.unwrap();
        assert!(final_path.exists());

        // Verify checkpoints directory deleted
        assert!(!meeting_folder.join(".checkpoints").exists());
    }

    /// Checkpoint boundaries must leave no trace in the finished recording.
    /// Joining encoded pieces by copying used to insert each piece's codec
    /// padding — tens of milliseconds of dead air every 30 seconds, mid-word.
    #[tokio::test]
    async fn merged_checkpoints_do_not_add_audio_at_the_seams() {
        let temp_dir = tempdir().unwrap();
        let meeting_folder = temp_dir.path().join("Seam_Test");
        std::fs::create_dir_all(&meeting_folder).unwrap();

        let sample_rate = 48_000u32;
        let mut saver =
            IncrementalAudioSaver::new_track(meeting_folder.clone(), sample_rate, "audio").unwrap();

        // Two full checkpoints of a continuous tone, so a seam falls mid-signal.
        let total_samples = sample_rate as usize * 60;
        let block = 24_000usize;
        for i in 0..(total_samples / block) {
            let data = (0..block)
                .map(|n| {
                    let t = (i * block + n) as f32 / sample_rate as f32;
                    (t * 440.0 * std::f32::consts::TAU).sin() * 0.25
                })
                .collect();
            saver
                .add_chunk(AudioChunk {
                    data,
                    sample_rate,
                    timestamp: (i * block) as f64 / sample_rate as f64,
                    chunk_id: i as u64,
                    device_type: DeviceType::Mixed,
                })
                .unwrap();
        }
        assert_eq!(saver.checkpoint_count, 2, "expected a seam to test");

        let final_path = saver.finalize().await.unwrap();
        let joined = super::super::decoder::decode_audio_file(&final_path).unwrap();

        // Compare against the same signal encoded in one piece: the delivery
        // codec's own delay and end padding are then common to both, and any
        // difference is what the seam added.
        let tone: Vec<f32> = (0..total_samples)
            .map(|n| (n as f32 / sample_rate as f32 * 440.0 * std::f32::consts::TAU).sin() * 0.25)
            .collect();
        let one_piece = meeting_folder.join("one_piece.mp4");
        super::super::encode::encode_single_audio(
            bytemuck::cast_slice(&tone),
            sample_rate,
            1,
            &one_piece,
        )
        .unwrap();
        let reference = super::super::decoder::decode_audio_file(&one_piece).unwrap();

        let added = joined.samples.len() as i64 - reference.samples.len() as i64;
        assert_eq!(
            added, 0,
            "the seam added {added} samples ({:.1} ms) to a {:.0} second recording",
            added as f64 / sample_rate as f64 * 1000.0,
            total_samples as f64 / sample_rate as f64
        );
    }

    #[tokio::test]
    async fn test_empty_recording() {
        let temp_dir = tempdir().unwrap();
        let meeting_folder = temp_dir.path().join("Empty_Test");
        std::fs::create_dir_all(&meeting_folder).unwrap();
        std::fs::create_dir_all(meeting_folder.join(".checkpoints")).unwrap();

        let mut saver = IncrementalAudioSaver::new(
            meeting_folder.clone(),
            48000
        ).unwrap();

        // Try to finalize without adding any chunks
        let result = saver.finalize().await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No audio checkpoints"));
    }
}
