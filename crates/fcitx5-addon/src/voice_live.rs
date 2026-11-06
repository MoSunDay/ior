//! Live (real-time-looking) voice mode. The configured ASR services are
//! batch-only, so this is client-synthesized streaming: while capture
//! continues, the accumulated audio is re-transcribed on a trailing
//! window and each answer is sent as a provisional `Partial`. The stop
//! path still runs the authoritative full-audio engine chain.

use crate::voice_state::{VoiceSettings, WorkerSignals};
use crate::voice_worker::{self, VoiceEvent};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use voice::asr;
use voice::capture;
use voice::engine::EngineConfig;

/// whisper-small's deployment window (`--max-model-len 448`). Partials
/// never upload more than this; the final decode is not truncated.
const MAX_PARTIAL_MS: u64 = 30_000;

/// Live worker body. Partial helpers are skipped while one is in flight:
/// node02 runs transcription with `--max-num-seqs 1`, and a queued burst
/// would only add latency.
pub(crate) fn run(
    settings: &VoiceSettings,
    signals: &Arc<WorkerSignals>,
    tx: &Sender<VoiceEvent>,
) -> Result<String, String> {
    let mut recorder = voice_worker::start_recorder(settings, signals)?;
    let stream = match partial_engine(settings) {
        None => {
            eprintln!(
                "ior: voice live partials unavailable (need a remote ASR engine); \\
                 final transcription unchanged"
            );
            capture::read_all(
                &mut recorder,
                voice_worker::max_capture_bytes(settings),
                Some(&|| signals.stopped()),
            )
            .map_err(|err| format!("capture read: {err}"))?
        }
        Some(mut remote) => {
            remote.timeout_ms = partial_timeout_ms(remote.timeout_ms, settings.live_interval_ms);
            let busy = Arc::new(AtomicBool::new(false));
            let interval_bytes = interval_bytes(settings);
            let sample_rate = settings.sample_rate;
            capture::read_live(
                &mut recorder,
                voice_worker::max_capture_bytes(settings),
                interval_bytes,
                Some(&|| signals.stopped()),
                |stream| {
                    if signals.cancelled() || busy.swap(true, Ordering::Relaxed) {
                        return;
                    }
                    let pcm = capture::strip_wav_header(stream)
                        .ok()
                        .map(|pcm| trailing_pcm(pcm, sample_rate))
                        .map(|pcm| capture::wav_bytes(pcm, sample_rate));
                    let wav = match pcm {
                        Some(wav) => wav,
                        None => {
                            busy.store(false, Ordering::Relaxed);
                            return;
                        }
                    };
                    let remote = remote.clone();
                    let tx = tx.clone();
                    let idle = Arc::clone(&busy);
                    let spawn = std::thread::Builder::new()
                        .name("ior-voice-live".to_string())
                        .spawn(move || {
                            let transcript = asr::transcribe(&remote, &wav)
                                .map(|text| voice_worker::simplify_transcript(&text));
                            idle.store(false, Ordering::Relaxed);
                            if let Ok(text) = transcript {
                                let _ = tx.send(VoiceEvent::Partial(text));
                            }
                        });
                    if let Err(err) = spawn {
                        busy.store(false, Ordering::Relaxed);
                        eprintln!("ior: voice live partial spawn failed: {err}");
                    }
                },
            )
            .map_err(|err| format!("capture read: {err}"))?
        }
    };
    drop(recorder); // kill + reap even on error paths
    voice_worker::transcribe_stream(settings, &stream, signals)
}

/// Live partials use the remote engine only: the local SenseVoice CLI
/// reloads its model on every invocation and cannot keep up with a
/// partial cadence.
fn partial_engine(settings: &VoiceSettings) -> Option<asr::AsrSettings> {
    settings.asr.iter().find_map(|engine| match engine {
        EngineConfig::Remote(remote) => Some(remote.clone()),
        EngineConfig::SenseVoice(_) => None,
    })
}

/// Bounds a partial's contention with the final request without a new
/// config knob: at least two intervals, at least two seconds, never more
/// than the endpoint's own timeout.
fn partial_timeout_ms(endpoint_ms: u64, interval_ms: u32) -> u64 {
    let budget = u64::from(interval_ms).saturating_mul(3).max(2_000);
    endpoint_ms.min(budget)
}

fn interval_bytes(settings: &VoiceSettings) -> usize {
    let bytes_per_sec = u64::from(settings.sample_rate).saturating_mul(2);
    let bytes = bytes_per_sec.saturating_mul(u64::from(settings.live_interval_ms)) / 1_000;
    usize::try_from(bytes.max(1)).unwrap_or(usize::MAX)
}

/// Latest `MAX_PARTIAL_MS` of PCM, aligned to the fixed 2-byte samples.
fn trailing_pcm(pcm: &[u8], sample_rate: u32) -> &[u8] {
    let bytes_per_sec = u64::from(sample_rate).saturating_mul(2);
    let window = bytes_per_sec.saturating_mul(MAX_PARTIAL_MS) / 1_000;
    let window = usize::try_from(window).unwrap_or(usize::MAX) & !1;
    if pcm.len() <= window {
        pcm
    } else {
        let start = pcm.len() - window;
        &pcm[start..]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_timeouts_stay_within_the_endpoint_budget() {
        assert_eq!(partial_timeout_ms(30_000, 2_000), 6_000);
        assert_eq!(partial_timeout_ms(1_000, 2_000), 1_000);
        assert_eq!(partial_timeout_ms(30_000, 500), 2_000);
    }

    #[test]
    fn trailing_pcm_caps_to_the_whisper_window_on_an_even_boundary() {
        let rate: u32 = 16_000;
        let window: usize = 30 * usize::try_from(rate).expect("rate fits usize") * 2;
        let pcm = vec![0u8; window + 3];
        assert_eq!(trailing_pcm(&pcm, rate).len(), window);
        assert_eq!(trailing_pcm(&pcm[..window], rate).len(), window);
    }
}
