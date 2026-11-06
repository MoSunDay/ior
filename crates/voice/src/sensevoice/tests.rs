use super::*;
use std::sync::atomic::{AtomicU32, Ordering};

static SEQ: AtomicU32 = AtomicU32::new(0);

fn temp_dir(tag: &str) -> PathBuf {
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "ior-sensevoice-test-{tag}-{}-{seq}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn settings(argv: &[&str], model_dir: PathBuf, timeout_ms: u64) -> SenseVoiceSettings {
    SenseVoiceSettings {
        argv: argv.iter().map(|arg| (*arg).to_string()).collect(),
        model_dir,
        num_threads: 2,
        timeout_ms,
    }
}

#[test]
fn probe_model_dir_requires_both_files() {
    let dir = temp_dir("probe");
    let explicit = dir.to_string_lossy().into_owned();
    assert_eq!(probe_model_dir(&explicit), None);
    fs::write(dir.join(MODEL_FILE), b"model").expect("model");
    assert_eq!(probe_model_dir(&explicit), None);
    fs::write(dir.join(TOKENS_FILE), b"tokens").expect("tokens");
    assert_eq!(probe_model_dir(&explicit), Some(dir.clone()));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn auto_model_dirs_prefer_ior_models_dir_then_xdg_then_home() {
    let dirs = auto_model_dirs_from(
        Some(PathBuf::from("/models")),
        Some(PathBuf::from("/data")),
        Some(PathBuf::from("/home/u")),
    );
    assert_eq!(
        dirs,
        vec![
            PathBuf::from("/models/sensevoice-small"),
            PathBuf::from("/data/ior/models/sensevoice-small"),
            PathBuf::from("/home/u/.local/share/ior/models/sensevoice-small"),
        ]
    );
    let deduped = auto_model_dirs_from(
        None,
        Some(PathBuf::from("/home/u/.local/share")),
        Some(PathBuf::from("/home/u")),
    );
    assert_eq!(
        deduped,
        vec![PathBuf::from(
            "/home/u/.local/share/ior/models/sensevoice-small"
        )]
    );
}

#[test]
fn resolve_command_splits_explicit_argv() {
    assert_eq!(
        resolve_command("  /opt/sherpa/bin/sherpa-onnx-offline --debug  "),
        Some(vec![
            "/opt/sherpa/bin/sherpa-onnx-offline".to_string(),
            "--debug".to_string(),
        ])
    );
    assert_eq!(resolve_command("   "), None);
}

#[test]
fn build_argv_is_byte_exact() {
    let settings = SenseVoiceSettings {
        argv: vec!["sherpa-onnx-offline".to_string()],
        model_dir: PathBuf::from("/models/sensevoice"),
        num_threads: 2,
        timeout_ms: 1_000,
    };
    assert_eq!(
        build_argv(&settings, Path::new("/tmp/audio.wav")),
        vec![
            "sherpa-onnx-offline",
            "--sense-voice-model=/models/sensevoice/model.int8.onnx",
            "--tokens=/models/sensevoice/tokens.txt",
            "--num-threads=2",
            "/tmp/audio.wav",
        ]
    );
}

#[test]
fn parse_transcript_reads_json_plain_and_text_prefix() {
    let output = "log line\n{\"lang\": \"<zh>\", \"text\": \"你好，世界\", \"x\": 1}\n";
    assert_eq!(parse_transcript(output), Some("你好，世界".to_string()));
    assert_eq!(
        parse_transcript("older\nText: 你好\n\n"),
        Some("你好".to_string())
    );
    assert_eq!(
        parse_transcript("plain transcript"),
        Some("plain transcript".to_string())
    );
    assert_eq!(parse_transcript(" \n\t\n"), None);
}

#[test]
fn transcribe_runs_a_stub_command() {
    let dir = temp_dir("stub");
    let settings = settings(&["sh", "-c", "echo 你好"], dir.clone(), 2_000);
    assert_eq!(transcribe(&settings, b"RIFF-stub").unwrap(), "你好");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn transcribe_reports_exit_status_and_stderr() {
    let dir = temp_dir("fail");
    let settings = settings(&["sh", "-c", "echo boom >&2; exit 3"], dir.clone(), 2_000);
    let err = transcribe(&settings, b"RIFF-stub").unwrap_err();
    assert!(err.contains("sensevoice exit 3"), "{err}");
    assert!(err.contains("boom"), "{err}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn transcribe_kills_a_hung_command() {
    let dir = temp_dir("timeout");
    let settings = settings(&["sh", "-c", "exec sleep 5"], dir.clone(), 100);
    let started = Instant::now();
    let err = transcribe(&settings, b"RIFF-stub").unwrap_err();
    assert!(err.contains("timed out after 100ms"), "{err}");
    assert!(started.elapsed() < Duration::from_secs(2));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn timeout_does_not_wait_for_a_pipe_holding_grandchild() {
    let dir = temp_dir("timeout-child");
    let settings = settings(&["sh", "-c", "sleep 5; true"], dir.clone(), 100);
    let started = Instant::now();
    let err = transcribe(&settings, b"RIFF-stub").unwrap_err();
    assert!(err.contains("timed out after 100ms"), "{err}");
    assert!(started.elapsed() < Duration::from_secs(2));
    let _ = fs::remove_dir_all(dir);
}
