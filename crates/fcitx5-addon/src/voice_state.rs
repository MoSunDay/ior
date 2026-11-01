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
use crate::voice_worker;

/// Cross-thread signals for one utterance: `stop` ends the recording
/// (the worker still transcribes), `cancelled` aborts everything and the
/// result is discarded. Splitting the two is what makes
/// `stop_and_transcribe` different from `cancel`.
pub struct WorkerSignals {
    /// Stop recording, keep transcribing.
    pub stop: AtomicBool,
    /// Abort: no upload, result dropped.
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
    /// Signals shared with the worker thread.
    pub signals: Option<Arc<WorkerSignals>>,
    /// Worker result channel.
    pub rx: Option<Receiver<Result<String, String>>>,
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
        signals: None,
        rx: None,
        handle: None,
    }
}

/// `(keysym, mods)` of the configured hotkey, `(0, 0)` when disabled.
pub fn hotkey(runtime: &VoiceRuntime) -> (u32, u32) {
    if runtime.settings.enabled {
        (runtime.settings.hotkey.keysym, runtime.settings.hotkey.mods)
    } else {
        (0, 0)
    }
}

/// Current phase code (`STATE_*`).
pub fn phase(runtime: &VoiceRuntime) -> u32 {
    runtime.phase
}

/// Voice hotkey press. Idle starts recording; anything else cancels
/// (the `toggle` semantics; `hold` presses while recording are no-ops).
pub fn toggle(runtime: &mut VoiceRuntime) -> u32 {
    match runtime.phase {
        STATE_IDLE if runtime.settings.enabled => start_recording(runtime),
        STATE_RECORDING | STATE_TRANSCRIBING => cancel(runtime),
        _ => runtime.phase,
    }
}

/// Hotkey release in `hold` mode: stop recording and transcribe. A
/// no-op in every other phase.
pub fn stop_and_transcribe(runtime: &mut VoiceRuntime) -> u32 {
    if runtime.phase == STATE_RECORDING {
        if let Some(signals) = &runtime.signals {
            signals.stop_recording();
        }
        runtime.phase = STATE_TRANSCRIBING;
    }
    runtime.phase
}

/// Aborts recording/transcription and drops any pending result.
pub fn cancel(runtime: &mut VoiceRuntime) -> u32 {
    if let Some(signals) = &runtime.signals {
        signals.cancel();
    }
    // Dropping the receiver makes the worker's send fail fast; the
    // thread exits on its own (bounded by the capture byte cap).
    runtime.rx = None;
    runtime.handle = None;
    runtime.signals = None;
    runtime.has_text = false;
    runtime.phase = STATE_IDLE;
    STATE_IDLE
}

/// Drains the worker: returns 1 when a transcript is ready in
/// `take_text`, else 0. Errors are logged to stderr (the journal) and
/// folded back to `Idle`.
pub fn poll(runtime: &mut VoiceRuntime) -> u32 {
    if let Some(rx) = &runtime.rx {
        match rx.try_recv() {
            Ok(result) => {
                match result {
                    Ok(text) => {
                        runtime.text_buf = to_cstring(&text);
                        runtime.has_text = true;
                    }
                    Err(context) => eprintln!("ior: voice: {context}"),
                }
                cleanup(runtime);
            }
            Err(mpsc::TryRecvError::Empty) => return 0,
            Err(mpsc::TryRecvError::Disconnected) => cleanup(runtime),
        }
    }
    u32::from(runtime.has_text)
}

/// Takes the finished transcript ("" when none). One-shot.
pub fn take_text(runtime: &mut VoiceRuntime) -> &str {
    runtime.has_text = false;
    runtime.text_buf.to_str().unwrap_or("")
}

/// Reaps the finished worker thread; phase returns to `Idle`.
fn cleanup(runtime: &mut VoiceRuntime) {
    if let Some(handle) = runtime.handle.take() {
        let _ = handle.join();
    }
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
            // A dropped receiver (cancel) turns the send into a no-op;
            // the worker still exits promptly thanks to the signals.
            let _ = tx.send(voice_worker::run(&settings, &worker_signals));
        });
    match spawn {
        Ok(handle) => {
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

    #[test]
    fn toggle_records_then_cancels_without_uploading() {
        let mut runtime = new_runtime(test_settings("cat /dev/null", "http://127.0.0.1:1/x"));
        assert_eq!(toggle(&mut runtime), STATE_RECORDING);
        assert_eq!(cancel(&mut runtime), STATE_IDLE);
        // The detached worker's late result must never surface.
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert_eq!(poll(&mut runtime), 0);
        assert_eq!(take_text(&mut runtime), "");
    }

    #[test]
    fn stop_and_transcribe_surfaces_the_transcript() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().expect("accept");
            let mut got = vec![0u8; 65536];
            let n = sock.read(&mut got).expect("read");
            let text = String::from_utf8_lossy(&got[..n]).to_string();
            assert!(text.contains("/v1/audio/transcriptions"), "{text}");
            let transcript = "你好世界";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{transcript}",
                transcript.len()
            );
            sock.write_all(response.as_bytes()).expect("write");
        });
        // /dev/zero streams endless PCM; a 1s cap keeps it small.
        let mut settings = test_settings(
            "cat /dev/zero",
            &format!("http://127.0.0.1:{port}/v1/audio/transcriptions"),
        );
        settings.max_duration_ms = 1_000;
        let mut runtime = new_runtime(settings);
        assert_eq!(toggle(&mut runtime), STATE_RECORDING);
        // Let the capture buffer some audio first (a real hold lasts
        // at least a few hundred milliseconds).
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert_eq!(stop_and_transcribe(&mut runtime), STATE_TRANSCRIBING);
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
}
