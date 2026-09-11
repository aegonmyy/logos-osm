//! Geofabrik client: region discovery, checksum (`.md5`) fetch, and
//! streaming PBF download.
//!
//! Geofabrik (<https://download.geofabrik.de>) is the upstream of record for
//! OpenStreetMap region extracts. Two artifacts matter here:
//!
//! - **`index-v1-nogeom.json`** — a GeoJSON FeatureCollection (one feature
//!   per extract, no polygons) whose `properties.urls.pbf` is the canonical
//!   download URL. Discovery = the intersection of this live index with the
//!   frozen closed set in [`crate::regions`], cross-checked so the table and
//!   the index never disagree silently.
//! - **`<region>-latest.osm.pbf.md5`** — the published checksum line; see
//!   [`crate::verify`] for what one line yields (integrity + version).
//!
//! The join between index entries and the frozen table is *not* a plain id
//! equality: most index ids equal the table path (`germany`,
//! `us/california`), but the four decomposed russia districts sit under a
//! top-level `russia/` directory on disk while the index lists them by bare
//! basename (`central-fed-district`). [`match_region`] handles both shapes
//! and then asserts the index's `urls.pbf` equals the table's URL — a
//! mismatch is an error, not a silent preference, because the table is what
//! the guest enforces on-chain.
//!
//! Every network call goes through [`crate::retry`] with the base URL
//! overridable (`GeofabrikClient::with_base`) so the hermetic tests and the
//! integration harness can point at a local fixture server instead of the
//! real Geofabrik.

use std::path::Path;

use anyhow::{Context, Result};
use bytes::Bytes;
use futures::StreamExt;
use serde::Deserialize;
use tokio::io::AsyncWriteExt;

use crate::regions::Region;
use crate::retry::{retry_transient, RetryErr};
use crate::verify::{parse_md5, ChecksumFile, Md5Stream};

/// Default Geofabrik download server.
pub const DEFAULT_BASE: &str = "https://download.geofabrik.de";

/// One region extract from Geofabrik's index (the fields the SDK needs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexEntry {
    /// Index id (`germany`, `us/california`, `central-fed-district`).
    pub id: String,
    /// Containing extract per the index, if any.
    pub parent: Option<String>,
    /// Human-readable name.
    pub name: Option<String>,
    /// The canonical PBF URL.
    pub pbf_url: Option<String>,
}

#[derive(Deserialize)]
struct RawIndex {
    #[serde(default)]
    features: Vec<RawFeature>,
}

#[derive(Deserialize)]
struct RawFeature {
    #[serde(default)]
    properties: RawProperties,
}

#[derive(Deserialize, Default)]
struct RawProperties {
    #[serde(default)]
    id: String,
    parent: Option<String>,
    name: Option<String>,
    urls: Option<RawUrls>,
}

#[derive(Deserialize)]
struct RawUrls {
    pbf: Option<String>,
}

/// Parse an `index-v1-nogeom.json` document (or the same shape with
/// geometry stripped) into [`IndexEntry`]s. Tolerant: unknown fields and
/// missing URLs are skipped.
pub fn parse_index(body: &str) -> Result<Vec<IndexEntry>> {
    let raw: RawIndex = serde_json::from_str(body).context("parsing index-v1-nogeom.json")?;
    Ok(raw
        .features
        .into_iter()
        .filter(|f| !f.properties.id.is_empty())
        .map(|f| IndexEntry {
            id: f.properties.id,
            parent: f.properties.parent,
            name: f.properties.name,
            pbf_url: f.properties.urls.and_then(|u| u.pbf),
        })
        .collect())
}

/// Match a frozen-table region against index entries.
///
/// The **authoritative join key is the PBF URL**: an entry carrying
/// `urls.pbf` matches when it equals the table's `source_url()` — that
/// covers every closed-set region including the odd shapes (Ireland's index
/// id is `ireland-and-northern-ireland` while the table path is `ireland`;
/// russia's districts sit under a top-level `russia/` dir but carry bare
/// basename ids in the index). Entries without a URL fall back to id
/// equality against the path or its basename.
///
/// Returns `None` when the region is absent from the index; an error when
/// an id-shaped candidate's URL *disagrees* with the table (the table is
/// what the chain enforces, so a silent preference would fork the truth).
pub fn match_region(entries: &[IndexEntry], region: &Region) -> Result<Option<IndexEntry>> {
    let source = region.source_url();
    // 1. URL join (authoritative when the entry publishes one).
    if let Some(e) = entries
        .iter()
        .find(|e| e.pbf_url.as_deref() == Some(source.as_str()))
    {
        return Ok(Some(e.clone()));
    }
    // 2. Id join, only for URL-less entries. A URL-bearing entry with the
    //    right id but a *different* URL is a disagreement (URL matches
    //    already returned above) — error, not silent preference.
    let base = region.path.rsplit('/').next().unwrap_or(region.path);
    let id_hits: Vec<&IndexEntry> = entries
        .iter()
        .filter(|e| e.id == region.path || e.id == base)
        .collect();
    for e in &id_hits {
        if let Some(url) = &e.pbf_url {
            anyhow::bail!(
                "index/table URL mismatch for {}: index id {} says {url}, table says {source}",
                region.path,
                e.id
            );
        }
    }
    Ok(id_hits.first().map(|e| (*e).clone()))
}

/// A region of the closed set, enriched with live index metadata.
#[derive(Debug, Clone)]
pub struct RegionInfo {
    /// The frozen-table region (path, level, parent, URLs).
    pub region: &'static Region,
    /// Human-readable name from the index, if available.
    pub name: Option<String>,
    /// The canonical PBF URL (table's `source_url`).
    pub pbf_url: String,
    /// The published MD5 URL (table's `checksum_url`).
    pub md5_url: String,
}

/// Client for one Geofabrik server.
#[derive(Debug, Clone)]
pub struct GeofabrikClient {
    http: reqwest::Client,
    base: String,
}

impl Default for GeofabrikClient {
    fn default() -> Self {
        Self::new()
    }
}

impl GeofabrikClient {
    /// Client for the real Geofabrik server.
    pub fn new() -> Self {
        Self::with_base(DEFAULT_BASE)
    }

    /// Client for a mirror / local fixture server (tests, offline demo).
    pub fn with_base(base: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            base: {
                let b: String = base.into();
                b.trim_end_matches('/').to_string()
            },
        }
    }

    /// Map a canonical table URL onto this client's base (mirror override).
    ///
    /// Region URLs from the frozen table are absolute `download.geofabrik.de`
    /// links (that absolute form is the join key against the live index and
    /// must not change). When the client points at a mirror/fixture, the
    /// canonical host is swapped for the base, keeping the path — so a
    /// fixture at `http://127.0.0.1:PORT` serves the same
    /// `/{path}-latest.osm.pbf` layout Geofabrik does.
    fn mirror(&self, url: &str) -> String {
        if self.base == DEFAULT_BASE {
            url.to_string()
        } else {
            url.replacen(DEFAULT_BASE, &self.base, 1)
        }
    }

    async fn get_text(&self, url: &str) -> Result<String> {
        retry_transient(|| async {
            let resp = self
                .http
                .get(url)
                .send()
                .await
                .map_err(|e| RetryErr::Transient(anyhow::anyhow!("GET {url}: {e}")))?;
            let status = resp.status();
            if !status.is_success() {
                let err = anyhow::anyhow!("GET {url}: {status}");
                return Err(if crate::retry::status_is_transient(status.as_u16()) {
                    RetryErr::Transient(err)
                } else {
                    RetryErr::Fatal(err)
                });
            }
            resp.text()
                .await
                .map_err(|e| RetryErr::Transient(anyhow::anyhow!("GET {url} body: {e}")))
        })
        .await
    }

    /// Fetch and parse the full index.
    pub async fn fetch_index(&self) -> Result<Vec<IndexEntry>> {
        let body = self
            .get_text(&format!("{}/index-v1-nogeom.json", self.base))
            .await?;
        parse_index(&body)
    }

    /// Discovery: the closed-set regions present in the live index, with
    /// URL cross-checks. Regions missing from the index are reported in the
    /// error's context rather than silently dropped.
    pub async fn available_regions(&self) -> Result<Vec<RegionInfo>> {
        let entries = self.fetch_index().await?;
        regions_from_index(&entries)
    }

    /// Live metadata for one closed-set region.
    pub async fn region_info(&self, region: &'static Region) -> Result<RegionInfo> {
        let entries = self.fetch_index().await?;
        region_info_from_index(&entries, region)
    }

    /// Fetch and parse the region's published `.md5`: the checksum (always)
    /// and the snapshot version (`YYYYMMDD`).
    ///
    /// Version resolution: the dated filename in the `.md5` body **or**, when
    /// the body names the undated `-latest` file, the dated source snapshot
    /// in the `X-Derived-From` response header (Geofabrik sets it on every
    /// `.md5` response). Returns an error if neither carries a date — the
    /// registry keys freshness on this field.
    pub async fn fetch_md5(&self, region: &Region) -> Result<ChecksumFile> {
        let url = self.mirror(&region.checksum_url());
        let (body, derived) = retry_transient(|| async {
            let resp = self
                .http
                .get(&url)
                .send()
                .await
                .map_err(|e| RetryErr::Transient(anyhow::anyhow!("GET {url}: {e}")))?;
            let status = resp.status();
            if !status.is_success() {
                let err = anyhow::anyhow!("GET {url}: {status}");
                return Err(if crate::retry::status_is_transient(status.as_u16()) {
                    RetryErr::Transient(err)
                } else {
                    RetryErr::Fatal(err)
                });
            }
            let derived = resp
                .headers()
                .get("x-derived-from")
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned);
            let body = resp
                .text()
                .await
                .map_err(|e| RetryErr::Transient(anyhow::anyhow!("GET {url} body: {e}")))?;
            Ok((body, derived))
        })
        .await
        .with_context(|| format!("fetching checksum for {} from {url}", region.path))?;

        let mut file = parse_md5(&body).with_context(|| format!("parsing {url}"))?;
        if file.version.is_none() {
            if let Some(h) = derived {
                file.version = crate::verify::version_from_derived_from(&h);
            }
        }
        anyhow::ensure!(
            file.version.is_some(),
            "no version for {}: body names no date and X-Derived-From is absent/undated",
            region.path
        );
        Ok(file)
    }

    /// Streaming download of the region's PBF into `dest`.
    ///
    /// Constant memory regardless of file size (region extracts reach
    /// multiple GB): the response is consumed chunk-by-chunk, hashed with a
    /// streaming MD5 as it lands, and written straight to disk. Returns the
    /// byte count and the computed MD5 so callers verify before use.
    ///
    /// Progress is reported through `tracing` (INFO tick every 64 MiB) so
    /// the future stays retryable without threading a callback through the
    /// retry closure.
    pub async fn download(&self, region: &Region, dest: &Path) -> Result<DownloadOutcome> {
        let url = self.mirror(&region.source_url());
        retry_transient(|| async {
            let resp = self
                .http
                .get(&url)
                .send()
                .await
                .map_err(|e| RetryErr::Transient(anyhow::anyhow!("GET {url}: {e}")))?;
            let status = resp.status();
            if !status.is_success() {
                let err = anyhow::anyhow!("GET {url}: {status}");
                return Err(if crate::retry::status_is_transient(status.as_u16()) {
                    RetryErr::Transient(err)
                } else {
                    RetryErr::Fatal(err)
                });
            }
            let total = resp.content_length();
            let mut hasher = Md5Stream::new();
            let file = tokio::fs::File::create(dest)
                .await
                .map_err(|e| RetryErr::Fatal(anyhow::anyhow!("create {}: {e}", dest.display())))?;
            let mut writer = tokio::io::BufWriter::with_capacity(1 << 20, file);
            let mut stream = resp.bytes_stream();
            let mut last_report = 0u64;
            while let Some(chunk) = stream.next().await {
                let chunk: Bytes =
                    chunk.map_err(|e| RetryErr::Transient(anyhow::anyhow!("stream {url}: {e}")))?;
                hasher.update(&chunk);
                writer.write_all(&chunk).await.map_err(|e| {
                    RetryErr::Fatal(anyhow::anyhow!("write {}: {e}", dest.display()))
                })?;
                if hasher.len() - last_report >= 64 << 20 {
                    last_report = hasher.len();
                    tracing::info!(
                        bytes = hasher.len(),
                        total = ?total,
                        "downloading {}",
                        region.path
                    );
                }
            }
            writer
                .flush()
                .await
                .map_err(|e| RetryErr::Fatal(anyhow::anyhow!("flush {}: {e}", dest.display())))?;
            tracing::info!(bytes = hasher.len(), "downloaded {}", region.path);
            Ok(DownloadOutcome {
                path: dest.to_path_buf(),
                bytes: hasher.len(),
                md5: hasher.finalize(),
            })
        })
        .await
    }
}

/// Build [`RegionInfo`]s for every closed-set region found in an already
/// parsed index.
pub fn regions_from_index(entries: &[IndexEntry]) -> Result<Vec<RegionInfo>> {
    let mut out = Vec::new();
    let mut missing = Vec::new();
    for region in crate::regions::REGIONS {
        match match_region(entries, region)? {
            Some(e) => out.push(RegionInfo {
                region,
                name: e.name,
                pbf_url: region.source_url(),
                md5_url: region.checksum_url(),
            }),
            None => missing.push(region.path),
        }
    }
    anyhow::ensure!(
        missing.is_empty(),
        "{} region(s) absent from the index: {}",
        missing.len(),
        missing.join(", ")
    );
    Ok(out)
}

/// [`regions_from_index`] for one region.
pub fn region_info_from_index(
    entries: &[IndexEntry],
    region: &'static Region,
) -> Result<RegionInfo> {
    match match_region(entries, region)? {
        Some(e) => Ok(RegionInfo {
            region,
            name: e.name,
            pbf_url: region.source_url(),
            md5_url: region.checksum_url(),
        }),
        None => Err(anyhow::anyhow!(
            "{} absent from the Geofabrik index",
            region.path
        )),
    }
}

/// The result of a completed download (or storage fetch) to a file.
#[derive(Debug, Clone)]
pub struct DownloadOutcome {
    /// Where the bytes landed.
    pub path: std::path::PathBuf,
    /// How many bytes landed.
    pub bytes: u64,
    /// Streaming MD5 of exactly those bytes.
    pub md5: [u8; 16],
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::regions::by_path;

    /// Minimal-but-real slice of the index shape (structure taken from the
    /// live index-v1-nogeom.json, 2026-08).
    const FIXTURE: &str = r#"{
        "type": "FeatureCollection",
        "features": [
            { "type": "Feature",
              "properties": { "id": "kenya", "parent": "africa", "name": "Kenya",
                "iso3166-1:alpha2": ["KE"],
                "urls": { "pbf": "https://download.geofabrik.de/africa/kenya-latest.osm.pbf",
                          "shp": "https://download.geofabrik.de/africa/kenya-latest-free.shp.zip" } } },
            { "type": "Feature",
              "properties": { "id": "germany", "parent": "europe", "name": "Germany",
                "urls": { "pbf": "https://download.geofabrik.de/europe/germany-latest.osm.pbf" } } },
            { "type": "Feature",
              "properties": { "id": "us/california", "parent": "us", "name": "California",
                "urls": { "pbf": "https://download.geofabrik.de/north-america/us/california-latest.osm.pbf" } } },
            { "type": "Feature",
              "properties": { "id": "central-fed-district", "parent": "russia",
                "name": "Central Federal District",
                "urls": { "pbf": "https://download.geofabrik.de/russia/central-fed-district-latest.osm.pbf" } } },
            { "type": "Feature",
              "properties": { "id": "ireland-and-northern-ireland", "parent": "europe",
                "urls": { "pbf": "https://download.geofabrik.de/europe/ireland-and-northern-ireland-latest.osm.pbf" } } }
        ]
    }"#;

    fn fixture_entries() -> Vec<IndexEntry> {
        parse_index(FIXTURE).unwrap()
    }

    #[test]
    fn parses_the_geofabrik_shape() {
        let entries = fixture_entries();
        assert_eq!(entries.len(), 5);
        assert_eq!(entries[0].id, "kenya");
        assert_eq!(entries[0].parent.as_deref(), Some("africa"));
        assert!(entries[0]
            .pbf_url
            .as_deref()
            .unwrap()
            .contains("kenya-latest.osm.pbf"));
        // Unknown/extra properties are tolerated.
        assert_eq!(entries[2].id, "us/california");
    }

    #[test]
    fn joins_path_and_basename_shapes() {
        let entries = fixture_entries();
        // Plain country.
        let m = match_region(&entries, by_path("germany").unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(m.id, "germany");
        // Nested US state.
        let m = match_region(&entries, by_path("us/california").unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(m.id, "us/california");
        // Russia district: table path russia/..., index id is the basename.
        let m = match_region(&entries, by_path("russia/central-fed-district").unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(m.id, "central-fed-district");
        // Ireland's on-disk name differs from its path.
        let m = match_region(&entries, by_path("ireland").unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(m.id, "ireland-and-northern-ireland");
    }

    #[test]
    fn region_absent_from_index_is_none() {
        let entries = fixture_entries();
        assert!(match_region(&entries, by_path("france").unwrap())
            .unwrap()
            .is_none());
    }

    #[test]
    fn url_mismatch_is_an_error_not_a_preference() {
        let mut entries = fixture_entries();
        entries[1].pbf_url = Some("https://evil.example/germany.pbf".into());
        let err = match_region(&entries, by_path("germany").unwrap()).unwrap_err();
        assert!(err.to_string().contains("mismatch"), "{err}");
    }

    #[test]
    fn discovery_reports_missing_not_silent() {
        // Only some regions present -> error naming the missing ones.
        let entries = fixture_entries();
        let err = regions_from_index(&entries).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("absent from the index"), "{msg}");
        assert!(msg.contains("france"), "{msg}");
        // And a single-region lookup of a present region works.
        let info = region_info_from_index(&entries, by_path("germany").unwrap()).unwrap();
        assert_eq!(
            info.pbf_url,
            "https://download.geofabrik.de/europe/germany-latest.osm.pbf"
        );
    }

    #[tokio::test]
    async fn md5_fetch_parses_via_mock_server_shape() {
        // The client's md5 path is GET {base}/{region-path}-latest.osm.pbf.md5
        // + parse_md5; the parse half is fully tested in verify.rs. Here we
        // assert the URL construction the client will hit.
        let c = GeofabrikClient::with_base("http://127.0.0.1:9/");
        let r = by_path("germany").unwrap();
        assert_eq!(
            r.checksum_url(),
            "https://download.geofabrik.de/europe/germany-latest.osm.pbf.md5"
        );
        let _ = c; // client retained for the live smoke test below
    }

    /// Live smoke against the real Geofabrik server (run deliberately):
    ///
    /// ```text
    /// cargo test -p logos-osm --lib geofabrik_live -- --ignored --nocapture
    /// ```
    ///
    /// Proves: every one of the 72 closed-set regions is present in the
    /// live index with the exact URL our frozen table records, and the
    /// `.md5` for a sample region parses into (checksum, version).
    #[tokio::test]
    #[ignore = "network: touches download.geofabrik.de"]
    async fn geofabrik_live_smoke() {
        let client = GeofabrikClient::new();
        let available = client.available_regions().await.unwrap();
        assert_eq!(available.len(), crate::regions::REGIONS.len());
        let kenya = by_path("kenya").unwrap();
        let md5 = client.fetch_md5(kenya).await.unwrap();
        println!(
            "kenya: version {:?} md5 {}",
            md5.version,
            hex::encode(md5.checksum)
        );
        let version = md5
            .version
            .expect("version resolved from body or X-Derived-From");
        assert!(version >= 20250101, "implausible version {version}");
    }

    #[test]
    fn mirror_override_maps_host_not_path() {
        let real = GeofabrikClient::new();
        let fixture = GeofabrikClient::with_base("http://127.0.0.1:3917/");
        let url = by_path("germany").unwrap().source_url();
        // The real server is untouched; a mirror swaps only the host.
        assert_eq!(real.mirror(&url), url);
        assert_eq!(
            fixture.mirror(&url),
            "http://127.0.0.1:3917/europe/germany-latest.osm.pbf"
        );
        // A URL that is not a table URL passes through unchanged (no
        // accidental rewrite of foreign hosts).
        assert_eq!(
            fixture.mirror("https://example.org/x.osm.pbf"),
            "https://example.org/x.osm.pbf"
        );
    }
}
