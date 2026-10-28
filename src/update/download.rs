//! Fetching a release asset, with the sha-256 computed as it lands.
//!
//! Calls are blocking; the update service runs them inside `spawn_blocking`.

use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use ring::digest::{Context, SHA256};

use crate::common::error::{Error, Result};

/// How long a download may take in total. The largest asset is tens of
/// megabytes, so a slow link still fits.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(600);

/// Download `url` to `dest`, returning the sha-256 of what was written.
///
/// `progress` receives `(written, total)`, where `total` is 0 when the response
/// carries no length. The caller verifies the returned digest before anything
/// treats the file as installable.
pub fn to_file(url: &str, dest: &Path, progress: &mut dyn FnMut(u64, u64)) -> Result<String> {
    ensure_secure(url)?;
    let mut response = client()
        .get(url)
        .call()
        .map_err(|error| Error::Other(format!("download: {error}")))?;
    check_status(&mut response, url)?;
    let total = response
        .headers()
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);

    let mut file = File::create(dest)?;
    let mut hasher = Context::new(&SHA256);
    let mut buffer = vec![0u8; 64 * 1024];
    let mut written = 0u64;
    let mut reader = response.body_mut().as_reader();
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|error| Error::Other(format!("download: {error}")))?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])?;
        hasher.update(&buffer[..read]);
        written += read as u64;
        progress(written, total);
    }
    file.flush()?;
    Ok(hex(&hasher.finish()))
}

/// Read a small text asset, such as `SHA256SUMS`.
pub fn text(url: &str) -> Result<String> {
    ensure_secure(url)?;
    let mut response = client()
        .get(url)
        .call()
        .map_err(|error| Error::Other(format!("download: {error}")))?;
    check_status(&mut response, url)?;
    response
        .body_mut()
        .read_to_string()
        .map_err(|error| Error::Other(format!("download: {error}")))
}

/// Refuse a non-success answer, naming what the server said.
///
/// An asset request that a proxy or GitHub rejects used to be written to disk
/// as if it were the file, and only the checksum caught it later.
fn check_status(response: &mut ureq::http::Response<ureq::Body>, url: &str) -> Result<()> {
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    let body = response
        .body_mut()
        .with_config()
        .limit(8 * 1024)
        .read_to_string()
        .unwrap_or_default();
    Err(Error::Other(format!(
        "download {url}: {}",
        crate::update::github::describe_failure(status.as_u16(), response.headers(), &body)
    )))
}

fn hex(digest: &ring::digest::Digest) -> String {
    digest
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Redirects are followed: GitHub answers an asset request with a redirect to
/// its object storage.
fn client() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(DOWNLOAD_TIMEOUT))
        .http_status_as_error(false)
        .build()
        .new_agent()
}

/// Refuse a download that would cross the network in clear text.
///
/// The URL comes from the release feed, which is HTTPS, so a plaintext asset URL
/// means the feed was tampered with or is not GitHub. Loopback is the exception:
/// that is the local server the tests use, and what it serves still has to match
/// a published checksum before anything installs it.
fn ensure_secure(url: &str) -> Result<()> {
    let uri = url
        .parse::<ureq::http::Uri>()
        .map_err(|error| Error::Other(format!("download: {error}")))?;
    let scheme = uri.scheme_str().unwrap_or_default();
    if scheme == "https" {
        return Ok(());
    }
    let loopback = uri.host().is_some_and(|host| {
        host == "localhost"
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|address| address.is_loopback())
    });
    if scheme == "http" && loopback {
        return Ok(());
    }
    Err(Error::Other(format!(
        "refusing to download a release asset over {scheme}"
    )))
}
