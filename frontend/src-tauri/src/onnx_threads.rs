//! Thread budgets for ONNX Runtime sessions.
//!
//! ORT gives every session its own intra-op pool ("per session threadpools"),
//! sized to the machine by default, and its threads spin between runs. A live
//! recording keeps four sessions busy (the GigaAM encoder, decoder and joint,
//! plus the speaker embedder), which on an 8-thread laptop saturated the
//! processor and starved a video call running beside the app. Every session
//! therefore gets an explicit, small pool whose threads sleep when idle.
//!
//! All the numbers live here, keyed by what a session does, so that one place
//! decides how much of the machine inference may take.
//!
//! Sessions built by a dependency are out of reach of [`budget`]: the silero
//! VAD hardcodes four intra-op threads, and with spinning on and a run every
//! 32 ms on each source, its pools never went to sleep. [`init_environment`]
//! gives the process a global pool of one thread that never spins; ORT puts
//! every session on it unless the session asks for its own, which ours do.

use ort::environment::GlobalThreadPoolOptions;
use ort::session::builder::SessionBuilder;

/// What a session is for; each role has its own intra-op thread count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// GigaAM encoder, the one heavy graph per chunk. The same session serves
    /// live transcription, retranscribe and import.
    GigaamEncoder,
    /// GigaAM decoder and joint network: tiny per-token steps.
    GigaamStep,
    /// Parakeet encoder.
    ParakeetEncoder,
    /// Parakeet mel preprocessor and fused decoder/joint.
    ParakeetStep,
    /// Speaker embedder during a recording, beside transcription.
    LiveSpeaker,
    /// Offline speaker identification, which waits out a recording.
    OfflineSpeaker,
}

/// Inter-op threads. Sessions run sequentially, so this pool stays unused.
const INTER_THREADS: usize = 1;

/// Whether idle pool threads may spin before blocking. Spinning is what kept
/// the cores busy between runs.
const SPINNING: bool = false;

/// Intra-op threads for a session in `role`.
pub fn intra_threads(role: Role) -> usize {
    match role {
        Role::GigaamEncoder | Role::ParakeetEncoder | Role::LiveSpeaker => 2,
        Role::GigaamStep | Role::ParakeetStep => 1,
        // Half the logical cores, between 1 and 4.
        Role::OfflineSpeaker => std::thread::available_parallelism()
            .map(|n| n.get() / 2)
            .unwrap_or(1)
            .clamp(1, 4),
    }
}

/// Threads of the global pool, which serves every session not built through
/// [`budget`]. One thread runs the graph on the caller and starts no workers.
const GLOBAL_INTRA_THREADS: usize = 1;

/// Install the process-wide ORT environment with the global pool. Must run
/// before the first session is built; later calls change nothing.
pub fn init_environment() {
    let options = GlobalThreadPoolOptions::default()
        .with_intra_threads(GLOBAL_INTRA_THREADS)
        .and_then(|o| o.with_inter_threads(INTER_THREADS))
        .and_then(|o| o.with_spin_control(SPINNING));
    let committed = options.and_then(|o| {
        ort::init()
            .with_name("talkkeeper")
            .with_global_thread_pool(o)
            .commit()
    });
    match committed {
        Ok(true) => log::info!("ONNX Runtime: global pool of {GLOBAL_INTRA_THREADS} thread, no spinning"),
        Ok(false) => log::warn!("ONNX Runtime environment already existed; global pool not installed"),
        Err(e) => log::warn!("ONNX Runtime global pool not installed: {e}"),
    }
}

/// Sequential graph execution with the thread budget of `role`, and pool
/// threads that block instead of spinning between runs. The session keeps a
/// pool of its own rather than sharing the one-thread global pool.
pub fn budget(builder: SessionBuilder, role: Role) -> ort::Result<SessionBuilder> {
    builder
        .with_independent_thread_pool()?
        .with_parallel_execution(false)?
        .with_intra_threads(intra_threads(role))?
        .with_inter_threads(INTER_THREADS)?
        .with_intra_op_spinning(SPINNING)?
        .with_inter_op_spinning(SPINNING)
}

#[cfg(all(test, windows))]
mod bench {
    //! CPU cost of the live path (GigaAM + live speaker embedding, paced in
    //! real time) and of the offline passes, measured as process CPU time.
    //!
    //! ```text
    //! TALKKEEPER_BENCH_GIGAAM=<gigaam model dir> \
    //! TALKKEEPER_BENCH_DIARIZATION=<diarization model dir> \
    //! TALKKEEPER_BENCH_WAV=<16 kHz mono wav> \
    //! cargo test --lib onnx_threads::bench -- --ignored --nocapture
    //! ```
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use crate::diarization::models::DiarizationModels;
    use crate::gigaam_engine::GigaamModel;

    /// Live segments in the app are cut by VAD; a few seconds is typical.
    const LIVE_SEGMENT_SECS: f32 = 5.0;

    #[repr(C)]
    struct FileTime {
        low: u32,
        high: u32,
    }

    extern "system" {
        fn GetCurrentProcess() -> isize;
        fn GetProcessTimes(
            process: isize,
            creation: *mut FileTime,
            exit: *mut FileTime,
            kernel: *mut FileTime,
            user: *mut FileTime,
        ) -> i32;
    }

    fn cpu_secs() -> f64 {
        let zero = || FileTime { low: 0, high: 0 };
        let (mut c, mut e, mut k, mut u) = (zero(), zero(), zero(), zero());
        unsafe { GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u) };
        // FILETIME counts 100 ns ticks.
        let ticks = |f: &FileTime| ((f.high as u64) << 32 | f.low as u64) as f64;
        (ticks(&k) + ticks(&u)) / 1e7
    }

    fn env_path(key: &str) -> PathBuf {
        PathBuf::from(std::env::var(key).unwrap_or_else(|_| panic!("{key} is not set")))
    }

    /// The two live VADs (microphone and system), fed 50 ms windows in real
    /// time for 20 s, as the pipeline does. No models needed: silero embeds
    /// its own. `TALKKEEPER_BENCH_NO_GLOBAL_POOL=1` measures the old setup.
    ///
    /// ```text
    /// cargo test --lib onnx_threads::bench::vad_live_cpu -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "paced benchmark, 20 s"]
    fn vad_live_cpu() {
        const WINDOW: usize = 800; // 50 ms at 16 kHz
        const WINDOWS: usize = 400; // 20 s
        let global_pool = std::env::var_os("TALKKEEPER_BENCH_NO_GLOBAL_POOL").is_none();
        if global_pool {
            super::init_environment();
        }

        let (cpu0, wall0) = (cpu_secs(), Instant::now());
        let sources: Vec<_> = (0..2)
            .map(|source| {
                std::thread::spawn(move || {
                    let mut vad = crate::audio::vad::ContinuousVadProcessor::new(16_000, 400)
                        .expect("vad");
                    // Alternating seconds of tone and near-silence, so the VAD
                    // goes through both states.
                    let mut n = 0usize;
                    for i in 0..WINDOWS {
                        let block: Vec<f32> = (0..WINDOW)
                            .map(|_| {
                                n += 1;
                                let t = n as f32 / 16_000.0;
                                let on = (t as usize + source) % 2 == 0;
                                let amp = if on { 0.3 } else { 0.001 };
                                amp * (t * 2.0 * std::f32::consts::PI * 220.0).sin()
                            })
                            .collect();
                        vad.process_audio(&block).expect("process");
                        let due = wall0 + Duration::from_millis(50 * (i as u64 + 1));
                        std::thread::sleep(due.saturating_duration_since(Instant::now()));
                    }
                })
            })
            .collect();
        for source in sources {
            source.join().expect("vad thread");
        }
        let (cpu, wall) = (cpu_secs() - cpu0, wall0.elapsed().as_secs_f64());
        println!(
            "vad x2 ({}): {wall:.1}s wall, {cpu:.2}s cpu = {:.2} cores avg",
            if global_pool { "global pool, 1 thread" } else { "silero default, 4 threads" },
            cpu / wall
        );
    }

    #[test]
    #[ignore = "needs the models and a wav; see the module docs"]
    fn live_and_offline_cpu() {
        let gigaam_dir = env_path("TALKKEEPER_BENCH_GIGAAM");
        let diar_dir = env_path("TALKKEEPER_BENCH_DIARIZATION");
        let wav = env_path("TALKKEEPER_BENCH_WAV");
        let (samples, sr) = crate::diarization::dsp::read_wav(&wav).expect("wav");
        assert_eq!(sr, 16_000, "bench expects 16 kHz audio");
        let audio_secs = samples.len() as f64 / 16_000.0;

        let mut gigaam = GigaamModel::new(&gigaam_dir).expect("gigaam");
        let mut live = DiarizationModels::load(&diar_dir, super::Role::LiveSpeaker)
            .expect("diarization");

        // Live: every segment is transcribed and embedded as it "arrives".
        let step = (LIVE_SEGMENT_SECS * 16_000.0) as usize;
        let (cpu0, wall0) = (cpu_secs(), Instant::now());
        let mut busy = Duration::ZERO;
        let mut worst = Duration::ZERO;
        for (i, seg) in samples.chunks(step).enumerate() {
            let t = Instant::now();
            gigaam.transcribe_samples(seg).expect("transcribe");
            if seg.len() >= 16_000 {
                live.embed(seg).expect("embed");
            }
            let spent = t.elapsed();
            busy += spent;
            worst = worst.max(spent);
            let due = wall0 + Duration::from_secs_f32(LIVE_SEGMENT_SECS * (i + 1) as f32);
            std::thread::sleep(due.saturating_duration_since(Instant::now()));
        }
        let (cpu, wall) = (cpu_secs() - cpu0, wall0.elapsed().as_secs_f64());
        let segments = samples.len().div_ceil(step);
        println!(
            "live: {audio_secs:.0}s audio, {wall:.1}s wall, {cpu:.1}s cpu = {:.2} cores avg; \
             per segment {:.0} ms mean, {:.0} ms worst",
            cpu / wall,
            busy.as_secs_f64() * 1000.0 / segments as f64,
            worst.as_secs_f64() * 1000.0
        );
        drop(live);

        // Offline transcription (retranscribe/import): back to back, 30 s chunks.
        let (cpu0, wall0) = (cpu_secs(), Instant::now());
        for seg in samples.chunks(30 * 16_000) {
            gigaam.transcribe_samples(seg).expect("transcribe");
        }
        let (cpu, wall) = (cpu_secs() - cpu0, wall0.elapsed().as_secs_f64());
        println!(
            "offline transcription: {wall:.1}s wall ({:.1}x realtime), {cpu:.1}s cpu",
            audio_secs / wall
        );
        drop(gigaam);

        // Offline speaker identification after the recording.
        let (cpu0, wall0) = (cpu_secs(), Instant::now());
        let result = crate::diarization::diarize_file_with_models(&wav, &diar_dir, None, None)
            .expect("diarize");
        let (cpu, wall) = (cpu_secs() - cpu0, wall0.elapsed().as_secs_f64());
        println!(
            "offline diarization: {wall:.1}s wall, {cpu:.1}s cpu, {} speakers, {} segments",
            result.num_speakers,
            result.segments.len()
        );
    }

    /// Same live pacing for Parakeet (`TALKKEEPER_BENCH_PARAKEET` = the int8
    /// model dir), without the speaker embedder.
    #[test]
    #[ignore = "needs the models and a wav; see the module docs"]
    fn parakeet_live_cpu() {
        let model_dir = env_path("TALKKEEPER_BENCH_PARAKEET");
        let wav = env_path("TALKKEEPER_BENCH_WAV");
        let (samples, sr) = crate::diarization::dsp::read_wav(&wav).expect("wav");
        assert_eq!(sr, 16_000, "bench expects 16 kHz audio");
        let audio_secs = samples.len() as f64 / 16_000.0;
        let mut model = crate::parakeet_engine::ParakeetModel::new(&model_dir, true).expect("parakeet");

        let step = (LIVE_SEGMENT_SECS * 16_000.0) as usize;
        let (cpu0, wall0) = (cpu_secs(), Instant::now());
        let mut busy = Duration::ZERO;
        let mut worst = Duration::ZERO;
        for (i, seg) in samples.chunks(step).enumerate() {
            let t = Instant::now();
            model.transcribe_samples(seg.to_vec()).expect("transcribe");
            let spent = t.elapsed();
            busy += spent;
            worst = worst.max(spent);
            let due = wall0 + Duration::from_secs_f32(LIVE_SEGMENT_SECS * (i + 1) as f32);
            std::thread::sleep(due.saturating_duration_since(Instant::now()));
        }
        let (cpu, wall) = (cpu_secs() - cpu0, wall0.elapsed().as_secs_f64());
        let segments = samples.len().div_ceil(step);
        println!(
            "parakeet live: {audio_secs:.0}s audio, {wall:.1}s wall, {cpu:.1}s cpu = {:.2} cores avg; \
             per segment {:.0} ms mean, {:.0} ms worst",
            cpu / wall,
            busy.as_secs_f64() * 1000.0 / segments as f64,
            worst.as_secs_f64() * 1000.0
        );

        let (cpu0, wall0) = (cpu_secs(), Instant::now());
        for seg in samples.chunks(30 * 16_000) {
            model.transcribe_samples(seg.to_vec()).expect("transcribe");
        }
        let (cpu, wall) = (cpu_secs() - cpu0, wall0.elapsed().as_secs_f64());
        println!(
            "parakeet offline: {wall:.1}s wall ({:.1}x realtime), {cpu:.1}s cpu",
            audio_secs / wall
        );
    }
}
