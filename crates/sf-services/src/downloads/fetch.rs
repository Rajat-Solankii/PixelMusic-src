//! A stream URL saved to a file the same way the player streams it (`sf_player::engine::youtube::download`):
//! `Range: bytes=a-b` requests of a few MiB one after another, with the stream's User-Agent. Bytes are appended
//! to a `.part` file, so a retry after a network failure goes on from where the file ends. 403 / 410 mean the
//! address expired: the caller resolves the stream again.
use std::path::Path;

use reqwest::StatusCode;
use reqwest::header::{CONTENT_RANGE, RANGE, USER_AGENT};
use tokio::io::AsyncWriteExt;

/// How much one request asks for (as the player does: YouTube throttles a whole-file request).
pub const CHUNK_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FetchError {
    /// 403 / 410: resolve the stream again
    Expired,
    /// network failure, unexpected answer or a disk error (worth another try)
    Failed(String),
}

/// What to fetch.
pub struct Target<'a> {
    pub url: &'a str,
    /// sent as `User-Agent`; empty — the client's own
    pub user_agent: &'a str,
    pub content_length: Option<u64>,
    pub chunk_bytes: u64,
}

/// Download `target` into `part` (appending to what is already there). `progress(downloaded, total)` is called
/// after every piece. Returns the file's size.
pub async fn to_file(
    client: &reqwest::Client,
    target: &Target<'_>,
    part: &Path,
    progress: &(dyn Fn(u64, Option<u64>) + Send + Sync),
) -> Result<u64, FetchError> {
    let disk = |error: std::io::Error| FetchError::Failed(format!("{}: {error}", part.display()));
    let mut file = tokio::fs::OpenOptions::new().create(true).append(true).open(part).await.map_err(disk)?;
    let mut offset = file.metadata().await.map_err(disk)?.len();
    let mut total = target.content_length.filter(|length| *length > 0);
    let chunk = target.chunk_bytes.max(1);
    if total.is_some_and(|total| offset > total) {
        file.set_len(0).await.map_err(disk)?;
        offset = 0;
    }
    progress(offset, total);
    loop {
        if total.is_some_and(|total| offset >= total) {
            break;
        }
        let mut end = offset + chunk - 1;
        if let Some(total) = total {
            end = end.min(total - 1);
        }
        let mut request = client.get(target.url).header(RANGE, format!("bytes={offset}-{end}"));
        if !target.user_agent.is_empty() {
            request = request.header(USER_AGENT, target.user_agent);
        }
        let mut response = request.send().await.map_err(|error| FetchError::Failed(error.to_string()))?;
        let status = response.status();
        let whole = match status {
            StatusCode::FORBIDDEN | StatusCode::GONE => return Err(FetchError::Expired),
            StatusCode::RANGE_NOT_SATISFIABLE if offset > 0 => break,
            StatusCode::OK => true,
            StatusCode::PARTIAL_CONTENT => false,
            other => return Err(FetchError::Failed(format!("HTTP {}", other.as_u16()))),
        };
        if whole {
            // the server ignored the range: this answer is the whole file
            if offset > 0 {
                file.set_len(0).await.map_err(disk)?;
                offset = 0;
            }
            total = response.content_length().or(total);
        } else if let Some(size) = response.headers().get(CONTENT_RANGE).and_then(|value| value.to_str().ok()).and_then(range_total) {
            total = Some(size);
        }
        let mut got = 0u64;
        while let Some(bytes) = response.chunk().await.map_err(|error| FetchError::Failed(error.to_string()))? {
            file.write_all(&bytes).await.map_err(disk)?;
            got += bytes.len() as u64;
            offset += bytes.len() as u64;
            progress(offset, total);
        }
        if whole {
            break;
        }
        if got == 0 {
            if total.is_some_and(|total| offset < total) {
                return Err(FetchError::Failed("empty answer".into()));
            }
            break;
        }
        // without a known size, a short piece is the end of the file
        if total.is_none() && got < end - offset.saturating_sub(got) + 1 {
            break;
        }
    }
    file.flush().await.map_err(disk)?;
    if let Some(total) = total
        && offset < total
    {
        return Err(FetchError::Failed(format!("got {offset} of {total} bytes")));
    }
    Ok(offset)
}

/// The size in `Content-Range: bytes 0-99/1234` (`*` — unknown).
fn range_total(value: &str) -> Option<u64> {
    value.rsplit_once('/').and_then(|(_, total)| total.trim().parse().ok())
}

#[cfg(test)]
mod tests {
    #[test]
    fn content_range_totals() {
        assert_eq!(super::range_total("bytes 0-99/1234"), Some(1234));
        assert_eq!(super::range_total("bytes 0-99/*"), None);
    }
}
