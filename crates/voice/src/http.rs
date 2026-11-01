//! Minimal HTTP/1.1 client over `std::net`: enough for one-shot POSTs to
//! a LAN ASR/LLM box. Plain `http://` only (no TLS; the node02 service is
//! trusted-LAN by design, see docs/NODE02-DEPLOY.md). Requests are pure
//! byte builders and response parsing is a pure function, so both are
//! unit-testable without a server.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

/// Parsed `http://host[:port]/path` URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Url {
    /// Host name or IP (no scheme, no port).
    pub host: String,
    /// Port, 80 when absent in the URL.
    pub port: u16,
    /// Request target including the query string, `/` when absent.
    pub path: String,
}

/// Parses an `http://` URL. `https://` is rejected: this client has no
/// TLS and failing loudly beats silently downgrading.
pub fn parse_url(url: &str) -> Result<Url, String> {
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| format!("http client speaks http:// only, got {url:?}"))?;
    let (authority, path) = match rest.find('/') {
        Some(idx) => (&rest[..idx], &rest[idx..]),
        None => (rest, "/"),
    };
    let authority = authority
        .rsplit_once('@')
        .map(|(_, host)| host)
        .unwrap_or(authority); // drop userinfo if present
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (
            host,
            port.parse::<u16>()
                .map_err(|_| format!("bad port in {url:?}"))?,
        ),
        None => (authority, 80),
    };
    if host.is_empty() {
        return Err(format!("missing host in {url:?}"));
    }
    Ok(Url {
        host: host.to_string(),
        port,
        path: path.to_string(),
    })
}

/// One POST request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Post {
    /// Target URL (http:// only).
    pub url: Url,
    /// `Content-Type` header value.
    pub content_type: String,
    /// Extra headers, e.g. `Authorization` (added after Host/Content-*).
    pub headers: Vec<(String, String)>,
    /// Body bytes.
    pub body: Vec<u8>,
    /// Connect/read/write timeout per operation, in milliseconds.
    pub timeout_ms: u64,
}

/// An HTTP response: status code plus decoded body (UTF-8, lossy).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    /// Status code, e.g. 200.
    pub status: u16,
    /// Body decoded lossily to UTF-8.
    pub body: String,
}

/// Sends `req` and waits for the full response. Errors carry a short
/// `what failed` prefix for the journal.
pub fn post(req: &Post) -> Result<Response, String> {
    let addr = (req.url.host.as_str(), req.url.port)
        .to_socket_addrs()
        .map_err(|err| format!("asr dns {}: {err}", req.url.host))?
        .next()
        .ok_or_else(|| format!("asr host {} resolved to nothing", req.url.host))?;
    let timeout = Duration::from_millis(req.timeout_ms);
    let mut stream = TcpStream::connect_timeout(&addr, timeout)
        .map_err(|err| format!("asr connect {addr}: {err}"))?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|err| format!("asr socket: {err}"))?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|err| format!("asr socket: {err}"))?;
    let request = build_request_bytes(&req.url, &req.content_type, &req.headers, &req.body);
    stream
        .write_all(&request)
        .map_err(|err| format!("asr send: {err}"))?;
    // Read to EOF (we always send Connection: close) but also stop as
    // soon as a complete response is parsable, so keep-alive servers
    // cannot stall us until the timeout.
    let mut raw = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        if let Ok(response) = parse_response(&raw) {
            return Ok(response);
        }
        match stream.read(&mut chunk) {
            Ok(0) => return parse_response(&raw),
            Ok(n) => raw.extend_from_slice(&chunk[..n]),
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(format!("asr recv: {err}")),
        }
    }
}

/// Request bytes: `Host`, `Content-Type`, `Content-Length`,
/// `Connection: close`, extra headers, then the body. Pure.
pub fn build_request_bytes(
    url: &Url,
    content_type: &str,
    headers: &[(String, String)],
    body: &[u8],
) -> Vec<u8> {
    let host = if url.port == 80 {
        url.host.clone()
    } else {
        format!("{}:{}", url.host, url.port)
    };
    let mut out = format!(
        "POST {} HTTP/1.1\r\nHost: {host}\r\nContent-Type: {content_type}\r\n\
         Content-Length: {}\r\nConnection: close\r\n",
        url.path,
        body.len()
    );
    for (name, value) in headers {
        out.push_str(&format!("{name}: {value}\r\n"));
    }
    out.push_str("\r\n");
    let mut bytes = out.into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

/// Parses one complete response: status line, headers, then a
/// `Content-Length` body or chunked transfer coding. Errors on partial
/// input so the read loop can keep reading.
pub fn parse_response(bytes: &[u8]) -> Result<Response, String> {
    let header_end = find_subslice(bytes, b"\r\n\r\n").ok_or("response headers incomplete")?;
    let head = std::str::from_utf8(&bytes[..header_end])
        .map_err(|_| "response headers not UTF-8".to_string())?;
    let mut lines = head.split("\r\n");
    let status_line = lines.next().ok_or("empty response")?;
    let status = status_line
        .split_ascii_whitespace()
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| format!("bad status line {status_line:?}"))?;
    let mut content_length = None;
    let mut chunked = false;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim();
        if name == "content-length" {
            content_length = value.parse::<usize>().ok();
        } else if name == "transfer-encoding" && value.to_ascii_lowercase().contains("chunked") {
            chunked = true;
        }
    }
    let raw_body = &bytes[header_end + 4..];
    let body_bytes = if chunked {
        decode_chunked(raw_body)?
    } else {
        let len = content_length.ok_or("response body incomplete (no length)")?;
        if raw_body.len() < len {
            return Err("response body incomplete".to_string());
        }
        raw_body[..len].to_vec()
    };
    Ok(Response {
        status,
        body: String::from_utf8_lossy(&body_bytes).to_string(),
    })
}

/// Decodes chunked transfer coding up to the terminating 0-chunk.
fn decode_chunked(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut rest = bytes;
    loop {
        let line_end = find_subslice(rest, b"\r\n").ok_or("chunked response incomplete")?;
        let size_text = std::str::from_utf8(&rest[..line_end])
            .map_err(|_| "bad chunk size".to_string())?
            .split(';')
            .next()
            .unwrap_or("")
            .trim();
        let size = usize::from_str_radix(size_text, 16)
            .map_err(|_| format!("bad chunk size {size_text:?}"))?;
        rest = &rest[line_end + 2..];
        if size == 0 {
            return Ok(out);
        }
        if rest.len() < size + 2 {
            return Err("chunked response incomplete".to_string());
        }
        out.extend_from_slice(&rest[..size]);
        rest = &rest[size + 2..]; // skip chunk + CRLF
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_urls_with_and_without_ports() {
        assert_eq!(
            parse_url("http://node02:8000/v1/audio/transcriptions").unwrap(),
            Url {
                host: "node02".into(),
                port: 8000,
                path: "/v1/audio/transcriptions".into()
            }
        );
        assert_eq!(
            parse_url("http://10.0.0.2/asr").unwrap(),
            Url {
                host: "10.0.0.2".into(),
                port: 80,
                path: "/asr".into()
            }
        );
        assert_eq!(
            parse_url("http://node02").unwrap().path,
            "/",
            "missing path defaults to /"
        );
        assert!(parse_url("https://node02:8000/v1").is_err());
        assert!(parse_url("node02:8000").is_err());
        assert!(parse_url("http://node02:notaport/").is_err());
        assert!(parse_url("http://:8000/").is_err());
    }

    #[test]
    fn builds_a_well_formed_post_request() {
        let url = parse_url("http://node02:8000/v1/audio/transcriptions").unwrap();
        let headers = vec![("Authorization".to_string(), "Bearer k".to_string())];
        let request = build_request_bytes(&url, "audio/wav", &headers, b"RIFF");
        let text = String::from_utf8(request.clone()).unwrap();
        assert!(text.starts_with(
            "POST /v1/audio/transcriptions HTTP/1.1\r\nHost: node02:8000\r\n\
             Content-Type: audio/wav\r\nContent-Length: 4\r\nConnection: close\r\n\
             Authorization: Bearer k\r\n\r\n"
        ));
        assert_eq!(&request[request.len() - 4..], b"RIFF");
    }

    #[test]
    fn parses_content_length_and_chunked_responses() {
        let direct = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello!!";
        assert_eq!(
            parse_response(direct).unwrap(),
            Response {
                status: 200,
                body: "hello".into()
            }
        );

        let chunked = b"HTTP/1.1 201 Created\r\nTransfer-Encoding: chunked\r\n\r\n\
                       3\r\nabc\r\n5\r\n def!\r\n0\r\n\r\n";
        assert_eq!(
            parse_response(chunked).unwrap(),
            Response {
                status: 201,
                body: "abc def!".into()
            }
        );
    }

    #[test]
    fn partial_responses_error_until_complete() {
        let partial = b"HTTP/1.1 200 OK\r\nContent-Length: 12\r\n\r\nshort";
        assert!(parse_response(partial).is_err());
        assert!(parse_response(b"HTTP/1.1 200 OK\r\n").is_err());
        assert!(parse_response(b"garbage\r\n\r\n").is_err());
    }

    #[test]
    fn post_to_a_refused_port_fails_fast_with_context() {
        // Port 1 on loopback: connection refused -> Err, not a panic.
        let req = Post {
            url: parse_url("http://127.0.0.1:1/v1/audio/transcriptions").unwrap(),
            content_type: "audio/wav".into(),
            headers: Vec::new(),
            body: Vec::new(),
            timeout_ms: 500,
        };
        let err = post(&req).unwrap_err();
        assert!(err.contains("connect"), "unexpected error: {err}");
    }
}
