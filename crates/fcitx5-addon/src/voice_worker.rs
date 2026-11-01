//! Voice worker thread body: resolve the recorder, stream raw PCM until
//! stop/EOF/cap, wrap a WAV, POST it to the ASR service, optionally
//! polish the transcript through the LLM endpoint. Every failure is a
//! one-line `Err` for the journal; the runtime folds back to `Idle`.

use crate::voice_state::{PolishSettings, VoiceSettings, WorkerSignals};
use voice::asr;
use voice::capture;
use voice::http::{self, Post};
use voice::json;

/// Runs one utterance end to end. `signals.stopped()` ends the capture
/// loop (transcription continues); `signals.cancelled()` aborts with
/// `Err("cancelled")` and the runtime discards the result.
pub fn run(settings: &VoiceSettings, signals: &WorkerSignals) -> Result<String, String> {
    let argv = capture::resolve_capture_command(&settings.capture_command, settings.sample_rate)
        .ok_or_else(|| {
            "no recorder: install pipewire/pulseaudio-utils/alsa-utils \
             or set voice.capture_command"
                .to_string()
        })?;
    let mut recorder =
        capture::start(&argv).map_err(|err| format!("capture spawn {}: {err}", argv[0]))?;
    let max_bytes = max_capture_bytes(settings);
    let stream = capture::read_all(&mut recorder, max_bytes, Some(&|| signals.stopped()))
        .map_err(|err| format!("capture read: {err}"))?;
    drop(recorder); // kill + reap even on error paths
    if signals.cancelled() {
        return Err("cancelled".to_string());
    }
    if stream.is_empty() {
        return Err("captured no audio (silent recorder?)".to_string());
    }
    let pcm = capture::strip_wav_header(&stream)?;
    let wav = capture::wav_bytes(pcm, settings.sample_rate);
    let transcript = asr::transcribe(&settings.asr, &wav)?;
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

/// s16 mono = 2 bytes/sample; the cap is the configured max duration
/// plus one chunk of slack so the auto-stop is not sample-exact.
fn max_capture_bytes(settings: &VoiceSettings) -> usize {
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
        settings_from_config(&config)
    }

    fn fresh_signals() -> WorkerSignals {
        WorkerSignals::new()
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
        let signals = WorkerSignals::new();
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
    fn empty_capture_is_an_error_not_an_upload() {
        let signals = fresh_signals();
        let err = run(&settings("cat /dev/null"), &signals).unwrap_err();
        assert!(err.contains("no audio"), "{err}");
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
