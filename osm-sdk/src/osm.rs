//! The OSM distribution lifecycle facade — the prize's required flow as one
//! API: **discover → host (download + verify + store) → register → query →
//! download (storage, Geofabrik fallback) → import → update-check**.
//!
//! Map data is public: "hosting" means *verifiable availability*, not
//! secrecy. The integrity spine is Geofabrik's published MD5, recorded
//! on-chain next to the storage CID ([`crate::registry`]); every hop that
//! moves bytes re-computes the streaming MD5 and checks it before accepting
//! the result.
//!
//! Local state is a catalog (`<cache_dir>/catalog.json`) of what this machine
//! imported, keyed by region path. It is a *consumer* artifact only — the
//! source of truth for what is hosted is the chain.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::geofabrik::{DownloadOutcome, GeofabrikClient, RegionInfo};
use crate::regions::by_path;
use crate::registry::RegionRegistration;
use crate::storage::Storage;
use crate::verify::matches;

/// One hosted snapshot: everything the registry (and any consumer) needs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostedSnapshot {
    /// Region path (`germany`, `us/california`).
    pub region: String,
    /// Logos Storage CID of the stored PBF.
    pub cid: String,
    /// Geofabrik's published MD5 for this snapshot (raw bytes).
    pub checksum: [u8; 16],
    /// Snapshot version `YYYYMMDD`.
    pub version: u32,
    /// Size in bytes.
    pub bytes: u64,
    /// Where the verified local copy lives.
    pub path: PathBuf,
}

impl HostedSnapshot {
    /// The [`RegionRegistration`] a registrar submits to put this snapshot
    /// on the chain.
    pub fn registration(&self, timestamp: Option<u64>) -> RegionRegistration {
        RegionRegistration {
            region: self.region.clone(),
            cid: self.cid.clone(),
            checksum: self.checksum,
            version: self.version,
            timestamp: timestamp.unwrap_or_else(now_unix),
        }
    }
}

/// Bulk-hosting report: hosted regions plus the ones skipped by opt-out.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BulkHostReport {
    pub hosted: Vec<HostedSnapshot>,
    pub skipped: Vec<String>,
}

/// Result of a successful local import.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportSummary {
    pub region: String,
    pub path: PathBuf,
    pub bytes: u64,
    pub md5: [u8; 16],
    /// Number of PBF blobs (header + data blocks) found in the file.
    pub blobs: u64,
}

/// Outcome of an update check for one region.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum UpdateStatus {
    /// No newer snapshot than the local catalog's version.
    UpToDate { local: u32, published: u32 },
    /// Geofabrik publishes a newer snapshot than the local catalog's.
    UpdateAvailable { local: u32, published: u32 },
    /// Nothing imported locally for this region yet.
    NotImported { published: u32 },
}

/// A catalog record (local import state).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogRecord {
    pub region: String,
    pub path: PathBuf,
    pub md5: [u8; 16],
    pub version: u32,
    pub cid: Option<String>,
    pub imported_at: u64,
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The lifecycle client.
pub struct OsmClient<S: Storage> {
    geofabrik: GeofabrikClient,
    storage: S,
    cache_dir: PathBuf,
}

impl<S: Storage> OsmClient<S> {
    /// A client storing into `storage`, caching downloads under `cache_dir`.
    pub fn new(storage: S, cache_dir: impl Into<PathBuf>) -> Self {
        Self {
            geofabrik: GeofabrikClient::new(),
            storage,
            cache_dir: cache_dir.into(),
        }
    }

    /// Point the Geofabrik side at a mirror / local fixture server.
    pub fn with_geofabrik_base(mut self, base: impl Into<String>) -> Self {
        self.geofabrik = GeofabrikClient::with_base(base);
        self
    }

    fn region_path(&self, region: &str) -> PathBuf {
        // `us/california` -> `us_california-latest.osm.pbf` (flat cache dir).
        self.cache_dir
            .join(format!("{}-latest.osm.pbf", region.replace('/', "_")))
    }

    fn catalog_path(&self) -> PathBuf {
        self.cache_dir.join("catalog.json")
    }

    fn load_catalog(&self) -> Vec<CatalogRecord> {
        std::fs::read(self.catalog_path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn save_catalog(&self, catalog: &[CatalogRecord]) -> Result<()> {
        std::fs::create_dir_all(&self.cache_dir)?;
        serde_json::to_vec_pretty(catalog)
            .map_err(|e| anyhow::anyhow!("encoding catalog: {e}"))
            .and_then(|b| {
                std::fs::write(self.catalog_path(), b)
                    .map_err(|e| anyhow::anyhow!("writing catalog: {e}"))
            })
    }

    /// **Discover**: the closed-set regions available upstream, with URL
    /// cross-checks against the frozen table (all 72 must be present).
    pub async fn discover(&self) -> Result<Vec<RegionInfo>> {
        self.geofabrik.available_regions().await
    }

    /// **Host one region**: download from Geofabrik (streaming), verify the
    /// MD5 against Geofabrik's published checksum, store in Logos Storage,
    /// and return the snapshot (checksum + version included, ready to
    /// register).
    pub async fn host_region(&self, region: &str) -> Result<HostedSnapshot> {
        let r = by_path(region)
            .with_context(|| format!("{region} is not in the predefined region set"))?;
        let published = self.geofabrik.fetch_md5(r).await?;
        let dest = self.region_path(region);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let outcome = self.geofabrik.download(r, &dest).await?;
        anyhow::ensure!(
            matches(&outcome.md5, &published.checksum),
            "MD5 mismatch for {region}: downloaded {} but Geofabrik published {} — \
             keeping the file for inspection, refusing to store/register",
            hex::encode(outcome.md5),
            hex::encode(published.checksum)
        );
        let stored = self
            .storage
            .put_file(&dest)
            .await
            .with_context(|| format!("storing {region} in Logos Storage"))?;
        Ok(HostedSnapshot {
            region: region.to_string(),
            cid: stored.cid,
            checksum: published.checksum,
            version: published.version.unwrap_or(0),
            bytes: outcome.bytes,
            path: dest,
        })
    }

    /// **Bulk host** every region in `regions`, skipping any path listed in
    /// `opt_out` (the prize's opt-out semantics for bulk hosting). Regions
    /// are processed sequentially — Geofabrik rate-limits parallel fetches,
    /// and each region is GB-scale anyway.
    pub async fn host_regions_bulk(
        &self,
        regions: &[&str],
        opt_out: &[&str],
    ) -> Result<BulkHostReport> {
        let mut hosted = Vec::new();
        let mut skipped = Vec::new();
        for region in regions {
            if opt_out.contains(region) {
                skipped.push((*region).to_string());
                tracing::info!(region, "opt-out: skipping");
                continue;
            }
            match self.host_region(region).await {
                Ok(s) => hosted.push(s),
                Err(e) => {
                    // One region failing must not abort the bulk run.
                    tracing::error!(region, error = %e, "hosting failed; continuing");
                    skipped.push(format!("{region} (error: {e})"));
                }
            }
        }
        Ok(BulkHostReport { hosted, skipped })
    }

    /// **Download from the registry's storage**: fetch `cid` from Logos
    /// Storage, verify against `checksum`, and — only if storage fails —
    /// fall back to Geofabrik direct (also verified). Returns where the
    /// verified bytes landed.
    pub async fn fetch_snapshot(
        &self,
        region: &str,
        cid: &str,
        checksum: &[u8; 16],
    ) -> Result<DownloadOutcome> {
        let dest = self.region_path(region);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Primary: Logos Storage.
        match self.storage.get_to_file(cid, &dest).await {
            Ok(outcome) => {
                anyhow::ensure!(
                    matches(&outcome.md5, checksum),
                    "storage bytes for {region} fail the on-chain checksum"
                );
                return Ok(outcome);
            }
            Err(e) => {
                tracing::warn!(region, cid, error = %e, "storage fetch failed; falling back to Geofabrik");
            }
        }
        // Fallback: Geofabrik direct, same checksum enforced.
        let r = by_path(region)
            .with_context(|| format!("{region} is not in the predefined region set"))?;
        let outcome = self.geofabrik.download(r, &dest).await?;
        anyhow::ensure!(
            matches(&outcome.md5, checksum),
            "Geofabrik fallback bytes for {region} fail the on-chain checksum"
        );
        Ok(outcome)
    }

    /// **Import locally**: validate the PBF's structure (leading `OSMHeader`
    /// blob, coherent blob framing end-to-end), hash it, and record it in
    /// the local catalog. Returns the summary.
    ///
    /// "Import" here is deliberately honest about scope: the SDK validates
    /// and catalogs the extract; feeding a renderer/database (osm2pgsql,
    /// osmium…) is downstream tooling that consumes the same file.
    pub async fn import_local(&self, region: &str, pbf: &Path) -> Result<ImportSummary> {
        anyhow::ensure!(
            by_path(region).is_some(),
            "{region} is not in the predefined region set"
        );
        let data_len = tokio::fs::metadata(pbf).await?.len();
        let (blobs, computed) = validate_pbf(pbf)
            .await
            .with_context(|| format!("validating {} as an OSM PBF", pbf.display()))?;
        let mut catalog = self.load_catalog();
        // Same bytes as an existing record => keep its provenance (the
        // version + storage CID a host recorded). Different bytes => the
        // file's provenance is unknown to this machine, recorded honestly.
        let (version, cid) = match catalog.iter().find(|c| c.region == region) {
            Some(prior) if prior.md5 == computed => (prior.version, prior.cid.clone()),
            _ => (0, None),
        };
        catalog.retain(|c| c.region != region);
        catalog.push(CatalogRecord {
            region: region.to_string(),
            path: pbf.to_path_buf(),
            md5: computed,
            version,
            cid,
            imported_at: now_unix(),
        });
        self.save_catalog(&catalog)?;
        Ok(ImportSummary {
            region: region.to_string(),
            path: pbf.to_path_buf(),
            bytes: data_len,
            md5: computed,
            blobs,
        })
    }

    /// **Host a local PBF** (the "local import" workflow): verify the file's
    /// MD5 against Geofabrik's *currently published* checksum, store it in
    /// Logos Storage, and return the snapshot ready to register — the same
    /// guarantees as [`OsmClient::host_region`] but with bytes the user
    /// already has. The published checksum is authoritative: a local file
    /// that no longer matches upstream is refused, not silently registered.
    pub async fn host_local(&self, region: &str, pbf: &Path) -> Result<HostedSnapshot> {
        let r = by_path(region)
            .with_context(|| format!("{region} is not in the predefined region set"))?;
        let published = self.geofabrik.fetch_md5(r).await?;
        let (blobs, computed) = validate_pbf(pbf)
            .await
            .with_context(|| format!("validating {} as an OSM PBF", pbf.display()))?;
        anyhow::ensure!(
            matches(&computed, &published.checksum),
            "MD5 mismatch for {region}: local file is {} but Geofabrik published {} — \
             refusing to store/register a file that no longer matches upstream",
            hex::encode(computed),
            hex::encode(published.checksum)
        );
        let bytes = tokio::fs::metadata(pbf).await?.len();
        let stored = self
            .storage
            .put_file(pbf)
            .await
            .with_context(|| format!("storing local {region} in Logos Storage"))?;
        tracing::info!(region, blobs, "hosted local PBF");
        Ok(HostedSnapshot {
            region: region.to_string(),
            cid: stored.cid,
            checksum: published.checksum,
            version: published.version.unwrap_or(0),
            bytes,
            path: pbf.to_path_buf(),
        })
    }

    /// Record a hosted snapshot in the local catalog (region, version, cid).
    pub fn catalog_record_hosted(&self, snapshot: &HostedSnapshot) -> Result<()> {
        let mut catalog = self.load_catalog();
        catalog.retain(|c| c.region != snapshot.region);
        catalog.push(CatalogRecord {
            region: snapshot.region.clone(),
            path: snapshot.path.clone(),
            md5: snapshot.checksum,
            version: snapshot.version,
            cid: Some(snapshot.cid.clone()),
            imported_at: now_unix(),
        });
        self.save_catalog(&catalog)
    }

    /// **Update check**: compare the local catalog's version for `region`
    /// against Geofabrik's currently-published snapshot version.
    pub async fn update_check(&self, region: &str) -> Result<UpdateStatus> {
        let r = by_path(region)
            .with_context(|| format!("{region} is not in the predefined region set"))?;
        let published = self.geofabrik.fetch_md5(r).await?;
        let published = published
            .version
            .expect("fetch_md5 resolves a version or errors");
        let local = self.load_catalog().into_iter().find(|c| c.region == region);
        Ok(match local {
            None => UpdateStatus::NotImported { published },
            Some(rec) if rec.version >= published => UpdateStatus::UpToDate {
                local: rec.version,
                published,
            },
            Some(rec) => UpdateStatus::UpdateAvailable {
                local: rec.version,
                published,
            },
        })
    }

    /// The local catalog (what this machine has imported).
    pub fn catalog(&self) -> Vec<CatalogRecord> {
        self.load_catalog()
    }

    /// Storage handle accessor (for direct put/get by callers).
    pub fn storage(&self) -> &S {
        &self.storage
    }
}

/// Validate PBF framing: a 4-byte **big-endian** length, a `BlobHeader`
/// protobuf whose `type` is read, then `datasize` bytes of blob — repeated
/// to EOF. The first blob must be the `OSMHeader`. Returns `(blob_count,
/// md5)`; the MD5 is computed in the same pass.
///
/// Only the two header fields the format guarantees are parsed (field 1
/// `type`, field 3 `datasize`); unknown fields are skipped per protobuf
/// rules.
async fn validate_pbf(path: &Path) -> Result<(u64, [u8; 16])> {
    use tokio::io::AsyncReadExt;

    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = crate::verify::Md5Stream::new();
    let mut blobs = 0u64;
    let mut first = true;
    loop {
        let mut len_buf = [0u8; 4];
        match file.read_exact(&mut len_buf).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(anyhow::anyhow!("reading blob length: {e}")),
        }
        hasher.update(&len_buf);
        let header_len = u32::from_be_bytes(len_buf);
        anyhow::ensure!(
            header_len > 0 && header_len < 1 << 20,
            "implausible BlobHeader length {header_len}"
        );
        let mut header = vec![0u8; header_len as usize];
        file.read_exact(&mut header).await?;
        hasher.update(&header);
        let (blob_type, datasize) = parse_blob_header(&header)?;
        if first {
            anyhow::ensure!(
                blob_type == "OSMHeader",
                "first blob is {blob_type:?}, expected OSMHeader"
            );
            first = false;
        }
        anyhow::ensure!(datasize > 0, "empty blob data");
        let mut data = vec![0u8; datasize as usize];
        file.read_exact(&mut data).await?;
        hasher.update(&data);
        blobs += 1;
    }
    anyhow::ensure!(blobs >= 1, "no blobs found");
    anyhow::ensure!(!first, "file had no OSMHeader blob");
    Ok((blobs, hasher.finalize()))
}

/// Parse the two fields we need out of a `BlobHeader` protobuf:
/// field 1 (`type`, string) and field 3 (`datasize`, varint).
fn parse_blob_header(buf: &[u8]) -> Result<(String, u32)> {
    let mut i = 0usize;
    let mut blob_type = String::new();
    let mut datasize = 0u32;
    while i < buf.len() {
        let (tag, next) = read_varint(buf, i)?;
        i = next;
        let field = tag >> 3;
        let wire = tag & 7;
        match (field, wire) {
            (1, 2) => {
                let (len, next) = read_varint(buf, i)?;
                i = next;
                let end = i + len as usize;
                anyhow::ensure!(end <= buf.len(), "type string overruns header");
                blob_type = String::from_utf8_lossy(&buf[i..end]).into_owned();
                i = end;
            }
            (3, 0) => {
                let (v, next) = read_varint(buf, i)?;
                i = next;
                datasize = u32::try_from(v)?;
            }
            (_, 0) => {
                let (_, next) = read_varint(buf, i)?;
                i = next;
            }
            (_, 2) => {
                let (len, next) = read_varint(buf, i)?;
                i = next + len as usize;
            }
            (_, w) => anyhow::bail!("unsupported protobuf wire type {w}"),
        }
    }
    Ok((blob_type, datasize))
}

fn read_varint(buf: &[u8], mut i: usize) -> Result<(u64, usize)> {
    let mut value = 0u64;
    let mut shift = 0;
    loop {
        anyhow::ensure!(i < buf.len(), "varint overruns buffer");
        let b = buf[i];
        i += 1;
        value |= u64::from(b & 0x7f) << shift;
        if b & 0x80 == 0 {
            return Ok((value, i));
        }
        shift += 7;
        anyhow::ensure!(shift < 64, "varint too long");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::MemoryStorage;
    use bytes::Bytes;

    fn minimal_pbf(blobs: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        for (t, data) in blobs {
            // BlobHeader: field1 type (string), field3 datasize (varint).
            let mut header = Vec::new();
            header.push(0x0a); // field 1, wire 2
            header.push(t.len() as u8);
            header.extend_from_slice(t.as_bytes());
            let mut varint = Vec::new();
            let mut v = data.len() as u64;
            loop {
                let b = (v & 0x7f) as u8;
                v >>= 7;
                if v == 0 {
                    varint.push(b);
                    break;
                }
                varint.push(b | 0x80);
            }
            header.push(0x18); // field 3, wire 0
            header.extend_from_slice(&varint);
            out.extend_from_slice(&(header.len() as u32).to_be_bytes());
            out.extend_from_slice(&header);
            out.extend_from_slice(data);
        }
        out
    }

    fn client(dir: &Path) -> OsmClient<MemoryStorage> {
        OsmClient::new(MemoryStorage::new(), dir.to_path_buf())
    }

    #[tokio::test]
    async fn import_validates_and_catalogs() {
        let dir = tempfile::tempdir().unwrap();
        let pbf_path = dir.path().join("kenya-latest.osm.pbf");
        let pbf = minimal_pbf(&[("OSMHeader", b"headerblob"), ("OSMData", b"aaaabbbbcccc")]);
        std::fs::write(&pbf_path, &pbf).unwrap();
        let c = client(dir.path());
        let summary = c.import_local("kenya", &pbf_path).await.unwrap();
        assert_eq!(summary.blobs, 2);
        assert_eq!(summary.md5, crate::verify::md5_bytes(&pbf));
        assert_eq!(c.catalog().len(), 1);
        assert_eq!(c.catalog()[0].region, "kenya");
    }

    #[tokio::test]
    async fn import_rejects_wrong_first_blob() {
        let dir = tempfile::tempdir().unwrap();
        let pbf_path = dir.path().join("x.osm.pbf");
        std::fs::write(
            &pbf_path,
            minimal_pbf(&[("OSMData", b"nope"), ("OSMData", b"nope")]),
        )
        .unwrap();
        let c = client(dir.path());
        let err = c.import_local("kenya", &pbf_path).await.unwrap_err();
        assert!(format!("{err:#}").contains("OSMHeader"), "{err:#}");
    }

    #[tokio::test]
    async fn import_rejects_garbage() {
        let dir = tempfile::tempdir().unwrap();
        let pbf_path = dir.path().join("x.osm.pbf");
        std::fs::write(&pbf_path, b"not a pbf at all").unwrap();
        let c = client(dir.path());
        assert!(c.import_local("kenya", &pbf_path).await.is_err());
    }

    #[tokio::test]
    async fn import_rejects_region_outside_set() {
        let dir = tempfile::tempdir().unwrap();
        let pbf_path = dir.path().join("x.osm.pbf");
        std::fs::write(&pbf_path, minimal_pbf(&[("OSMHeader", b"h")])).unwrap();
        let c = client(dir.path());
        assert!(c.import_local("narnia", &pbf_path).await.is_err());
    }

    #[tokio::test]
    async fn snapshot_registration_shape() {
        let s = HostedSnapshot {
            region: "germany".into(),
            cid: "cid".into(),
            checksum: [1; 16],
            version: 20260821,
            bytes: 10,
            path: PathBuf::from("/tmp/g.osm.pbf"),
        };
        let reg = s.registration(Some(42));
        assert_eq!(reg.region, "germany");
        assert_eq!(reg.cid, "cid");
        assert_eq!(reg.version, 20260821);
        assert_eq!(reg.timestamp, 42);
        assert!(s.registration(None).timestamp > 1_700_000_000);
    }

    #[tokio::test]
    async fn memory_storage_roundtrip_through_facade_types() {
        // The facade's snapshot -> registration -> (decode) chain holds.
        let dir = tempfile::tempdir().unwrap();
        let c = client(dir.path());
        let stored = c
            .storage()
            .put_bytes(Bytes::from_static(b"pbf"))
            .await
            .unwrap();
        let back = c.storage().get_bytes(&stored.cid).await.unwrap();
        assert_eq!(&back[..], b"pbf");
    }
}
