//! Logos Storage (Codex) client: streaming put/get of region snapshots.
//!
//! The prize's flow is *download from Geofabrik → verify → store in Logos
//! Storage → register the CID on LEZ*. Map data is public, so nothing here
//! encrypts — the integrity story is Geofabrik's published MD5 (recorded
//! on-chain next to the CID), not secrecy.
//!
//! Region PBFs reach multiple GB, so both directions stream:
//!
//! - **put** — `POST /api/storage/v1/data` with the file's bytes wrapped in
//!   a `ReaderStream` (constant memory; `Content-Length` set from the file
//!   metadata). The response body is the CID.
//! - **get** — `GET /api/storage/v1/data/{cid}/network/stream`, consumed
//!   chunk-by-chunk straight to disk while a streaming MD5 runs alongside
//!   (so a fetched snapshot can be re-verified against the chain's checksum
//!   without a second pass).
//!
//! Transient transport failures (connect/timeout/5xx/429; 404 on get, which
//! can be propagation delay) retry with backoff via [`crate::retry`]; other
//! 4xx are fatal. Request shapes proven against a real Codex node in the
//! vault build.
//!
//! [`Storage`] is a trait with an in-memory implementation
//! ([`MemoryStorage`]) so the SDK facade, the CLI, and tests run hermetically
//!; [`CodexStorage`] talks to a real node.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, Result};
use bytes::Bytes;
use futures::StreamExt;
use tokio::io::AsyncWriteExt;
use tokio_util::io::ReaderStream;

use crate::geofabrik::DownloadOutcome;
use crate::retry::{retry_transient, RetryErr};
use crate::verify::Md5Stream;

/// What came back from storing bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredObject {
    /// The Logos Storage CID (content address / locator).
    pub cid: String,
    /// Size stored, bytes.
    pub bytes: u64,
}

/// A Logos Storage backend. All operations stream where the backend supports
/// it; the default `put_file` / `get_to_file` buffer (fine for the in-memory
/// backend), and [`CodexStorage`] overrides both with true streaming.
///
/// Uses native async-fn-in-trait deliberately: the SDK awaits these directly
/// (never `tokio::spawn`s them, never dyn-dispatches), so `Send` futures are
/// not required.
#[allow(async_fn_in_trait)]
pub trait Storage: Send + Sync {
    /// Store an in-memory buffer.
    async fn put_bytes(&self, data: Bytes) -> Result<StoredObject>;
    /// Store a file (streaming where the backend supports it).
    async fn put_file(&self, path: &Path) -> Result<StoredObject> {
        let data = tokio::fs::read(path)
            .await
            .with_context(|| format!("reading {}", path.display()))?;
        self.put_bytes(data.into()).await
    }
    /// Fetch an object fully into memory.
    async fn get_bytes(&self, cid: &str) -> Result<Bytes>;
    /// Fetch an object to a file, MD5-ing as it lands.
    async fn get_to_file(&self, cid: &str, dest: &Path) -> Result<DownloadOutcome> {
        let data = self.get_bytes(cid).await?;
        let mut hasher = Md5Stream::new();
        hasher.update(&data);
        tokio::fs::write(dest, &data)
            .await
            .with_context(|| format!("writing {}", dest.display()))?;
        Ok(DownloadOutcome {
            path: dest.to_path_buf(),
            bytes: hasher.len(),
            md5: hasher.finalize(),
        })
    }
}

/// A real Logos Storage / Codex node over HTTP.
#[derive(Debug, Clone)]
pub struct CodexStorage {
    http: reqwest::Client,
    base: String,
}

impl CodexStorage {
    /// `base` is the Codex REST endpoint, e.g. `http://127.0.0.1:8080`.
    pub fn new(base: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            base: {
                let b: String = base.into();
                b.trim_end_matches('/').to_string()
            },
        }
    }

    /// The REST endpoint this client talks to.
    pub fn base(&self) -> &str {
        &self.base
    }
}

impl Storage for CodexStorage {
    async fn put_bytes(&self, data: Bytes) -> Result<StoredObject> {
        let len = data.len() as u64;
        let cid = retry_transient(|| {
            let body = data.clone();
            async move {
                let resp = self
                    .http
                    .post(format!("{}/api/storage/v1/data", self.base))
                    .header("Content-Type", "application/octet-stream")
                    .header("Content-Length", len.to_string())
                    .body(body)
                    .send()
                    .await
                    .map_err(|e| {
                        RetryErr::Transient(anyhow::anyhow!("POST /api/storage/v1/data: {e}"))
                    })?;
                let status = resp.status();
                if !status.is_success() {
                    let err = anyhow::anyhow!("codex put failed: {status}");
                    return Err(if crate::retry::status_is_transient(status.as_u16()) {
                        RetryErr::Transient(err)
                    } else {
                        RetryErr::Fatal(err)
                    });
                }
                resp.text()
                    .await
                    .map(|t| t.trim().trim_matches('"').to_owned())
                    .map_err(|e| {
                        RetryErr::Transient(anyhow::anyhow!("reading codex put body: {e}"))
                    })
            }
        })
        .await?;
        Ok(StoredObject { cid, bytes: len })
    }

    async fn put_file(&self, path: &Path) -> Result<StoredObject> {
        let len = tokio::fs::metadata(path)
            .await
            .with_context(|| format!("stat {}", path.display()))?
            .len();

        // A retried upload re-opens the file and restarts the body from the
        // beginning, so the stream is built inside the retry closure.
        let cid = retry_transient(|| async {
            let file = tokio::fs::File::open(path).await.map_err(|e| {
                RetryErr::Fatal(anyhow::anyhow!("open {}: {e}", path.display()))
            })?;
            let stream = ReaderStream::with_capacity(file, 1 << 20);
            let resp = self
                .http
                .post(format!("{}/api/storage/v1/data", self.base))
                .header("Content-Type", "application/octet-stream")
                .header("Content-Length", len.to_string())
                .body(reqwest::Body::wrap_stream(stream))
                .send()
                .await
                .map_err(|e| {
                    RetryErr::Transient(anyhow::anyhow!("POST /api/storage/v1/data: {e}"))
                })?;
            let status = resp.status();
            if !status.is_success() {
                let err = anyhow::anyhow!("codex put failed: {status}");
                return Err(if crate::retry::status_is_transient(status.as_u16()) {
                    RetryErr::Transient(err)
                } else {
                    RetryErr::Fatal(err)
                });
            }
            resp.text()
                .await
                .map(|t| t.trim().trim_matches('"').to_owned())
                .map_err(|e| {
                    RetryErr::Transient(anyhow::anyhow!("reading codex put body: {e}"))
                })
        })
        .await?;
        Ok(StoredObject { cid, bytes: len })
    }

    async fn get_bytes(&self, cid: &str) -> Result<Bytes> {
        // A 404 can be propagation delay, so it is retried too.
        retry_transient(|| async {
            let resp = self
                .http
                .get(format!(
                    "{}/api/storage/v1/data/{cid}/network/stream",
                    self.base
                ))
                .send()
                .await
                .map_err(|e| RetryErr::Transient(anyhow::anyhow!("GET codex data: {e}")))?;
            let status = resp.status();
            if !status.is_success() {
                let transient = crate::retry::status_is_transient(status.as_u16())
                    || status.as_u16() == 404;
                let err = anyhow::anyhow!("codex get failed: {status}");
                return Err(if transient {
                    RetryErr::Transient(err)
                } else {
                    RetryErr::Fatal(err)
                });
            }
            resp.bytes()
                .await
                .map_err(|e| RetryErr::Transient(anyhow::anyhow!("reading object bytes: {e}")))
        })
        .await
    }

    async fn get_to_file(&self, cid: &str, dest: &Path) -> Result<DownloadOutcome> {
        let url = format!("{}/api/storage/v1/data/{cid}/network/stream", self.base);
        retry_transient(|| async {
            let resp = self
                .http
                .get(&url)
                .send()
                .await
                .map_err(|e| RetryErr::Transient(anyhow::anyhow!("GET {url}: {e}")))?;
            let status = resp.status();
            if !status.is_success() {
                let transient =
                    crate::retry::status_is_transient(status.as_u16()) || status.as_u16() == 404;
                let err = anyhow::anyhow!("GET {url}: {status}");
                return Err(if transient {
                    RetryErr::Transient(err)
                } else {
                    RetryErr::Fatal(err)
                });
            }
            let mut hasher = Md5Stream::new();
            let file = tokio::fs::File::create(dest)
                .await
                .map_err(|e| RetryErr::Fatal(anyhow::anyhow!("create {}: {e}", dest.display())))?;
            let mut writer = tokio::io::BufWriter::with_capacity(1 << 20, file);
            let mut stream = resp.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let chunk: Bytes =
                    chunk.map_err(|e| RetryErr::Transient(anyhow::anyhow!("stream {url}: {e}")))?;
                hasher.update(&chunk);
                writer
                    .write_all(&chunk)
                    .await
                    .map_err(|e| {
                        RetryErr::Fatal(anyhow::anyhow!("write {}: {e}", dest.display()))
                    })?;
            }
            writer
                .flush()
                .await
                .map_err(|e| RetryErr::Fatal(anyhow::anyhow!("flush {}: {e}", dest.display())))?;
            Ok(DownloadOutcome {
                path: dest.to_path_buf(),
                bytes: hasher.len(),
                md5: hasher.finalize(),
            })
        })
        .await
    }
}

/// In-memory backend for tests, examples, and offline demos.
#[derive(Default, Clone)]
pub struct MemoryStorage {
    inner: std::sync::Arc<Mutex<HashMap<String, Bytes>>>,
}

impl MemoryStorage {
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of stored objects.
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().len()
    }

    /// True when nothing is stored.
    pub fn is_empty(&self) -> bool {
        self.inner.lock().unwrap().is_empty()
    }
}

impl Storage for MemoryStorage {
    async fn put_bytes(&self, data: Bytes) -> Result<StoredObject> {
        use sha2::{Digest, Sha256};
        let cid = hex::encode(Sha256::digest(&data));
        self.inner
            .lock()
            .unwrap()
            .insert(cid.clone(), data.clone());
        Ok(StoredObject {
            cid,
            bytes: data.len() as u64,
        })
    }

    async fn get_bytes(&self, cid: &str) -> Result<Bytes> {
        self.inner
            .lock()
            .unwrap()
            .get(cid)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("not found: {cid}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn memory_roundtrip() {
        let s = MemoryStorage::new();
        assert!(s.is_empty());
        let stored = s.put_bytes(Bytes::from_static(b"osm pbf bytes")).await.unwrap();
        assert_eq!(s.len(), 1);
        let back = s.get_bytes(&stored.cid).await.unwrap();
        assert_eq!(&back[..], b"osm pbf bytes");
        assert!(s.get_bytes("bogus").await.is_err());
    }

    #[tokio::test]
    async fn memory_get_to_file_hashes() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out.pbf");
        let s = MemoryStorage::new();
        let stored = s
            .put_bytes(Bytes::from_static(b"the quick brown fox"))
            .await
            .unwrap();
        let out = s.get_to_file(&stored.cid, &dest).await.unwrap();
        assert_eq!(out.bytes, 19);
        assert_eq!(hex::encode(out.md5), hex::encode(crate::verify::md5_bytes(b"the quick brown fox")));
        assert_eq!(std::fs::read(&dest).unwrap(), b"the quick brown fox");
    }

    #[tokio::test]
    async fn memory_put_file_default_impl() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("in.pbf");
        std::fs::write(&src, b"file body").unwrap();
        let s = MemoryStorage::new();
        let stored = s.put_file(&src).await.unwrap();
        assert_eq!(stored.bytes, 9);
        assert_eq!(&s.get_bytes(&stored.cid).await.unwrap()[..], b"file body");
    }

    #[test]
    fn codex_urls_well_formed() {
        let c = CodexStorage::new("http://127.0.0.1:8080/");
        assert_eq!(c.base(), "http://127.0.0.1:8080");
    }
}
