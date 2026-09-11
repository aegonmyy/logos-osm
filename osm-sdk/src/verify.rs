//! Streaming MD5 verification against Geofabrik's published checksums.
//!
//! Geofabrik publishes an `.md5` next to every `.osm.pbf` whose body is
//! `<hex hash>  <dated filename>`, e.g.
//!
//! ```text
//! 3196c1e372bb0131ae7deaa013954c63  germany-260524.osm.pbf
//! ```
//!
//! Two facts come out of that one line: the **checksum** (which certifies the
//! downloaded bytes are exactly what Geofabrik published — the integrity
//! story for a public dataset that needs no encryption) and the **version**
//! (the `yymmdd` inside the dated filename, normalized to `YYYYMMDD` — the
//! snapshot date the registry records and updates are compared against).
//!
//! Region PBFs reach multiple GB, so the digest is computed as a *stream*
//! (constant memory), never by buffering the file.

use md5::{Digest, Md5};

/// A parsed Geofabrik `.md5` line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChecksumFile {
    /// The published MD5, raw 16 bytes.
    pub checksum: [u8; 16],
    /// The filename named in the `.md5` body (may be dated or `-latest`).
    pub filename: String,
    /// The snapshot version `YYYYMMDD`, when one is recoverable from the
    /// body. Most `.md5` bodies name the *undated* `-latest` file; the dated
    /// source snapshot is then in the HTTP `X-Derived-From` header, which the
    /// client reads as a fallback (see [`version_from_derived_from`]).
    pub version: Option<u32>,
}

/// Parse a Geofabrik `.md5` body (`<hex>  <filename>`).
///
/// The version is read out of the dated filename **when the body carries
/// one** (some regions publish `<region>-<yymmdd>.osm.pbf` directly);
/// otherwise it is `None` and the caller fills it from the `X-Derived-From`
/// response header. Only the checksum and filename are mandatory.
pub fn parse_md5(body: &str) -> anyhow::Result<ChecksumFile> {
    let line = body.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let mut parts = line.split_whitespace();
    let hex_hash = parts.next().unwrap_or("");
    let filename = parts.next().unwrap_or("").to_string();

    anyhow::ensure!(!hex_hash.is_empty(), "md5 body has no hash: {body:?}");
    anyhow::ensure!(!filename.is_empty(), "md5 body has no filename: {body:?}");
    let checksum = decode_md5_hex(hex_hash)?;

    let version = version_from_filename(&filename).ok();

    Ok(ChecksumFile {
        checksum,
        filename,
        version,
    })
}

/// Decode a 32-char lowercase-or-uppercase hex MD5 into raw bytes.
pub fn decode_md5_hex(hex_hash: &str) -> anyhow::Result<[u8; 16]> {
    anyhow::ensure!(
        hex_hash.len() == 32,
        "md5 hex must be 32 chars, got {} ({hex_hash:?})",
        hex_hash.len()
    );
    let mut out = [0u8; 16];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex_hash[i * 2..i * 2 + 2], 16)
            .map_err(|e| anyhow::anyhow!("bad md5 hex {hex_hash:?}: {e}"))?;
    }
    Ok(out)
}

/// Extract the `YYYYMMDD` version from a dated Geofabrik filename
/// (`germany-260524.osm.pbf` → `20260524`).
///
/// The stem between the last `-` and `.osm.pbf` is `yymmdd`; the century is
/// Geofabrik's (2000s). Files without a 6-digit date suffix (e.g.
/// `kenya-latest.osm.pbf`) return `Err` — the dated source snapshot for
/// those lives in the HTTP `X-Derived-From` header; see
/// [`version_from_derived_from`].
pub fn version_from_filename(filename: &str) -> anyhow::Result<u32> {
    let stem = filename
        .strip_suffix(".osm.pbf")
        .ok_or_else(|| anyhow::anyhow!("not a .osm.pbf filename: {filename:?}"))?;
    let dated = stem
        .rsplit('-')
        .next()
        .ok_or_else(|| anyhow::anyhow!("no date segment in {filename:?}"))?;
    anyhow::ensure!(
        dated.len() == 6 && dated.chars().all(|c| c.is_ascii_digit()),
        "no yymmdd date in {filename:?}"
    );
    let yy: u32 = dated[0..2].parse()?;
    let mmdd: u32 = dated[2..6].parse()?;
    let version = 20_000_000 + yy * 10000 + mmdd;
    anyhow::ensure!(
        (1..=12).contains(&((mmdd / 100) % 100)),
        "implausible month in {filename:?}"
    );
    Ok(version)
}

/// Parse the `YYYYMMDD` version out of a Geofabrik `X-Derived-From` header.
///
/// Geofabrik sets `X-Derived-From: <path>/<region>-<yymmdd>.osm.pbf[.md5]`
/// on its `.md5` responses, naming the *dated* upstream snapshot the
/// `-latest` pointer currently resolves to — the reliable version signal
/// for regions whose `.md5` body itself names the undated `-latest` file.
/// `None` when the header is absent or carries no date.
pub fn version_from_derived_from(header: &str) -> Option<u32> {
    let name = header.rsplit('/').next()?;
    let name = name.strip_suffix(".md5").unwrap_or(name);
    version_from_filename(name).ok()
}

/// A streaming MD5 digester — feed chunks, finish into raw 16 bytes.
///
/// Constant memory regardless of input size; the PBFs this verifies reach
/// multiple GB.
#[derive(Default)]
pub struct Md5Stream {
    inner: Md5,
    len: u64,
}

impl Md5Stream {
    pub fn new() -> Self {
        Self::default()
    }

    /// Absorb one chunk.
    pub fn update(&mut self, chunk: &[u8]) {
        self.inner.update(chunk);
        self.len += chunk.len() as u64;
    }

    /// Bytes absorbed so far.
    pub fn len(&self) -> u64 {
        self.len
    }

    /// True if nothing was absorbed.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Finish: the raw 16-byte MD5 of everything absorbed.
    pub fn finalize(self) -> [u8; 16] {
        self.inner.finalize().into()
    }
}

/// Hash an in-memory buffer (used for tests and tiny files).
pub fn md5_bytes(data: &[u8]) -> [u8; 16] {
    let mut s = Md5Stream::new();
    s.update(data);
    s.finalize()
}

/// Compare a computed digest against a published one.
pub fn matches(computed: &[u8; 16], published: &[u8; 16]) -> bool {
    constant_time_eq(computed, published)
}

/// Length-independent, branch-free equality (digests are not secrets in this
/// public-data system, but there is no cost to comparing them safely).
fn constant_time_eq(a: &[u8; 16], b: &[u8; 16]) -> bool {
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    const BODY: &str = "3196c1e372bb0131ae7deaa013954c63  germany-260524.osm.pbf\n";

    #[test]
    fn parses_real_geofabrik_line() {
        // Dated body (germany, live-verified 2026-08-22).
        let f = parse_md5(BODY).unwrap();
        assert_eq!(
            f.checksum,
            decode_md5_hex("3196c1e372bb0131ae7deaa013954c63").unwrap()
        );
        assert_eq!(f.filename, "germany-260524.osm.pbf");
        assert_eq!(f.version, Some(20260524));
    }

    #[test]
    fn parses_undated_body_without_version() {
        // Most regions publish the -latest filename (live-verified: kenya,
        // france, california, ...). The version then comes from
        // X-Derived-From.
        let f = parse_md5("4774b0fee4d1a66c1552e55d4dc7a6cf  kenya-latest.osm.pbf").unwrap();
        assert_eq!(f.version, None);
        assert_eq!(f.checksum, decode_md5_hex("4774b0fee4d1a66c1552e55d4dc7a6cf").unwrap());
    }

    #[test]
    fn derived_from_header_carries_the_version() {
        // Live-verified 2026-08-22: kenya's .md5 response sets
        // X-Derived-From: africa/kenya-260821.osm.pbf.md5
        assert_eq!(
            version_from_derived_from("africa/kenya-260821.osm.pbf.md5"),
            Some(20260821)
        );
        // Tolerates the non-.md5 shape and a bare name.
        assert_eq!(
            version_from_derived_from("europe/germany-260524.osm.pbf"),
            Some(20260524)
        );
        assert_eq!(version_from_derived_from("germany-260524.osm.pbf"), Some(20260524));
        // Absent/undated headers are None, not errors.
        assert_eq!(version_from_derived_from(""), None);
        assert_eq!(version_from_derived_from("africa/kenya-latest.osm.pbf.md5"), None);
    }

    #[test]
    fn parses_newlineless_body() {
        let f = parse_md5("d41d8cd98f00b204e9800998ecf8427e  us/california-260810.osm.pbf")
            .unwrap();
        assert_eq!(f.version, Some(20260810));
    }

    #[test]
    fn rejects_bad_hex() {
        let e = parse_md5("zz  germany-260524.osm.pbf").unwrap_err();
        assert!(e.to_string().contains("32 chars"));
    }

    #[test]
    fn streaming_matches_buffered() {
        // 3 MiB in odd-sized chunks — streaming digest equals the one-shot.
        let data: Vec<u8> = (0..3 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
        let mut s = Md5Stream::new();
        let mut off = 0;
        while off < data.len() {
            let end = (off + 65537).min(data.len());
            s.update(&data[off..end]);
            off = end;
        }
        assert_eq!(s.len(), data.len() as u64);
        assert!(!s.is_empty());
        assert_eq!(s.finalize(), md5_bytes(&data));
    }

    #[test]
    fn known_md5_vector() {
        // RFC 1321: MD5("abc") = 900150983cd24fb0d6963f7d28e17f72
        assert_eq!(
            hex::encode(md5_bytes(b"abc")),
            "900150983cd24fb0d6963f7d28e17f72"
        );
    }

    #[test]
    fn version_rejects_implausible_month() {
        assert!(version_from_filename("kenya-261302.osm.pbf").is_err());
    }

    #[test]
    fn version_handles_single_digit_segments() {
        // Geofabrik zero-pads, but a defensive parse of 260101 works.
        assert_eq!(version_from_filename("kenya-260101.osm.pbf").unwrap(), 20260101);
    }
}
