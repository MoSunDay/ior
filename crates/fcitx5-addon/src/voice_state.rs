//! Voice input runtime state machine for the fcitx5 addon.
//!
//! A [`VoiceRuntime`] lives on the main thread and drives a worker thread
//! ([`crate::voice_worker`]) that reports back through a channel, which
//! `poll` drains. Settings come from [`crate::voice_settings`].

use std::ffi::CString;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread::JoinHandle;

pub use crate::voice_settings::{
    PolishSettings, VoiceSettings, STATE_IDLE, STATE_RECORDING, STATE_TRANSCRIBING,
};
use crate::voice_worker::{self, VoiceEvent};

/// Cross-thread signals for one utterance: `stop` ends the recording
/// (the worker still transcribes), `cancelled` aborts everything and the
/// result is discarded. Splitting the two is what makes
/// `stop_and_transcribe` different from `cancel`.
pub struct WorkerSignals {
    /// Stop recording, keep transcribing.
    pub stop: AtomicBool,
    /// Abort: skip further uploads and drop the result. An in-flight
    /// request cannot be recalled.
    pub cancelled: AtomicBool,
}

impl WorkerSignals {
    /// Fresh "keep going" signals.
    pub fn new() -> Self {
        WorkerSignals {
            stop: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
        }
    }

    /// Ends the capture read loop.
    pub fn stop_recording(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    /// Ends the whole utterance.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    /// Whether the capture loop should stop.
    pub fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed) || self.cancelled.load(Ordering::Relaxed)
    }

    /// Whether the whole utterance was aborted.
    pub fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
}

impl Default for WorkerSignals {
    fn default() -> Self {
        Self::new()
    }
}

/// Live voice state. Owned by [`crate::engine_state::IorEngine`]; every
/// field is main-thread-only except the worker's signal flags.
pub struct VoiceRuntime {
    /// Frozen settings (from the user config at startup).
    pub settings: VoiceSettings,
    /// Current phase, one of the `STATE_*` codes.
    pub phase: u32,
    /// Finished transcript awaiting `take_text` (C string backing).
    pub text_buf: CString,
    /// Whether `text_buf` holds an untaken transcript.
    pub has_text: bool,
    /// Latest provisional live transcript (display only, never committed).
    pub partial_buf: CString,
    /// Signals shared with the worker thread.
    pub signals: Option<Arc<WorkerSignals>>,
    /// Worker event channel (live partials + one final result).
    pub rx: Option<Receiver<VoiceEvent>>,
    /// Worker thread (checked/joined lazily by poll).
    pub handle: Option<JoinHandle<()>>,
}

/// A disabled runtime (tests, `voice.enabled = false` paths).
pub fn new_runtime(settings: VoiceSettings) -> VoiceRuntime {
    VoiceRuntime {
        settings,
        phase: STATE_IDLE,
        text_buf: CString::new("").expect("empty CString"),
        has_text: false,
        partial_buf: CString::new("").expect("empty CString"),
        signals: None,
        rx: None,
        handle: None,
    }
}

/// `(keysym, mods)` of the configured hotkey, `(0, 0)` when disabled —
/// either `voice.enabled = false` or a dead hotkey spec (`"none"` /
/// unparsable; keysym 0 never matches a real key).
pub fn hotkey(runtime: &VoiceRuntime) -> (u32, u32) {
    if runtime.settings.enabled && runtime.settings.hotkey.keysym != 0 {
        (runtime.settings.hotkey.keysym, runtime.settings.hotkey.mods)
    } else {
        (0, 0)
    }
}

/// Current phase code (`STATE_*`).
pub fn phase(runtime: &VoiceRuntime) -> u32 {
    runtime.phase
}

/// Voice hotkey press. Idle starts recording; in toggle mode a press
/// while recording stops capture and starts transcription, and a press
/// while transcribing cancels. `hold` mode calls [`stop_and_transcribe`]
/// on key release; a hold-mode press while recording is a no-op, while
/// a press during transcription still cancels.
pub fn toggle(runtime: &mut VoiceRuntime) -> u32 {
    match runtime.phase {
        STATE_IDLE if runtime.settings.enabled => start_recording(runtime),
        STATE_RECORDING if runtime.settings.hold_mode => runtime.phase,
        STATE_RECORDING => stop_and_transcribe(runtime),
        STATE_TRANSCRIBING => cancel(runtime),
        _ => runtime.phase,
    }
}

/// Stops capture and transcribes: the toggle-mode second press and the
/// `hold`-mode release. A no-op in every other phase.
pub fn stop_and_transcribe(runtime: &mut VoiceRuntime) -> u32 {
    if runtime.phase == STATE_RECORDING {
        if let Some(signals) = &runtime.signals {
            signals.stop_recording();
        }
        runtime.phase = STATE_TRANSCRIBING;
    }
    runtime.phase
}

/// Aborts recording/transcription and drops any pending result. The worker
/// observes the flag between capture, each engine attempt, and polish; an
/// in-flight child/HTTP request finishes before the thread exits.
pub fn cancel(runtime: &mut VoiceRuntime) -> u32 {
    if let Some(signals) = &runtime.signals {
        signals.cancel();
    }
    // Dropping the receiver makes the worker's send fail fast. The worker
    // observes the flag after the in-flight step and exits.
    runtime.rx = None;
    runtime.handle = None;
    runtime.signals = None;
    runtime.has_text = false;
    runtime.partial_buf = CString::new("").expect("empty CString");
    runtime.phase = STATE_IDLE;
    STATE_IDLE
}

/// Drains the worker: `Partial` events update [`partial_text`] while the
/// runtime stays recording; the one `Final` event returns 1 exactly once
/// when a transcript is ready in `take_text`. Errors are logged to stderr
/// (the journal) and folded back to `Idle`.
pub fn poll(runtime: &mut VoiceRuntime) -> u32 {
    while let Some(rx) = runtime.rx.as_ref() {
        let event = rx.try_recv();
        match event {
            Ok(VoiceEvent::Partial(text)) => {
                runtime.partial_buf = to_cstring(&text);
            }
            Ok(VoiceEvent::Final(result)) => {
                match result {
                    Ok(text) => {
                        runtime.text_buf = to_cstring(&text);
                        runtime.has_text = true;
                    }
                    Err(context) => eprintln!("ior: voice: {context}"),
                }
                cleanup(runtime);
                break;
            }
            Err(mpsc::TryRecvError::Empty) => break,
            Err(mpsc::TryRecvError::Disconnected) => cleanup(runtime),
        }
    }
    u32::from(runtime.has_text)
}

/// Latest provisional transcript ("" before the first live answer). It is
/// display-only: the shell renders it as the voice preedit and replaces
/// it wholesale on every update; only `take_text` is ever committed.
pub fn partial_text(runtime: &VoiceRuntime) -> &str {
    runtime.partial_buf.to_str().unwrap_or("")
}

/// Returns the latest transcript ("" before the first one). Delivery is
/// one-shot via `poll`'s ready flag: the buffer itself stays until the
/// next result overwrites it (the returned `&str` borrows it), so only
/// call this after `poll` reported 1.
pub fn take_text(runtime: &mut VoiceRuntime) -> &str {
    runtime.has_text = false;
    runtime.text_buf.to_str().unwrap_or("")
}

/// Reaps the finished worker thread; phase returns to `Idle`.
fn cleanup(runtime: &mut VoiceRuntime) {
    if let Some(handle) = runtime.handle.take() {
        let _ = handle.join();
    }
    runtime.partial_buf = CString::new("").expect("empty CString");
    runtime.rx = None;
    runtime.signals = None;
    runtime.phase = STATE_IDLE;
}

/// Spawns the worker and enters `Recording`.
fn start_recording(runtime: &mut VoiceRuntime) -> u32 {
    let (tx, rx) = mpsc::channel();
    let signals = Arc::new(WorkerSignals::new());
    let settings = runtime.settings.clone();
    let worker_signals = Arc::clone(&signals);
    let spawn = std::thread::Builder::new()
        .name("ior-voice".to_string())
        .spawn(move || {
            // A dropped receiver (cancel) turns sends into no-ops; the
            // worker still exits promptly thanks to the signals.
            voice_worker::run_event(&settings, &worker_signals, &tx);
        });
    match spawn {
        Ok(handle) => {
            runtime.partial_buf = CString::new("").expect("empty CString");
            runtime.rx = Some(rx);
            runtime.signals = Some(signals);
            runtime.handle = Some(handle);
            runtime.phase = STATE_RECORDING;
        }
        Err(err) => eprintln!("ior: voice: worker spawn failed: {err}"),
    }
    runtime.phase
}

fn to_cstring(text: &str) -> CString {
    CString::new(text).unwrap_or_else(|_| CString::new("").expect("empty CString"))
}

impl Drop for VoiceRuntime {
    fn drop(&mut self) {
        // Engine teardown mid-recording: signal, drop the channel and
        // detach. The worker is bounded (byte cap) and self-terminates.
        cancel(self);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice_settings::settings_from_config;
    use ior_config::Config;

    fn test_settings(capture_command: &str, endpoint: &str) -> VoiceSettings {
        let mut config = Config::default();
        config.voice.capture_command = capture_command.to_string();
        config.llm.asr.endpoint = endpoint.to_string();
        config.llm.asr.timeout_ms = 500;
        config.voice.asr.engine = ior_config::AsrEngineMode::Remote;
        settings_from_config(&config)
    }

    fn wait_idle(runtime: &mut VoiceRuntime) -> u32 {
        for _ in 0..200 {
            if poll(runtime) != 0 || phase(runtime) == STATE_IDLE {
                return phase(runtime);
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("voice runtime never finished");
    }

    /// Capture stub that keeps trickling PCM so the byte cap cannot end
    /// the recording on its own. Run it as `sh <path>` (the custom
    /// capture command is split on whitespace, so no chmod is needed).
    fn slow_capture_script() -> std::path::PathBuf {
        static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "ior-voice-slow-{}-{}.sh",
            std::process::id(),
            SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::write(
            &path,
            "#!/bin/sh\nwhile :; do\n  dd if=/dev/zero bs=3200 count=1 2>/dev/null\n  sleep 0.1\ndone\n",
        )
        .expect("write slow capture script");
        path
    }

    /// Stub OpenAI-compatible transcription endpoint. It reads one request,
    /// signals `ready`, waits for `release`, then replies with `transcript`.
    /// Gating the reply keeps the worker in `transcribing` deterministically.
    fn spawn_transcript_stub(
        transcript: &'static str,
    ) -> (u16, Receiver<()>, mpsc::Sender<()>, JoinHandle<()>) {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().expect("accept");
            let mut got = vec![0u8; 65536];
            let n = sock.read(&mut got).expect("read");
            let text = String::from_utf8_lossy(&got[..n]).to_string();
            assert!(text.contains("/v1/audio/transcriptions"), "{text}");
            let _ = ready_tx.send(());
            let _ = release_rx.recv();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{transcript}",
                transcript.len()
            );
            // The client may have cancelled (receiver dropped) meanwhile.
            let _ = sock.write_all(response.as_bytes());
        });
        (port, ready_rx, release_tx, server)
    }

    /// One immediate transcription response, then the server exits. Used
    /// by live-mode tests that only need a partial (cancel never sends a
    /// second request).
    fn spawn_live_partial_stub(transcript: &str) -> (u16, JoinHandle<()>) {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        let transcript = transcript.to_string();
        let server = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().expect("accept");
            let mut got = vec![0u8; 65536];
            let n = sock.read(&mut got).expect("read request");
            let text = String::from_utf8_lossy(&got[..n]).to_string();
            assert!(text.contains("/v1/audio/transcriptions"), "{text}");
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{transcript}",
                transcript.len()
            );
            let _ = sock.write_all(response.as_bytes());
        });
        (port, server)
    }

    #[test]
    fn live_partials_surface_before_the_final_transcript() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        // First connection is the trailing-window partial; the final full
        // decode after the second press is the second connection.
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            for transcript in ["实时部分", "最终全文"] {
                let (mut sock, _) = listener.accept().expect("accept");
                let mut got = vec![0u8; 65536];
                let n = sock.read(&mut got).expect("read request");
                let text = String::from_utf8_lossy(&got[..n]).to_string();
                assert!(text.contains("/v1/audio/transcriptions"), "{text}");
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{transcript}",
                    transcript.len()
                );
                let _ = sock.write_all(response.as_bytes());
            }
        });

        let script = slow_capture_script();
        let mut settings = test_settings(
            &format!("sh {}", script.display()),
            &format!("http://127.0.0.1:{port}/v1/audio/transcriptions"),
        );
        settings.live = true;
        settings.live_interval_ms = 500;
        settings.max_duration_ms = 60_000;
        let mut runtime = new_runtime(settings);
        assert_eq!(toggle(&mut runtime), STATE_RECORDING);

        let mut partial = String::new();
        for _ in 0..600 {
            assert_eq!(phase(&runtime), STATE_RECORDING, "still capturing");
            poll(&mut runtime);
            let text = partial_text(&runtime).to_string();
            if !text.is_empty() {
                partial = text;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(partial, "实时部分", "a partial arrived before stop");
        assert_eq!(poll(&mut runtime), 0, "partials never report final text");

        assert_eq!(toggle(&mut runtime), STATE_TRANSCRIBING);
        for _ in 0..600 {
            if poll(&mut runtime) == 1 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(take_text(&mut runtime), "最终全文");
        assert_eq!(phase(&runtime), STATE_IDLE);
        assert_eq!(partial_text(&runtime), "", "idle clears the live preedit");
        server.join().expect("server thread");
        let _ = std::fs::remove_file(&script);
    }

    #[test]
    fn cancelling_live_discards_the_partial_and_any_late_result() {
        let (port, server) = spawn_live_partial_stub("会被取消");
        let script = slow_capture_script();
        let mut settings = test_settings(
            &format!("sh {}", script.display()),
            &format!("http://127.0.0.1:{port}/v1/audio/transcriptions"),
        );
        settings.live = true;
        settings.live_interval_ms = 500;
        settings.max_duration_ms = 60_000;
        let mut runtime = new_runtime(settings);
        assert_eq!(toggle(&mut runtime), STATE_RECORDING);
        for _ in 0..600 {
            poll(&mut runtime);
            if !partial_text(&runtime).is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(partial_text(&runtime), "会被取消");
        assert_eq!(cancel(&mut runtime), STATE_IDLE);
        assert_eq!(partial_text(&runtime), "");
        assert_eq!(poll(&mut runtime), 0);
        assert_eq!(take_text(&mut runtime), "");
        server.join().expect("partial server thread");
        let _ = std::fs::remove_file(&script);
    }

    #[test]
    fn toggle_second_press_stops_capture_and_transcribes() {
        // Slow capture: without the second press the worker would keep
        // recording for a minute, so ASR cannot start on its own.
        let (port, ready, release, server) = spawn_transcript_stub("你好世界");
        let script = slow_capture_script();
        let mut settings = test_settings(
            &format!("sh {}", script.display()),
            &format!("http://127.0.0.1:{port}/v1/audio/transcriptions"),
        );
        settings.max_duration_ms = 60_000;
        let mut runtime = new_runtime(settings);
        assert_eq!(toggle(&mut runtime), STATE_RECORDING);
        std::thread::sleep(std::time::Duration::from_millis(250));
        assert_eq!(
            ready.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty),
            "ASR started before the second press"
        );
        // The second press stops capture and starts transcribing.
        assert_eq!(toggle(&mut runtime), STATE_TRANSCRIBING);
        ready
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("worker reached the ASR stub after the second press");
        release.send(()).expect("release stub");
        for _ in 0..200 {
            if poll(&mut runtime) == 1 {
                break;
            }
            assert_eq!(phase(&runtime), STATE_TRANSCRIBING, "still working");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(take_text(&mut runtime), "你好世界");
        assert_eq!(phase(&runtime), STATE_IDLE);
        server.join().expect("server thread");
        let _ = std::fs::remove_file(&script);
    }

    #[test]
    fn toggle_while_transcribing_cancels_without_late_text() {
        // Second press starts transcription; a third press while
        // transcribing must abort it and the stub's late reply must never
        // surface.
        let (port, ready, release, server) = spawn_transcript_stub("迟到的文本");
        let script = slow_capture_script();
        let mut settings = test_settings(
            &format!("sh {}", script.display()),
            &format!("http://127.0.0.1:{port}/v1/audio/transcriptions"),
        );
        settings.max_duration_ms = 60_000;
        let mut runtime = new_runtime(settings);
        assert_eq!(toggle(&mut runtime), STATE_RECORDING);
        std::thread::sleep(std::time::Duration::from_millis(250));
        assert_eq!(toggle(&mut runtime), STATE_TRANSCRIBING);
        ready
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("worker reached the ASR stub");
        assert_eq!(toggle(&mut runtime), STATE_IDLE); // third press cancels
        release.send(()).expect("release stub");
        server.join().expect("server thread");
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert_eq!(poll(&mut runtime), 0);
        assert_eq!(take_text(&mut runtime), "");
        assert_eq!(phase(&runtime), STATE_IDLE);
        let _ = std::fs::remove_file(&script);
    }

    #[test]
    fn stop_and_transcribe_surfaces_the_transcript() {
        // Slow capture: only the explicit hold-release stop can end this
        // recording, so a ready ASR stub proves stop really stopped it.
        let (port, ready, release, server) = spawn_transcript_stub("你好世界");
        let script = slow_capture_script();
        let mut settings = test_settings(
            &format!("sh {}", script.display()),
            &format!("http://127.0.0.1:{port}/v1/audio/transcriptions"),
        );
        settings.max_duration_ms = 60_000;
        let mut runtime = new_runtime(settings);
        assert_eq!(toggle(&mut runtime), STATE_RECORDING);
        // Let the capture buffer some audio first (a real hold lasts
        // at least a few hundred milliseconds).
        std::thread::sleep(std::time::Duration::from_millis(250));
        assert_eq!(
            ready.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty),
            "ASR started before the hold-release stop"
        );
        assert_eq!(stop_and_transcribe(&mut runtime), STATE_TRANSCRIBING);
        ready
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("worker reached the ASR stub");
        release.send(()).expect("release stub");
        for _ in 0..200 {
            if poll(&mut runtime) == 1 {
                break;
            }
            assert_eq!(phase(&runtime), STATE_TRANSCRIBING, "still working");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(take_text(&mut runtime), "你好世界");
        assert_eq!(phase(&runtime), STATE_IDLE);
        server.join().expect("server thread");
        let _ = std::fs::remove_file(&script);
    }

    #[test]
    fn hold_toggle_while_recording_is_a_noop() {
        let mut settings = test_settings("cat /dev/null", "http://127.0.0.1:1/x");
        settings.hold_mode = true;
        let mut runtime = new_runtime(settings);
        assert_eq!(toggle(&mut runtime), STATE_RECORDING);
        assert_eq!(toggle(&mut runtime), STATE_RECORDING);
        assert_eq!(cancel(&mut runtime), STATE_IDLE);
    }

    #[test]
    fn stop_unblocks_a_recorder_that_never_writes() {
        // A silent recorder used to block the worker in read() forever:
        // no bytes -> no byte cap, and the between-reads stop check
        // never ran. The kill watchdog must end the capture so the
        // runtime (and the panel hint) returns to idle.
        let mut runtime = new_runtime(test_settings("sleep 30", "http://127.0.0.1:1/x"));
        assert_eq!(toggle(&mut runtime), STATE_RECORDING);
        assert_eq!(toggle(&mut runtime), STATE_TRANSCRIBING);
        assert_eq!(wait_idle(&mut runtime), STATE_IDLE);
        assert_eq!(take_text(&mut runtime), "");
    }

    #[test]
    fn worker_errors_return_to_idle_with_no_text() {
        // arecord-missing command -> worker fails fast -> poll -> idle.
        let mut runtime = new_runtime(test_settings(
            "ior-voice-no-such-recorder --stub",
            "http://127.0.0.1:1/x",
        ));
        assert_eq!(toggle(&mut runtime), STATE_RECORDING);
        wait_idle(&mut runtime);
        assert_eq!(poll(&mut runtime), 0);
        assert_eq!(take_text(&mut runtime), "");
    }

    #[test]
    fn disabled_runtime_ignores_the_hotkey() {
        let mut settings = test_settings("cat /dev/null", "http://127.0.0.1:1/x");
        settings.enabled = false;
        let mut runtime = new_runtime(settings);
        assert_eq!(hotkey(&runtime), (0, 0));
        assert_eq!(toggle(&mut runtime), STATE_IDLE);
        assert_eq!(phase(&runtime), STATE_IDLE);
    }

    #[test]
    fn dead_hotkey_spec_gates_like_disabled() {
        // `hotkey = "none"` (or unparsable): enabled stays true, the
        // keysym-0 gate must still kill the hotkey — and since no key
        // can reach toggle, the runtime stays idle.
        let mut settings = test_settings("cat /dev/null", "http://127.0.0.1:1/x");
        settings.hotkey.keysym = 0;
        settings.hotkey.mods = 0;
        let runtime = new_runtime(settings);
        assert_eq!(hotkey(&runtime), (0, 0));
    }
}
