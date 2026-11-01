//! Transcription client for the node02 service (Qwen-first) speaking the
//! OpenAI-compatible `/v1/audio/transcriptions` API. With
//! `response_format=text` the response body IS the transcript, so no
//! JSON parsing is needed on the happy path.

use crate::http::{self, Post, Url};

/// Connection settings for one transcription endpoint; mirrors
/// `ior_config::LlmAsr` (the voice crate stays dependency-free, so the
/// addon copies the fields across).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsrSettings {
    /// `http://host:port/v1/audio/transcriptions`.
    pub endpoint: String,
    /// Model id, e.g. `Qwen/Qwen3-ASR-4B`.
    pub model: String,
    /// Name of the environment variable holding the API key (unset or
    /// empty variable = no Authorization header, for keyless LAN vLLM).
    pub api_key_env: String,
    /// Language hint, e.g. `zh`.
    pub language: String,
    /// Network timeout per request, milliseconds.
    pub timeout_ms: u64,
}

/// POSTs `wav` and returns the transcript (trimmed). Errors are
/// journal-ready one-liners with an `asr`/`asr http <status>` prefix.
pub fn transcribe(settings: &AsrSettings, wav: &[u8]) -> Result<String, String> {
    let url: Url = http::parse_url(&settings.endpoint)?;
    let boundary = make_boundary();
    let body = multipart_body(
        &boundary,
        &[
            ("model".to_string(), settings.model.clone()),
            ("language".to_string(), settings.language.clone()),
            ("response_format".to_string(), "text".to_string()),
        ],
        "file",
        "audio.wav",
        "audio/wav",
        wav,
    );
    let response = http::post(&Post {
        url,
        content_type: format!("multipart/form-data; boundary={boundary}"),
        headers: auth_header(&settings.api_key_env),
        body,
        timeout_ms: settings.timeout_ms,
    })?;
    if response.status != 200 {
        return Err(format!(
            "asr http {}: {}",
            response.status,
            response.body.trim()
        ));
    }
    let text = response.body.trim();
    if text.is_empty() {
        return Err("asr returned an empty transcript".to_string());
    }
    Ok(text.to_string())
}

/// `Authorization: Bearer <key>` from the variable named by
/// `api_key_env`, or nothing when unset/empty (keys never enter the
/// config file).
pub fn auth_header(api_key_env: &str) -> Vec<(String, String)> {
    match std::env::var(api_key_env) {
        Ok(key) if !key.trim().is_empty() => {
            vec![(
                "Authorization".to_string(),
                format!("Bearer {}", key.trim()),
            )]
        }
        _ => Vec::new(),
    }
}

/// Boundary from time + a process-unique counter (no external crates).
pub fn make_boundary() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    format!("ior-voice-{nanos:x}-{seq:x}")
}

/// Builds a `multipart/form-data` body: text `fields` first, then one
/// file part. Pure, so byte-exact tests cover the wire format.
pub fn multipart_body(
    boundary: &str,
    fields: &[(String, String)],
    file_field: &str,
    filename: &str,
    content_type: &str,
    data: &[u8],
) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, value) in fields {
        out.extend_from_slice(
            format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n")
                .as_bytes(),
        );
        out.extend_from_slice(value.as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"{file_field}\"; \
             filename=\"{filename}\"\r\nContent-Type: {content_type}\r\n\r\n"
        )
        .as_bytes(),
    );
    out.extend_from_slice(data);
    out.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(endpoint: &str) -> AsrSettings {
        AsrSettings {
            endpoint: endpoint.to_string(),
            model: "Qwen/Qwen3-ASR-4B".to_string(),
            api_key_env: "IOR_ASR_TEST_KEY".to_string(),
            language: "zh".to_string(),
            timeout_ms: 500,
        }
    }

    #[test]
    fn multipart_body_matches_the_wire_format() {
        let body = multipart_body(
            "BND",
            &[("model".to_string(), "m".to_string())],
            "file",
            "audio.wav",
            "audio/wav",
            b"RIFF",
        );
        let text = String::from_utf8_lossy(&body);
        assert_eq!(
            text,
            "--BND\r\n\
             Content-Disposition: form-data; name=\"model\"\r\n\r\nm\r\n\
             --BND\r\n\
             Content-Disposition: form-data; name=\"file\"; filename=\"audio.wav\"\r\n\
             Content-Type: audio/wav\r\n\r\n\
             RIFF\r\n\
             --BND--\r\n"
        );
    }

    #[test]
    fn transcribe_reads_a_text_response_from_a_local_stub_server() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().expect("accept");
            // The whole request is one write on loopback; one read gets it.
            let mut got = vec![0u8; 65536];
            let n = sock.read(&mut got).expect("read");
            let text = String::from_utf8_lossy(&got[..n]).to_string();
            assert!(
                text.starts_with("POST /v1/audio/transcriptions HTTP/1.1"),
                "request line: {}",
                text.lines().next().unwrap_or("")
            );
            assert!(text.contains("multipart/form-data; boundary="));
            assert!(text.contains("name=\"model\""), "model field");
            assert!(text.contains("RIFF-stub"), "audio payload");
            let transcript = "你好，普通话测试。";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}",
                transcript.len(),
                transcript
            );
            sock.write_all(response.as_bytes()).expect("write");
        });
        let result = transcribe(
            &settings(&format!("http://127.0.0.1:{port}/v1/audio/transcriptions")),
            b"RIFF-stub",
        );
        assert_eq!(result.unwrap(), "你好，普通话测试。");
        server.join().expect("server thread");
    }

    #[test]
    fn transcribe_maps_endpoint_and_http_errors() {
        let err = transcribe(
            &settings("http://127.0.0.1:1/v1/audio/transcriptions"),
            b"RIFF",
        )
        .unwrap_err();
        assert!(err.contains("connect"), "{err}");

        let err =
            transcribe(&settings("https://node02/v1/audio/transcriptions"), b"RIFF").unwrap_err();
        assert!(err.contains("http:// only"), "{err}");
    }

    #[test]
    fn auth_header_follows_the_environment() {
        std::env::set_var("IOR_ASR_TEST_KEY", " sekret ");
        assert_eq!(
            auth_header("IOR_ASR_TEST_KEY"),
            vec![("Authorization".to_string(), "Bearer sekret".to_string())]
        );
        std::env::remove_var("IOR_ASR_TEST_KEY");
        assert!(auth_header("IOR_ASR_TEST_KEY").is_empty());
        assert!(auth_header("IOR_ASR_NEVER_SET").is_empty());
    }
}
