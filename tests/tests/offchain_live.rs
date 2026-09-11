//! Off-chain end-to-end: the full Geofabrik→Storage→consume lifecycle.
//!
//! Two tests share a local **fixture Geofabrik server** (a real HTTP server
//! speaking the same layout as `download.geofabrik.de`: the
//! `index-v1-nogeom.json`, `<region>-latest.osm.pbf.md5` with the
//! `X-Derived-From` version signal, and the PBF itself):
//!
//! - `hermetic_lifecycle_on_fixture` (always runs, no network beyond
//!   loopback): discover → host (download → MD5 verify → store) →
//!   registration tx → fetch → verify → import → update check → tamper
//!   rejection, against the in-memory storage backend.
//! - `offchain_lifecycle_on_real_codex` (ignored; needs the real Logos
//!   Storage node): the same flow against a real Codex node, proving the
//!   streaming put/get request shapes and CID readback over the wire.
//!
//! Codex start (digest-pinned + entrypoint override — the image's default
//! config is broken upstream: CMD ["codex"] but the binary is
//! /usr/local/bin/storage):
//! ```text
//! docker run -d --name osm-codex -p 8080:8080 -e NAT_IP_AUTO=false \
//!   --entrypoint /usr/local/bin/storage \
//!   codexstorage/nim-codex@sha256:25d9409b591da37200896e68fd9e4c591566f1ad0a80c0633c6cd5011ae56edb \
//!   --api-bindaddr=0.0.0.0 --api-port=8080
//! cargo test -p osm-integration-tests --test offchain_live -- --ignored --nocapture
//! ```
//! Endpoint override with OSM_CODEX_URL.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use logos_osm::regions::{by_path, Level, REGIONS};
use logos_osm::registry::{build_register_region, region_pda, registry_pda};
use logos_osm::storage::{CodexStorage, MemoryStorage, Storage};
use logos_osm::{OsmClient, UpdateStatus};

// ---------------------------------------------------------------------------
// A real (tiny) HTTP server speaking Geofabrik's layout
// ---------------------------------------------------------------------------

/// Mutable fixture state: what "upstream" currently publishes for germany.
#[derive(Clone)]
struct Fixture {
    /// germany's current published version (YYYYMMDD).
    germany_version: Arc<Mutex<u32>>,
}

impl Fixture {
    fn new() -> Self {
        Self {
            germany_version: Arc::new(Mutex::new(20260821)),
        }
    }

    /// The PBF bytes for a region at the fixture's current published state.
    fn pbf(&self, path: &str) -> Vec<u8> {
        let mut ver = 20260820u32;
        if path == "germany" {
            ver = *self.germany_version.lock().unwrap();
        }
        // A structurally valid PBF: OSMHeader blob + OSMData blob(s), with
        // the version baked into the payload so versions differ.
        let header_payload = format!("osm-header-{path}-v{ver}").into_bytes();
        let data_payload = format!("osm-data-{path}-v{ver}").into_bytes();
        let mut out = blob("OSMHeader", &header_payload);
        out.extend_from_slice(&blob("OSMData", &data_payload));
        out
    }

    /// The `.md5` body + the dated `X-Derived-From` value (undated `-latest`
    /// body, like the real server for most regions).
    fn md5(&self, path: &str) -> (String, String) {
        let bytes = self.pbf(path);
        let hash = logos_osm::verify::md5_bytes(&bytes);
        let ver = if path == "germany" {
            *self.germany_version.lock().unwrap()
        } else {
            20260820
        };
        let body = format!("{}  {}-latest.osm.pbf\n", hex::encode(hash), path);
        // The dated upstream snapshot: yymmdd (the last 6 digits), as the
        // real server's X-Derived-From names it.
        let yymmdd = ver % 1_000_000;
        let derived = format!("/{path}-{yymmdd:06}.osm.pbf.md5");
        (body, derived)
    }

    /// The index document: every closed-set region with its canonical
    /// (real-Geofabrik) PBF URL — the URL-first join must succeed for all.
    fn index(&self) -> String {
        let features: Vec<String> = REGIONS
            .iter()
            .map(|r| {
                format!(
                    r#"{{ "type": "Feature", "properties": {{ "id": "{}", "parent": {}, "name": "{}", "urls": {{ "pbf": "{}" }} }} }}"#,
                    r.path,
                    r.parent
                        .map(|p| format!(r#""{p}""#))
                        .unwrap_or_else(|| "null".into()),
                    r.path,
                    r.source_url(),
                )
            })
            .collect();
        format!(
            r#"{{ "type": "FeatureCollection", "features": [{}] }}"#,
            features.join(",")
        )
    }
}

/// One PBF blob: 4-byte BE BlobHeader length, BlobHeader (field 1 `type`
/// string, field 3 `datasize` varint), then `datasize` bytes of payload.
fn blob(kind: &str, payload: &[u8]) -> Vec<u8> {
    let mut header = Vec::new();
    header.push(0x0a); // field 1 (type), wire 2
    header.push(kind.len() as u8);
    header.extend_from_slice(kind.as_bytes());
    let mut varint = Vec::new();
    let mut v = payload.len() as u64;
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            varint.push(b);
            break;
        }
        varint.push(b | 0x80);
    }
    header.push(0x18); // field 3 (datasize), wire 0
    header.extend_from_slice(&varint);

    let mut out = Vec::with_capacity(4 + header.len() + payload.len());
    out.extend_from_slice(&(header.len() as u32).to_be_bytes());
    out.extend_from_slice(&header);
    out.extend_from_slice(payload);
    out
}

/// Serve the fixture forever on an ephemeral port; return its base URL.
async fn spawn_fixture(fixture: Fixture) -> Result<String> {
    // Exact URL-path -> table region lookup (the same paths the canonical
    // Geofabrik URLs use, e.g. `/europe/germany-latest.osm.pbf`).
    let by_url: std::collections::HashMap<String, String> = REGIONS
        .iter()
        .map(|r| {
            let url = r.source_url();
            (
                url.trim_start_matches(logos_osm::geofabrik::DEFAULT_BASE)
                    .to_string(),
                r.path.to_string(),
            )
        })
        .collect();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                continue;
            };
            let fx = fixture.clone();
            let by_url = by_url.clone();
            tokio::spawn(async move {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut buf = vec![0u8; 8192];
                let Ok(n) = sock.read(&mut buf).await else {
                    return;
                };
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
                let (status, content_type, body, derived): (&str, &str, Vec<u8>, Option<String>) =
                    if path == "/index-v1-nogeom.json" {
                        ("200 OK", "application/json", fx.index().into_bytes(), None)
                    } else if let Some(region_path) = by_url.get(&path).cloned() {
                        (
                            "200 OK",
                            "application/octet-stream",
                            fx.pbf(&region_path),
                            None,
                        )
                    } else if let Some(region_path) =
                        by_url.get(path.strip_suffix(".md5").unwrap_or("")).cloned()
                    {
                        let (body, derived) = fx.md5(&region_path);
                        ("200 OK", "text/plain", body.into_bytes(), Some(derived))
                    } else {
                        ("404 Not Found", "text/plain", b"not found".to_vec(), None)
                    };
                let mut resp = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n",
                    body.len()
                );
                if let Some(d) = &derived {
                    resp.push_str(&format!("X-Derived-From: {d}\r\n"));
                }
                resp.push_str("\r\n");
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.write_all(&body).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    Ok(format!("http://{addr}"))
}

fn codex_url() -> String {
    std::env::var("OSM_CODEX_URL").unwrap_or_else(|_| "http://127.0.0.1:8080".to_owned())
}

/// The full off-chain flow against `storage`, with the given fixture base.
async fn run_lifecycle<S: Storage + 'static>(
    storage: S,
    geofabrik: &str,
    dir: &std::path::Path,
    fx: &Fixture,
) -> Result<()> {
    let client = OsmClient::new(storage, dir).with_geofabrik_base(geofabrik);

    // --- Discover: the whole closed set must be present upstream. ---
    let available = client
        .discover()
        .await
        .context("discover against fixture")?;
    assert_eq!(
        available.len(),
        REGIONS.len(),
        "every closed-set region must be discoverable"
    );

    // --- Host germany: download -> verify published MD5 -> store. ---
    let snap = client
        .host_region("germany")
        .await
        .context("host germany")?;
    assert_eq!(snap.region, "germany");
    assert_eq!(
        snap.version, 20260821,
        "version from the X-Derived-From signal"
    );
    assert_eq!(snap.bytes, fx.pbf("germany").len() as u64);
    let published = logos_osm::verify::md5_bytes(&fx.pbf("germany"));
    assert_eq!(
        snap.checksum, published,
        "snapshot MD5 must equal the published checksum"
    );
    assert!(!snap.cid.is_empty(), "stored: a CID came back");

    // --- The stored object is exactly the verified PBF (storage readback). ---
    let back = client
        .storage()
        .get_bytes(&snap.cid)
        .await
        .context("storage readback")?;
    assert_eq!(
        &back[..],
        &fx.pbf("germany")[..],
        "stored bytes == verified PBF"
    );

    // --- Registration tx shape (the wallet submits this). ---
    let registrar = lee_core::account::AccountId::new([9u8; 32]);
    let program_id = osm_registry::osm_registry_id();
    let built = build_register_region(&program_id, &registrar, &snap.registration(None));
    assert_eq!(
        built.accounts.len(),
        3,
        "registry state + region PDA + signer"
    );
    assert_eq!(built.accounts[1], region_pda(&program_id, "germany"));
    assert_eq!(built.accounts[0], registry_pda(&program_id));
    assert!(!built.instruction.is_empty());

    // --- Catalog it, then fetch from storage + verify + import. ---
    client.catalog_record_hosted(&snap)?;
    let fetched = client
        .fetch_snapshot("germany", &snap.cid, &snap.checksum)
        .await
        .context("fetch germany from storage")?;
    assert_eq!(fetched.md5, snap.checksum, "fetched snapshot re-verifies");
    let summary = client
        .import_local("germany", &fetched.path)
        .await
        .context("import")?;
    assert_eq!(summary.blobs, 2, "OSMHeader + OSMData");
    let catalog = client.catalog();
    assert_eq!(catalog.len(), 1);
    assert_eq!(catalog[0].region, "germany");
    assert_eq!(catalog[0].version, 20260821);

    // --- A subregion hosts too (decomposed us/ layout). ---
    let cali = client
        .host_region("us/california")
        .await
        .context("host us/california")?;
    assert_eq!(cali.version, 20260820);
    assert!(by_path("us/california").unwrap().level == Level::Subregion);

    // --- Upstream publishes a NEWER germany; update check must see it. ---
    {
        let mut v = fx.germany_version.lock().unwrap();
        *v = 20260822;
    }
    match client.update_check("germany").await? {
        UpdateStatus::UpdateAvailable { local, published } => {
            assert_eq!(local, 20260821);
            assert_eq!(published, 20260822);
        }
        other => bail!("expected UpdateAvailable, got {other:?}"),
    }
    // And the newer snapshot hosts at the new version with a new checksum.
    let snap2 = client
        .host_region("germany")
        .await
        .context("host germany v2")?;
    assert_eq!(snap2.version, 20260822);
    assert_ne!(
        snap2.checksum, snap.checksum,
        "new snapshot bytes => new MD5"
    );
    assert_ne!(snap2.cid, snap.cid, "content-addressed: new CID");

    // --- Local import workflow: user-provided PBF -> verify -> store -> tx. ---
    // The same bytes at v2 must pass and yield the same CID (content-addressed).
    let local_copy = dir.join("user-provided-germany.osm.pbf");
    std::fs::write(&local_copy, fx.pbf("germany"))?;
    let hosted_local = client
        .host_local("germany", &local_copy)
        .await
        .context("host_local with the exact published bytes")?;
    assert_eq!(hosted_local.checksum, snap2.checksum);
    assert_eq!(hosted_local.cid, snap2.cid, "same bytes => same CID");
    assert_eq!(hosted_local.version, 20260822);
    // A local file that no longer matches upstream is refused, not registered.
    let mut stale = fx.pbf("germany");
    let last = stale.len() - 1;
    stale[last] ^= 0xff;
    let stale_path = dir.join("stale-germany.osm.pbf");
    std::fs::write(&stale_path, stale)?;
    assert!(
        client.host_local("germany", &stale_path).await.is_err(),
        "a local PBF diverging from the published checksum must be refused"
    );

    // --- Tamper: a wrong published checksum must fail the fetch. ---
    let mut bad = snap.checksum;
    bad[0] ^= 0xff;
    assert!(
        client
            .fetch_snapshot("germany", &snap.cid, &bad)
            .await
            .is_err(),
        "a checksum mismatch must be an error, not a silent pass"
    );

    Ok(())
}

#[tokio::test]
async fn hermetic_lifecycle_on_fixture() -> Result<()> {
    let fx = Fixture::new();
    let base = spawn_fixture(fx.clone()).await?;
    let dir = tempfile::tempdir()?;
    run_lifecycle(MemoryStorage::new(), &base, dir.path(), &fx)
        .await
        .context("hermetic off-chain lifecycle")
}

#[tokio::test]
#[ignore = "needs the real Logos Storage (Codex) node — see module docs"]
async fn offchain_lifecycle_on_real_codex() -> Result<()> {
    // Codex needs a moment to bootstrap after the port binds, so retry the
    // probe (~90s) before bailing with start instructions.
    let probe = CodexStorage::new(codex_url());
    let mut warmed = false;
    for attempt in 1..=45 {
        if probe
            .put_bytes(bytes::Bytes::from_static(b"osm-offchain-live-probe"))
            .await
            .is_ok()
        {
            warmed = true;
            break;
        }
        if attempt == 1 {
            eprintln!("codex not ready yet; warming up...");
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    if !warmed {
        bail!(
            "Codex storage unreachable at {} — start it with:\n  \
             docker run -d --name osm-codex -p 8080:8080 -e NAT_IP_AUTO=false \\\n  \
             --entrypoint /usr/local/bin/storage \\\n  \
             codexstorage/nim-codex@sha256:25d9409b591da37200896e68fd9e4c591566f1ad0a80c0633c6cd5011ae56edb \\\n  \
             --api-bindaddr=0.0.0.0 --api-port=8080\n\
             (or set OSM_CODEX_URL)",
            codex_url()
        );
    }
    let fx = Fixture::new();
    let base = spawn_fixture(fx.clone()).await?;
    let dir = tempfile::tempdir()?;
    run_lifecycle(CodexStorage::new(codex_url()), &base, dir.path(), &fx)
        .await
        .context("real-Codex off-chain lifecycle")
}
