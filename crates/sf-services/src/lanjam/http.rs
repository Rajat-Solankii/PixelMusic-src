//! The bare HTTP/1.1 both Kotlin apps speak on the LAN: one request per connection, parameters in the query
//! string, a JSON body back with `Content-Length` and `Connection: close`. Nothing else is needed, so there is no
//! HTTP library here.
use std::net::SocketAddr;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

/// Request head limit: a long `/control` (an added track's fields) is a few KB.
const MAX_HEAD: usize = 64 * 1024;
/// Response limit: a big queue in `/sync`.
const MAX_BODY: usize = 8 * 1024 * 1024;

/// `application/x-www-form-urlencoded` value: unreserved bytes as is, the rest as %XX (a space too, so
/// decoders that don't turn `+` into a space read it the same).
pub fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'*') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Java `URLDecoder`: `+` is a space, %XX a byte; a broken escape is kept as text.
pub fn decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' => {
                let hex = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok());
                match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    Some(b) => {
                        out.push(b);
                        i += 2;
                    }
                    None => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn parse_query(raw: &str) -> Vec<(String, String)> {
    raw.split('&')
        .filter(|p| !p.is_empty())
        .map(|p| match p.split_once('=') {
            Some((k, v)) => (decode(k), decode(v)),
            None => (decode(p), String::new()),
        })
        .collect()
}

pub fn build_query(params: &[(String, String)]) -> String {
    params.iter().map(|(k, v)| format!("{}={}", encode(k), encode(v))).collect::<Vec<_>>().join("&")
}

pub struct Request {
    pub method: String,
    pub path: String,
    pub query: Vec<(String, String)>,
}

impl Request {
    pub fn param(&self, key: &str) -> Option<&str> {
        self.query.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }
}

fn find_head_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4).or_else(|| buf.windows(2).position(|w| w == b"\n\n").map(|p| p + 2))
}

/// Reads the request line and headers (and a body, if one is declared, so closing the socket doesn't reset it).
pub async fn read_request(stream: &mut TcpStream, limit: Duration) -> Option<Request> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = timeout(limit, async {
        loop {
            if let Some(end) = find_head_end(&buf) {
                return Some(end);
            }
            if buf.len() > MAX_HEAD {
                return None;
            }
            let n = stream.read(&mut chunk).await.ok()?;
            if n == 0 {
                return find_head_end(&buf).or((!buf.is_empty()).then_some(buf.len()));
            }
            buf.extend_from_slice(&chunk[..n]);
        }
    })
    .await
    .ok()??;
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.lines();
    let mut parts = lines.next()?.split(' ');
    let method = parts.next()?.to_owned();
    let target = parts.next()?;
    let length = lines
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.trim().eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.trim().parse::<usize>().ok())
        .unwrap_or(0)
        .min(MAX_HEAD);
    let mut have = buf.len() - head_end;
    while have < length {
        match timeout(Duration::from_secs(2), stream.read(&mut chunk)).await {
            Ok(Ok(n)) if n > 0 => have += n,
            _ => break,
        }
    }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    Some(Request { method, path: path.to_owned(), query: parse_query(query) })
}

pub async fn write_response(stream: &mut TcpStream, status: u16, body: &str) {
    let reason = match status {
        200 => "OK",
        403 => "Forbidden",
        404 => "Not Found",
        _ => "Error",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json; charset=UTF-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes()).await;
    let _ = stream.write_all(body.as_bytes()).await;
    let _ = stream.flush().await;
    let _ = stream.shutdown().await;
}

#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    #[error("connect: {0}")]
    Connect(String),
    #[error("timed out")]
    Timeout,
    #[error("io: {0}")]
    Io(String),
    #[error("bad response")]
    BadResponse,
}

/// One request; returns (status, body). `connect` bounds the TCP connect, `read` the wait for the whole reply.
pub async fn request(
    addr: SocketAddr,
    method: &str,
    path_and_query: &str,
    connect: Duration,
    read: Duration,
) -> Result<(u16, String), HttpError> {
    let mut stream = match timeout(connect, TcpStream::connect(addr)).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => return Err(HttpError::Connect(e.to_string())),
        Err(_) => return Err(HttpError::Timeout),
    };
    let _ = stream.set_nodelay(true);
    let req = format!(
        "{method} {path_and_query} HTTP/1.1\r\nHost: {addr}\r\nUser-Agent: Materialfy\r\nAccept: application/json\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(req.as_bytes()).await.map_err(|e| HttpError::Io(e.to_string()))?;
    let mut buf = Vec::new();
    let read_all = async {
        let mut chunk = [0u8; 16 * 1024];
        loop {
            let n = stream.read(&mut chunk).await?;
            if n == 0 {
                return Ok::<(), std::io::Error>(());
            }
            buf.extend_from_slice(&chunk[..n]);
            if buf.len() > MAX_BODY {
                return Err(std::io::Error::other("response too large"));
            }
            // Stop at Content-Length rather than waiting for the close.
            if let Some(end) = find_head_end(&buf) {
                let head = String::from_utf8_lossy(&buf[..end]);
                let length = head
                    .lines()
                    .filter_map(|l| l.split_once(':'))
                    .find(|(k, _)| k.trim().eq_ignore_ascii_case("content-length"))
                    .and_then(|(_, v)| v.trim().parse::<usize>().ok());
                if length.is_some_and(|len| buf.len() >= end + len) {
                    return Ok(());
                }
            }
        }
    };
    match timeout(read, read_all).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(HttpError::Io(e.to_string())),
        Err(_) => return Err(HttpError::Timeout),
    }
    let end = find_head_end(&buf).ok_or(HttpError::BadResponse)?;
    let head = String::from_utf8_lossy(&buf[..end]).into_owned();
    let status = head.lines().next().and_then(|l| l.split(' ').nth(1)).and_then(|s| s.parse::<u16>().ok()).ok_or(HttpError::BadResponse)?;
    let length = head
        .lines()
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.trim().eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.trim().parse::<usize>().ok());
    let body = &buf[end..];
    let body = match length {
        Some(len) => &body[..len.min(body.len())],
        None => body,
    };
    Ok((status, String::from_utf8_lossy(body).into_owned()))
}
