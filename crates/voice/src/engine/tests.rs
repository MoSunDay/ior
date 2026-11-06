use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

static SEQ: AtomicU32 = AtomicU32::new(0);

fn temp_dir(tag: &str) -> PathBuf {
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "ior-engine-test-{tag}-{}-{seq}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    fs::write(dir.join(sensevoice::MODEL_FILE), b"model").expect("model");
    fs::write(dir.join(sensevoice::TOKENS_FILE), b"tokens").expect("tokens");
    dir
}

fn remote(endpoint: &str) -> AsrSettings {
    AsrSettings {
        endpoint: endpoint.to_string(),
        model: "Qwen/Qwen3-ASR-4B".to_string(),
        api_key_env: "IOR_ASR_TEST_KEY".to_string(),
        language: "zh".to_string(),
        timeout_ms: 2_000,
    }
}

fn options(policy: EnginePolicy, model_dir: &Path, fallback: bool) -> EngineOptions {
    EngineOptions {
        policy,
        fallback,
        sensevoice: SenseVoiceConfig {
            command: "sh".to_string(),
            model_dir: model_dir.to_string_lossy().into_owned(),
            num_threads: 1,
            timeout_ms: 2_000,
        },
        remote: remote("http://127.0.0.1:1/v1/audio/transcriptions"),
    }
}

#[test]
fn auto_prefers_local_then_remote() {
    let dir = temp_dir("auto");
    let chain = resolve_chain(&options(EnginePolicy::Auto, &dir, true));
    assert_eq!(chain_summary(&chain), "sensevoice -> remote");
    let local_only = resolve_chain(&options(EnginePolicy::Auto, &dir, false));
    assert_eq!(chain_summary(&local_only), "sensevoice");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn auto_without_local_is_remote_only() {
    let dir = temp_dir("missing");
    let _ = fs::remove_dir_all(&dir);
    let chain = resolve_chain(&options(EnginePolicy::Auto, &dir, true));
    assert_eq!(chain_summary(&chain), "remote");
    let no_fallback = resolve_chain(&options(EnginePolicy::Auto, &dir, false));
    assert_eq!(chain_summary(&no_fallback), "remote");
}

#[test]
fn explicit_sensevoice_without_local_obeys_fallback() {
    let dir = temp_dir("no-local");
    let _ = fs::remove_dir_all(&dir);
    assert_eq!(
        chain_summary(&resolve_chain(&options(
            EnginePolicy::SenseVoice,
            &dir,
            true
        ))),
        "remote"
    );
    assert!(resolve_chain(&options(EnginePolicy::SenseVoice, &dir, false)).is_empty());
}

#[test]
fn remote_policy_ignores_the_local_probe() {
    let dir = temp_dir("remote-policy");
    let chain = resolve_chain(&options(EnginePolicy::Remote, &dir, true));
    assert_eq!(chain_summary(&chain), "remote");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn transcribe_best_falls_back_and_journals() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut sock, _) = listener.accept().expect("accept");
        let mut got = vec![0u8; 65536];
        let _ = sock.read(&mut got).expect("read");
        let transcript = "远端结果";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}",
            transcript.len(),
            transcript
        );
        sock.write_all(response.as_bytes()).expect("write");
    });
    let dir = temp_dir("fallback");
    let mut opts = options(EnginePolicy::Auto, &dir, true);
    opts.remote = remote(&format!("http://127.0.0.1:{port}/v1/audio/transcriptions"));
    let mut chain = resolve_chain(&opts);
    chain[0] = EngineConfig::SenseVoice(SenseVoiceSettings {
        argv: vec!["sh".into(), "-c".into(), "exit 1".into()],
        model_dir: dir.clone(),
        num_threads: 1,
        timeout_ms: 2_000,
    });
    let result = transcribe_best(&chain, b"RIFF-stub").expect("remote fallback");
    assert_eq!(result.text, "远端结果");
    assert_eq!(result.journal.len(), 1);
    assert!(
        result.journal[0].contains("asr engine sensevoice:")
            && result.journal[0].contains("fallback -> remote"),
        "{:?}",
        result.journal
    );
    server.join().expect("server thread");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn cancellation_between_engines_skips_the_remote_fallback() {
    let dir = temp_dir("cancel");
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    listener.set_nonblocking(true).expect("nonblocking");
    let chain = vec![
        EngineConfig::SenseVoice(SenseVoiceSettings {
            argv: vec!["sh".into(), "-c".into(), "exit 1".into()],
            model_dir: dir.clone(),
            num_threads: 1,
            timeout_ms: 2_000,
        }),
        EngineConfig::Remote(remote(&format!(
            "http://127.0.0.1:{port}/v1/audio/transcriptions"
        ))),
    ];
    let checks = AtomicU32::new(0);
    let cancelled = || checks.fetch_add(1, Ordering::Relaxed) >= 1;
    let err = transcribe_best_with_cancel(&chain, b"RIFF-stub", &cancelled).unwrap_err();
    assert_eq!(err, "cancelled");
    assert_eq!(checks.load(Ordering::Relaxed), 2);
    match listener.accept() {
        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {}
        other => panic!("remote must not be attempted after cancellation: {other:?}"),
    }
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn transcribe_best_reports_every_failure() {
    let dir = temp_dir("all-fail");
    let chain = vec![EngineConfig::SenseVoice(SenseVoiceSettings {
        argv: vec!["sh".into(), "-c".into(), "exit 7".into()],
        model_dir: dir.clone(),
        num_threads: 1,
        timeout_ms: 2_000,
    })];
    let err = transcribe_best(&chain, b"RIFF-stub").unwrap_err();
    assert!(err.contains("all ASR engines failed"), "{err}");
    assert!(
        err.contains("asr engine sensevoice: sensevoice exit 7"),
        "{err}"
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn empty_chain_is_a_clear_error() {
    let err = transcribe_best(&[], b"RIFF").unwrap_err();
    assert!(err.contains("no ASR engine available"), "{err}");
}

#[test]
fn gpu_device_nodes_are_detected() {
    let dir = temp_dir("gpu");
    assert!(!gpu_device_present(&dir));
    fs::write(dir.join("nvidia0"), b"").expect("device node");
    assert!(gpu_device_present(&dir));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn policy_strings_roundtrip() {
    for (policy, text) in [
        (EnginePolicy::Auto, "auto"),
        (EnginePolicy::SenseVoice, "sensevoice"),
        (EnginePolicy::Remote, "remote"),
    ] {
        assert_eq!(EnginePolicy::parse(text), Some(policy));
        assert_eq!(policy.as_str(), text);
    }
    assert_eq!(EnginePolicy::parse("gpu"), None);
}
