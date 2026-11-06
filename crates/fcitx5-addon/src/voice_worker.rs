//! Voice worker thread body: resolves the recorder, stream raw PCM until
//! stop/EOF/cap, wrap a WAV, walk the frozen ASR engine chain, optionally
//! polish the transcript through the LLM endpoint. Every failure is a
//! one-line `Err` for the journal; the runtime folds back to `Idle`.

use crate::voice_live;
use crate::voice_state::{PolishSettings, VoiceSettings, WorkerSignals};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::{Duration, Instant};
use voice::asr;
use voice::capture;
use voice::engine;
use voice::http::{self, Post};
use voice::json;

/// One worker-to-runtime message. `Partial` is provisional display text
/// (never committed); `Final` ends the utterance exactly like the batch
/// worker's old one-shot result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoiceEvent {
    /// Latest trailing-window transcript while capture continues.
    Partial(String),
    /// Authoritative transcript (or the utterance error).
    Final(Result<String, String>),
}

/// Worker entry used by the runtime: live mode streams partials through
/// `tx` before its one `Final`; batch mode sends only `Final`.
pub fn run_event(settings: &VoiceSettings, signals: &Arc<WorkerSignals>, tx: &Sender<VoiceEvent>) {
    let result = if settings.live {
        voice_live::run(settings, signals, tx)
    } else {
        run(settings, signals)
    };
    let _ = tx.send(VoiceEvent::Final(result));
}

/// Runs one batch utterance end to end. `signals.stopped()` ends the
/// capture loop (transcription continues); `signals.cancelled()` aborts
/// with `Err("cancelled")` and the runtime discards the result.
pub fn run(settings: &VoiceSettings, signals: &Arc<WorkerSignals>) -> Result<String, String> {
    let mut recorder = start_recorder(settings, signals)?;
    let max_bytes = max_capture_bytes(settings);
    let stream = capture::read_all(&mut recorder, max_bytes, Some(&|| signals.stopped()))
        .map_err(|err| format!("capture read: {err}"))?;
    drop(recorder); // kill + reap even on error paths
    transcribe_stream(settings, &stream, signals)
}

/// Slack over `max_duration_ms` before the recorder kill-watchdog fires.
/// The byte cap ends healthy recordings right at the max duration; this
/// only bounds recorders that never write a byte at all.
const RECORDER_DEADLINE_GRACE: Duration = Duration::from_secs(5);

/// Resolves and spawns the configured capture command behind a kill
/// watchdog: a recorder that never writes a byte blocks the worker's
/// `read` forever (the runtime would stay in Recording/Transcribing and
/// the panel hint would never clear), so the child is killed at the
/// auto-stop deadline or as soon as stop/cancel is signalled —
/// whichever comes first.
pub(crate) fn start_recorder(
    settings: &VoiceSettings,
    signals: &Arc<WorkerSignals>,
) -> Result<capture::Recorder, String> {
    let argv = capture::resolve_capture_command(&settings.capture_command, settings.sample_rate)
        .ok_or_else(|| {
            "no recorder: install pipewire/pulseaudio-utils/alsa-utils \
             or set voice.capture_command"
                .to_string()
        })?;
    let deadline = Instant::now()
        + Duration::from_millis(u64::from(settings.max_duration_ms))
        + RECORDER_DEADLINE_GRACE;
    let hot = Arc::clone(signals);
    let should_kill: Arc<dyn Fn() -> bool + Send + Sync> = Arc::new(move || hot.stopped());
    capture::start_guarded(&argv, deadline, should_kill)
        .map_err(|err| format!("capture spawn {}: {err}", argv[0]))
}

/// Turns a captured stdout stream (raw PCM or a WAV container) into the
/// final, committed transcript.
pub(crate) fn transcribe_stream(
    settings: &VoiceSettings,
    stream: &[u8],
    signals: &WorkerSignals,
) -> Result<String, String> {
    if signals.cancelled() {
        return Err("cancelled".to_string());
    }
    if stream.is_empty() {
        return Err("captured no audio (silent recorder?)".to_string());
    }
    let pcm = capture::strip_wav_header(stream)?;
    let wav = capture::wav_bytes(pcm, settings.sample_rate);
    let transcription =
        engine::transcribe_best_with_cancel(&settings.asr, &wav, &|| signals.cancelled())?;
    for line in &transcription.journal {
        eprintln!("ior: {line}");
    }
    // ASR models frequently answer `language=zh` in traditional script
    // (whisper-small does); commit simplified like the rest of the IME.
    let transcript = simplify_transcript(&transcription.text);
    if signals.cancelled() {
        return Err("cancelled".to_string());
    }
    match &settings.polish {
        None => Ok(transcript),
        Some(polish) => Ok(
            polish_transcript(polish, &transcript).unwrap_or_else(|err| {
                eprintln!("ior: voice polish failed ({err}); committing raw transcript");
                transcript
            }),
        ),
    }
}

/// Normalizes a transcript to simplified hanzi through the vendored
/// OpenCC tables. Malformed vendored data (impossible in practice) falls
/// back to the raw transcript.
pub(crate) fn simplify_transcript(transcript: &str) -> String {
    use std::sync::OnceLock;
    static TABLES: OnceLock<Option<t2s::Simplifier>> = OnceLock::new();
    match TABLES.get_or_init(|| t2s::bundled().ok()) {
        Some(s) => t2s::simplify_text(s, transcript),
        None => transcript.to_string(),
    }
}

/// s16 mono = 2 bytes/sample; the cap is the configured max duration
/// plus one chunk of slack so the auto-stop is not sample-exact.
pub(crate) fn max_capture_bytes(settings: &VoiceSettings) -> usize {
    let bytes_per_sec = u64::from(settings.sample_rate) * 2;
    let cap = bytes_per_sec * u64::from(settings.max_duration_ms) / 1_000;
    cap.saturating_add(8192) as usize
}

/// One chat-completions round with the shared polish prompt; returns
/// the model's `content` string.
fn polish_transcript(polish: &PolishSettings, transcript: &str) -> Result<String, String> {
    let url = http::parse_url(&polish.endpoint)?;
    let prompt = llm::build_polish_prompt(transcript, &[]);
    let body = format!(
        "{{\"model\":\"{}\",\"messages\":[{{\"role\":\"user\",\"content\":\"{}\"}}],\
         \"temperature\":0.2,\"max_tokens\":512}}",
        json::escape_string(&polish.model),
        json::escape_string(&prompt)
    );
    let response = http::post(&Post {
        url,
        content_type: "application/json".to_string(),
        headers: asr::auth_header(&polish.api_key_env),
        body: body.into_bytes(),
        timeout_ms: polish.timeout_ms,
    })?;
    if response.status != 200 {
        return Err(format!(
            "polish http {}: {}",
            response.status,
            response.body.trim()
        ));
    }
    json::extract_string_field(&response.body, "content")
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| "polish response had no content".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice_settings::settings_from_config;
    use ior_config::Config;

    fn settings(capture_command: &str) -> VoiceSettings {
        let mut config = Config::default();
        config.voice.capture_command = capture_command.to_string();
        config.llm.asr.endpoint = "http://127.0.0.1:1/v1/audio/transcriptions".into();
        config.llm.asr.timeout_ms = 500;
        config.voice.asr.engine = ior_config::AsrEngineMode::Remote;
        settings_from_config(&config)
    }

    fn fresh_signals() -> Arc<WorkerSignals> {
        Arc::new(WorkerSignals::new())
    }

    #[test]
    fn transcripts_normalize_to_simplified_hanzi() {
        // whisper-small answered `language=zh` in traditional script.
        assert_eq!(
            simplify_transcript("今天天氣很好,我們一起去公園三步吧!"),
            "今天天气很好,我们一起去公园三步吧!"
        );
        // Simplified input and ASCII are unchanged.
        assert_eq!(simplify_transcript("nihao 世界 ok"), "nihao 世界 ok");
    }

    #[test]
    fn byte_cap_follows_rate_and_duration() {
        let mut s = settings("cat /dev/null");
        s.sample_rate = 16_000;
        s.max_duration_ms = 1_000;
        assert_eq!(max_capture_bytes(&s), 32_000 + 8_192);
        s.max_duration_ms = 600_000;
        assert_eq!(max_capture_bytes(&s), 16_000 * 2 * 600 + 8_192);
    }

    #[test]
    fn missing_recorder_is_a_clean_error() {
        let signals = fresh_signals();
        // A custom command is not PATH-validated up front; the spawn
        // error is the clean failure.
        let err = run(&settings("ior-voice-missing-recorder"), &signals).unwrap_err();
        assert!(err.contains("capture spawn"), "{err}");
        // "auto" resolves per-host: either the install hint or a real
        // recorder argv; both are valid outcomes, only assert the shape.
        if let Some(argv) = voice::capture::resolve_capture_command("auto", 16_000) {
            assert!(!argv.is_empty());
        }
    }

    #[test]
    fn cancelled_runs_skip_the_upload() {
        use std::net::TcpListener;
        // A local listener that must never see a connection.
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        let signals = Arc::new(WorkerSignals::new());
        signals.cancel(); // cancelled before it starts
        let err = run(
            &settings(&format!("cat /dev/null #{port}-never-used")),
            &signals,
        )
        .unwrap_err();
        assert_eq!(err, "cancelled");
        listener.set_nonblocking(true).expect("nonblocking");
        match listener.accept() {
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {}
            other => panic!("upload must be skipped when cancelled: {other:?}"),
        }
    }

    #[test]
    fn cancel_during_a_slow_local_engine_skips_the_remote_fallback() {
        use std::fs;
        use std::net::TcpListener;
        use std::time::Duration;
        use voice::asr::AsrSettings;
        use voice::capture;
        use voice::engine::EngineConfig;
        use voice::sensevoice::SenseVoiceSettings;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).expect("nonblocking");

        let base = std::env::temp_dir().join(format!("ior-voice-cancel-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).expect("temp dir");
        let wav_path = base.join("fixture.wav");
        fs::write(&wav_path, capture::wav_bytes(&[0u8; 320], 16_000)).expect("wav");
        let started = base.join("local-started");

        let mut settings = settings(&format!("cat {}", wav_path.display()));
        settings.asr = vec![
            EngineConfig::SenseVoice(SenseVoiceSettings {
                argv: vec![
                    "sh".into(),
                    "-c".into(),
                    format!("touch '{}'; sleep 1; exit 1", started.display()),
                ],
                model_dir: base.clone(),
                num_threads: 1,
                timeout_ms: 5_000,
            }),
            EngineConfig::Remote(AsrSettings {
                endpoint: format!("http://127.0.0.1:{port}/v1/audio/transcriptions"),
                model: "Qwen/Qwen3-ASR-4B".to_string(),
                api_key_env: "IOR_ASR_TEST_KEY".to_string(),
                language: "zh".to_string(),
                timeout_ms: 500,
            }),
        ];
        let signals = Arc::new(WorkerSignals::new());
        let err = std::thread::scope(|scope| {
            let worker = scope.spawn(|| run(&settings, &signals));
            for _ in 0..200 {
                if started.exists() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(started.exists(), "slow local engine never started");
            signals.cancel();
            worker.join().expect("worker")
        })
        .unwrap_err();
        assert_eq!(err, "cancelled");
        match listener.accept() {
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {}
            other => panic!("remote must not be attempted after cancellation: {other:?}"),
        }
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn empty_capture_is_an_error_not_an_upload() {
        let signals = fresh_signals();
        let err = run(&settings("cat /dev/null"), &signals).unwrap_err();
        assert!(err.contains("no audio"), "{err}");
    }

    #[test]
    fn worker_transcribes_through_a_frozen_local_engine() {
        use std::fs;
        use std::sync::atomic::{AtomicU32, Ordering};
        use voice::engine::EngineConfig;
        use voice::sensevoice::SenseVoiceSettings;

        static SEQ: AtomicU32 = AtomicU32::new(0);
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let base =
            std::env::temp_dir().join(format!("ior-voice-worker-{}-{seq}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).expect("temp dir");
        let wav_path = base.join("fixture.wav");
        fs::write(&wav_path, capture::wav_bytes(&[0u8; 320], 16_000)).expect("wav");

        let mut settings = settings(&format!("cat {}", wav_path.display()));
        settings.asr = vec![EngineConfig::SenseVoice(SenseVoiceSettings {
            argv: vec!["sh".into(), "-c".into(), "echo 你好".into()],
            model_dir: base.clone(),
            num_threads: 1,
            timeout_ms: 2_000,
        })];
        let signals = fresh_signals();
        assert_eq!(run(&settings, &signals).unwrap(), "你好");
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn polish_transcript_posts_and_extracts_content() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().expect("accept");
            let mut got = vec![0u8; 65536];
            let n = sock.read(&mut got).expect("read");
            let text = String::from_utf8_lossy(&got[..n]).to_string();
            assert!(text.starts_with("POST /v1/chat/completions"), "{text}");
            assert!(text.contains("application/json"));
            assert!(text.contains("Qwen/Qwen2.5-7B-Instruct"));
            assert!(
                text.contains("Transcript: raw transcript"),
                "polish prompt: {text}"
            );
            let body = r#"{"choices":[{"message":{"role":"assistant","content":"你好，世界。"}}]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            sock.write_all(response.as_bytes()).expect("write");
        });
        let polish = PolishSettings {
            endpoint: format!("http://127.0.0.1:{port}/v1/chat/completions"),
            model: "Qwen/Qwen2.5-7B-Instruct".to_string(),
            api_key_env: "IOR_LLM_TEST_KEY".to_string(),
            timeout_ms: 2_000,
        };
        assert_eq!(
            polish_transcript(&polish, "raw transcript").unwrap(),
            "你好，世界。"
        );
        server.join().expect("server thread");
    }
}
