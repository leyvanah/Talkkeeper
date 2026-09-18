use super::*;
use crate::audio::echo_cancel::EchoCanceller;
use std::path::{Path, PathBuf};

fn bench_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/audio-bench")
        .canonicalize()
        .expect("run scripts/make-bench-speech.ps1 first")
}

/// The bench files are 48 kHz mono 16-bit PCM, written by our own script,
/// so the header is read positionally rather than with a decoder.
fn read_wav(path: &Path) -> Vec<f32> {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&bytes[0..4], b"RIFF", "{} is not a WAV", path.display());
    assert_eq!(bytes.len() % 2, 0);
    bytes[44..]
        .chunks_exact(2)
        .map(|pair| i16::from_le_bytes([pair[0], pair[1]]) as f32 / 32768.0)
        .collect()
}

fn write_wav(path: &Path, samples: &[f32], sample_rate: u32) {
    let data_len = samples.len() * 2;
    let mut out = Vec::with_capacity(44 + data_len);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_len as u32).to_le_bytes());
    for sample in samples {
        let clamped = (sample.clamp(-1.0, 1.0) * 32767.0) as i16;
        out.extend_from_slice(&clamped.to_le_bytes());
    }
    std::fs::write(path, out).unwrap();
}

/// How rough the waveform is: the largest step between neighbouring samples
/// against the average step. A splice shows up here long before it is
/// audible, and this is the same measure used when the capture chain was
/// compared against ffmpeg recording the same microphone.
fn spikiness(samples: &[f32]) -> f32 {
    let steps: Vec<f32> = samples.windows(2).map(|w| (w[1] - w[0]).abs()).collect();
    if steps.is_empty() {
        return 0.0;
    }
    let mean = steps.iter().sum::<f32>() / steps.len() as f32;
    let max = steps.iter().cloned().fold(0.0f32, f32::max);
    if mean == 0.0 {
        0.0
    } else {
        max / mean
    }
}

/// Steps far larger than the signal's own average — one per audible click.
/// Counting them is what turns "I think I heard a few" into a number.
fn click_count(samples: &[f32]) -> usize {
    let steps: Vec<f32> = samples.windows(2).map(|w| (w[1] - w[0]).abs()).collect();
    if steps.is_empty() {
        return 0;
    }
    let mean = steps.iter().sum::<f32>() / steps.len() as f32;
    let threshold = (mean * 25.0).max(0.02);
    steps.iter().filter(|step| **step > threshold).count()
}

/// Arrival times modelled on the measured recordings: mostly on time, with
/// stretches where the handler falls behind by up to `peak` and then
/// delivers in a burst. Deterministic, so two runs compare.
/// A plateau, not a sawtooth, because that is what the logs showed: the
/// handler falls behind and *stays* behind for seconds at a time, which is
/// exactly what defeats a smoothing window that takes the minimum over one.
/// A sawtooth dips back to zero every cycle and the minimum never rises.
///
/// Seven seconds on time, three seconds behind — the rhythm that produced
/// a repair roughly every 6.7s in the owner's 74-second recording.
fn jittered_arrival(block_index: usize, block_seconds: f64, peak: f64) -> f64 {
    let ideal = block_index as f64 * block_seconds;
    let phase = ideal % 10.0;
    let behind = if phase >= 7.0 {
        // Ramp on over 300ms so the plateau starts like a handler getting
        // busy rather than like a cut.
        let into = phase - 7.0;
        peak * (into / 0.3).min(1.0)
    } else {
        0.0
    };
    ideal + behind
}

#[test]
#[ignore = "audio bench: run scripts/make-bench-speech.ps1 first"]
fn speech_through_the_mixer_under_measured_jitter() {
    let dir = bench_dir();
    let sample_rate = 48_000u32;
    let mic_source = read_wav(&dir.join("speaker-ru.wav"));
    let far_source = read_wav(&dir.join("farend-ru.wav"));

    let block = sample_rate as usize / 100; // 10ms, as the device delivers
    let block_seconds = block as f64 / sample_rate as f64;

    let mut ring = AudioMixerRingBuffer::new(sample_rate, true, true);
    let mut mixed_mic = Vec::new();

    let blocks = mic_source.len() / block;
    for index in 0..blocks {
        let from = index * block;
        let mic_block = mic_source[from..from + block].to_vec();
        // The far end plays continuously; it runs out before the near end,
        // so it repeats rather than falling silent halfway.
        let far_block: Vec<f32> = (0..block)
            .map(|offset| far_source[(from + offset) % far_source.len()])
            .collect();

        // The microphone is the jittered one, as in the logs; the system
        // capture arrives on time.
        ring.add_samples(
            DeviceType::Microphone,
            mic_block,
            jittered_arrival(index, block_seconds, 0.35) + block_seconds,
        );
        ring.add_samples(
            DeviceType::System,
            far_block,
            (index as f64 + 1.0) * block_seconds,
        );

        while let Some((mic_window, _system_window)) = ring.extract_window() {
            mixed_mic.extend_from_slice(&mic_window);
        }
    }
    if let Some((mic_window, _)) = ring.extract_remaining() {
        mixed_mic.extend_from_slice(&mic_window);
    }

    write_wav(&dir.join("out-mic.wav"), &mixed_mic, sample_rate);

    let delivered = mic_source.len() as f64 / sample_rate as f64;
    let produced = mixed_mic.len() as f64 / sample_rate as f64;
    println!("\n=== speech through the mixer, jitter peaking at 350ms ===");
    println!("  fed in            {delivered:.3}s");
    println!("  came out          {produced:.3}s  ({:+.3}s)", produced - delivered);
    println!("  {}", ring.seam_report());
    println!("  spikiness  source {:.1}  ->  output {:.1}", spikiness(&mic_source), spikiness(&mixed_mic));
    println!("  hard steps source {}  ->  output {}", click_count(&mic_source), click_count(&mixed_mic));
    println!("  written to {}", dir.join("out-mic.wav").display());

    // The bench prints for a person to read, but one thing is worth failing
    // on: audio must not be invented or lost wholesale.
    assert!(
        (produced - delivered).abs() < 1.0,
        "the pipeline changed the recording's length by {:.3}s",
        produced - delivered
    );
}

/// Energy of a span, for comparing what is left of the far end.
fn energy(samples: &[f32]) -> f64 {
    samples.iter().map(|s| (*s as f64) * (*s as f64)).sum()
}

/// The complaint this bench exists for: a video playing through the
/// speakers was transcribed as the owner's own speech.
///
/// The scene is the one he made by hand — the far end plays, its echo
/// reaches the microphone, and he talks over it — under the delivery jitter
/// the logs measured. What it checks is not the canceller in isolation
/// (there is a unit test for that) but the canceller fed by the mixer:
/// echo suppression works on *aligned* pairs, and alignment is exactly what
/// broke. A window of microphone half filled with silence against a full
/// window of system audio puts the two out of step, and the far end's voice
/// survives in the near channel.
#[test]
#[ignore = "audio bench: run scripts/make-bench-speech.ps1 first"]
fn the_far_end_does_not_survive_in_the_near_channel() {
    let dir = bench_dir();
    let sample_rate = 48_000u32;
    let near = read_wav(&dir.join("speaker-ru.wav"));
    let far = read_wav(&dir.join("farend-ru.wav"));

    let block = sample_rate as usize / 100;
    let block_seconds = block as f64 / sample_rate as f64;
    // The speakers are about 120ms away through the air and the room, and
    // what returns is far quieter than what was played.
    let echo_delay = (0.120 * sample_rate as f64) as usize;
    let echo_gain = 0.35f32;
    // The owner stays quiet for the first stretch, so what is left of the
    // far end there can be measured on its own.
    let near_starts = sample_rate as usize * 8;

    let mut ring = AudioMixerRingBuffer::new(sample_rate, true, true);
    let mut canceller = EchoCanceller::new(sample_rate).expect("canceller for 48 kHz");

    let mut cleaned = Vec::new();
    let mut echo_only = Vec::new();
    let mut held: Vec<(Vec<f32>, f64)> = Vec::new();

    let blocks = near.len() / block;
    for index in 0..blocks {
        let from = index * block;

        let far_block: Vec<f32> = (0..block)
            .map(|offset| far[(from + offset) % far.len()])
            .collect();
        let mic_block: Vec<f32> = (0..block)
            .map(|offset| {
                let at = from + offset;
                let echo = if at >= echo_delay {
                    far[(at - echo_delay) % far.len()] * echo_gain
                } else {
                    0.0
                };
                let own = if at >= near_starts { near[at] } else { 0.0 };
                echo + own
            })
            .collect();

        echo_only.extend(mic_block.iter().take(if from < near_starts { block } else { 0 }));

        // A busy handler does not deliver late, it does not deliver at all
        // and then delivers everything at once. Holding the blocks back and
        // releasing them in a burst is what empties the buffer on the other
        // side — modelling only the timestamp leaves the buffer full and
        // misses the failure entirely.
        held.push((mic_block, jittered_arrival(index, block_seconds, 0.35) + block_seconds));
        // 350ms of held blocks every three seconds: the measured lag,
        // delivered the way a busy handler delivers it.
        let busy = (index as f64 * block_seconds) % 3.0 >= 2.65;
        if !busy {
            for (block, arrival) in held.drain(..) {
                ring.add_samples(DeviceType::Microphone, block, arrival);
            }
        }

        ring.add_samples(
            DeviceType::System,
            far_block,
            (index as f64 + 1.0) * block_seconds,
        );

        while let Some((mic_window, system_window)) = ring.extract_window() {
            cleaned.extend_from_slice(&canceller.process(&mic_window, &system_window));
        }
    }
    for (block, arrival) in held.drain(..) {
        ring.add_samples(DeviceType::Microphone, block, arrival);
    }
    while let Some((mic_window, system_window)) = ring.extract_window() {
        cleaned.extend_from_slice(&canceller.process(&mic_window, &system_window));
    }

    write_wav(&dir.join("out-aec.wav"), &cleaned, sample_rate);

    // Measure over the stretch where only the far end was playing: whatever
    // is left there is echo the canceller did not remove.
    let quiet = near_starts.min(cleaned.len());
    let before = energy(&echo_only[..quiet.min(echo_only.len())]);
    let after = energy(&cleaned[..quiet]);
    let erle = 10.0 * (before / after.max(1e-12)).log10();

    // And the owner's own voice has to still be there afterwards.
    let own_after = energy(&cleaned[quiet..]);
    let own_before = energy(&near[near_starts..near.len().min(cleaned.len())]);
    let kept = 10.0 * (own_after / own_before.max(1e-12)).log10();

    println!("\n=== far end through the speakers, owner talking over it ===");
    println!("  {}", ring.seam_report());
    println!("  echo left in the near channel: ERLE {erle:.1} dB");
    println!("  owner's own voice afterwards:  {kept:+.1} dB against the source");
    println!("  written to {}", dir.join("out-aec.wav").display());

    assert!(
        erle > 10.0,
        "only {erle:.1} dB of the far end was removed; it would be transcribed as the owner"
    );
    assert!(
        kept > -6.0,
        "the owner's own voice lost {kept:.1} dB, the canceller is eating the near end"
    );
}

/// The whole hybrid, end to end, on the scene it exists for.
///
/// The owner is talking over a far end coming out of his speakers. The
/// microphone we record is the ordinary one, so his voice is all there and
/// so is the echo; the detector stream is what Windows hands a video call,
/// which holds his voice and nothing of the far end. What has to come out
/// of it: the stretch where only the speakers were playing survives as no
/// microphone speech at all, and the stretch where he spoke over them
/// survives whole.
///
/// This is the case a plain canceller cannot settle. 35 dB of suppression
/// still leaves something, and something is enough for a recognizer to
/// make words out of and hand to the wrong speaker.
#[test]
#[ignore = "audio bench: run scripts/make-bench-speech.ps1 first"]
fn the_speakers_do_not_become_the_owners_words() {
    let dir = bench_dir();
    let sample_rate = 48_000u32;
    let near = read_wav(&dir.join("speaker-ru.wav"));
    let far = read_wav(&dir.join("farend-ru.wav"));

    let block = sample_rate as usize / 100;
    let block_seconds = block as f64 / sample_rate as f64;
    let echo_delay = (0.120 * sample_rate as f64) as usize;
    let echo_gain = 0.35f32;
    // Nothing but the speakers for the first stretch, so the two halves of
    // the claim can be measured apart from each other.
    let near_starts = sample_rate as usize * 8;
    let near_starts_ms = near_starts as f64 / sample_rate as f64 * 1000.0;

    let mut ring = AudioMixerRingBuffer::new(sample_rate, true, true);
    let mut canceller = EchoCanceller::new(sample_rate).expect("canceller for 48 kHz");
    let gate = crate::audio::own_speech::OwnSpeechGate::new();
    gate.opened(sample_rate);

    let mut own_timeline =
        crate::audio::own_speech::WindowTimeline::new(MIXING_WINDOW_MS as f64);
    let mut far_timeline =
        crate::audio::own_speech::WindowTimeline::new(MIXING_WINDOW_MS as f64);
    // A real meeting folder this time: the second half of the bench reads
    // the track back off disk the way a retranscription does.
    let meeting = tempfile::tempdir().expect("meeting folder");
    let mut mic_work = WorkingTrack::new(
        "microphone",
        sample_rate,
        mixing_window_samples(sample_rate),
        Some(crate::audio::working_track::working_track_path(
            meeting.path(),
            "mic",
        )),
    )
    .expect("working track");
    let mut record =
        crate::audio::own_speech_record::GateRecorder::new(MIXING_WINDOW_MS);
    record.open_in(meeting.path());
    let mut vad = ContinuousVadProcessor::new_with_thresholds(WORKING_SAMPLE_RATE, 800, 0.20, 0.10)
        .expect("VAD");

    let mut segments = Vec::new();
    let blocks = near.len() / block;
    for index in 0..blocks {
        let from = index * block;
        let arrival = (index + 1) as f64 * block_seconds;

        let far_block: Vec<f32> = (0..block)
            .map(|offset| far[(from + offset) % far.len()])
            .collect();
        // The owner's own voice, as the detector stream carries it: the
        // far end is not in there at all, by construction.
        let own_block: Vec<f32> = (0..block)
            .map(|offset| {
                let at = from + offset;
                if at >= near_starts {
                    near[at]
                } else {
                    0.0
                }
            })
            .collect();
        let mic_block: Vec<f32> = (0..block)
            .map(|offset| {
                let at = from + offset;
                let echo = if at >= echo_delay {
                    far[(at - echo_delay) % far.len()] * echo_gain
                } else {
                    0.0
                };
                echo + own_block[offset]
            })
            .collect();

        gate.push(&own_block);
        ring.add_samples(DeviceType::Microphone, mic_block, arrival);
        ring.add_samples(DeviceType::System, far_block, arrival);

        while let Some((mic_window, sys_window)) = ring.extract_window() {
            let far_end = Some(far_end_is_playing(&sys_window));
            let own_speech = gate.take_window(MIXING_WINDOW_MS as f64);
            far_timeline.push(far_end);
            own_timeline.push(own_speech);
            record.observe(own_speech, far_end);

            let cleaned = canceller.process(&mic_window, &sys_window);
            let mic_16k = mic_work.push(&cleaned);
            if let Ok(found) = vad.process_audio(&mic_16k) {
                segments.extend(found);
            }
        }
    }
    if let Ok(found) = vad.flush() {
        segments.extend(found);
    }
    mic_work.finish();
    record.finish();

    let mut echo_kept_ms = 0.0;
    let mut own_kept_ms = 0.0;
    // What the recording would have carried without the detector, so the
    // bench says whether it caught anything at all rather than passing
    // because there was nothing to catch.
    let mut echo_without_detector_ms = 0.0;
    let mut dropped = 0usize;
    for segment in &segments {
        let from = segment.start_timestamp_ms;
        let to = segment.end_timestamp_ms;
        if to <= near_starts_ms {
            echo_without_detector_ms += to - from;
        }
        let (own_ms, far_ms) = measure_segment(&own_timeline, &far_timeline, from, to);
        let is_echo = crate::audio::own_speech::is_only_the_speakers(own_ms, far_ms);
        if is_echo {
            dropped += 1;
            continue;
        }
        if to <= near_starts_ms {
            echo_kept_ms += to - from;
        } else {
            own_kept_ms += to - from;
        }
    }

    println!("\n=== the speakers, and the owner talking over them ===");
    println!("  {}", ring.seam_report());
    println!("  microphone segments found:  {}", segments.len());
    println!("  dropped as the speakers:    {dropped}");
    println!("  echo the canceller left:    {echo_without_detector_ms:.0} ms");
    println!("  echo kept as his speech:    {echo_kept_ms:.0} ms");
    println!("  his own speech kept:        {own_kept_ms:.0} ms");

    assert!(
        echo_without_detector_ms > 500.0,
        "the canceller left nothing for the detector to catch, so this bench proves nothing"
    );
    assert!(
        echo_kept_ms < 500.0,
        "{echo_kept_ms:.0} ms of the speakers survived as the owner's own speech"
    );
    assert!(
        own_kept_ms > 3_000.0,
        "only {own_kept_ms:.0} ms of his own speech was kept; the detector is eating him"
    );

    // The same recording, read again from disk — which is the half this
    // whole record exists for. The live pass had the detector running; a
    // retranscription has only the file, and until the record was written
    // down the two reached different verdicts about the same session.
    //
    // Nothing here is reused from above: the track is decoded off disk,
    // segmented by the offline VAD with the settings retranscription uses,
    // and judged by the same function retranscription calls.
    let stored = crate::audio::working_track::find_working_track(meeting.path(), "mic")
        .expect("the working track was published");
    let decoded = crate::audio::decoder::decode_audio_file(&stored).expect("decodes");
    let mut samples = decoded.to_whisper_format();
    let his_half = (near_starts_ms / 1000.0 * WORKING_SAMPLE_RATE as f64) as usize;
    let stored_echo_before = energy(&samples[..his_half.min(samples.len())]);
    let his_half_before = energy(&samples[his_half.min(samples.len())..]);

    let (own_read, far_read) =
        crate::audio::own_speech_record::read_timelines(meeting.path())
            .expect("the record was published beside the track");
    let spans = crate::audio::own_speech::echo_spans(&own_read, &far_read).len();
    let untouched = crate::audio::vad::get_speech_chunks_with_thresholds_and_progress(
        &samples,
        2_000,
        0.20,
        0.10,
        |_, _| true,
    )
    .expect("offline VAD");
    let his_speech_untouched: f64 = untouched
        .iter()
        .map(|segment| {
            (segment.end_timestamp_ms - segment.start_timestamp_ms.max(near_starts_ms)).max(0.0)
        })
        .sum();

    let silenced_ms = crate::audio::retranscription::silence_the_speakers(
        &mut samples,
        WORKING_SAMPLE_RATE,
        &own_read,
        &far_read,
    );
    let offline = crate::audio::vad::get_speech_chunks_with_thresholds_and_progress(
        &samples,
        2_000,
        0.20,
        0.10,
        |_, _| true,
    )
    .expect("offline VAD");

    let stored_echo_after = energy(&samples[..his_half.min(samples.len())]);
    let his_half_after = energy(&samples[his_half.min(samples.len())..]);
    let his_speech_ms: f64 = offline
        .iter()
        .map(|segment| {
            (segment.end_timestamp_ms - segment.start_timestamp_ms.max(near_starts_ms)).max(0.0)
        })
        .sum();

    let echo_left = stored_echo_after / stored_echo_before;
    let his_speech_kept = his_speech_ms / his_speech_untouched.max(1.0);
    let too_early = offline
        .iter()
        .filter(|segment| segment.end_timestamp_ms <= near_starts_ms)
        .count();

    println!("\n=== the same recording, read again from disk ===");
    println!("  stretches silenced:         {spans} ({silenced_ms:.0} ms)");
    println!("  far end left in the track:  {stored_echo_before:.4} → {stored_echo_after:.4} ({:.0}% left)", echo_left * 100.0);
    println!("  his own half of the track:  {his_half_before:.1} → {his_half_after:.1}");
    println!(
        "  speech found after {:.0}s:     {his_speech_untouched:.0} → {his_speech_ms:.0} ms",
        near_starts_ms / 1000.0
    );
    println!("  segments before he spoke:   {too_early}");

    // First: the stored track really does carry the far end. Without this
    // the rest would pass on a scene with nothing in it.
    assert!(
        stored_echo_before > 1e-4,
        "the stored track holds no leftover far end, so reading it back proves nothing"
    );
    // Second: what is left of it cannot be taken for speech. Not zero —
    // the record says when the speakers *played*, and the echo of that
    // arrives about 120 ms later, so the tail after each pause survives —
    // but far too little to be recognized, and the pass finds no speech at
    // all before he starts.
    assert!(
        echo_left < 0.25,
        "{:.0}% of the far end is still in the track a later pass would transcribe",
        echo_left * 100.0
    );
    assert_eq!(
        too_early, 0,
        "the second reading still found speech in the stretch where only the speakers played"
    );
    // Third, and the one that matters most: his own speech survives. A
    // record that ate any of it would be worse than no record at all.
    assert!(
        his_speech_kept > 0.95,
        "the second reading kept only {:.0}% of what he said",
        his_speech_kept * 100.0
    );
}
