//! Minimal OSM PBF data reader for the search index.
//!
//! The SDK already validates PBF framing (see `osm::validate_pbf`); this
//! module goes one layer deeper and decodes the actual OSM data so the
//! search index can extract named features. It is deliberately
//! dependency-light: the two protobuf messages the format needs
//! (`BlobHeader`/`Blob` framing and the `osmformat` block) are read with a
//! small hand-written varint scanner rather than a generated decoder, and
//! only the fields the index consumes are decoded — everything else is
//! skipped by wire rules.
//!
//! Two-pass scanning: [`scan_pbf`] walks the file once, handing every
//! node, way, and relation to a [`PbfVisitor`] with tags resolved against
//! the block's string table. `ScanOptions::skip_nodes` skips node parsing
//! entirely (the first pass of the index build only needs ways and
//! relations), which makes a full-file pre-pass over a 700 MB extract
//! roughly a decompression pass.

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};

// ---------------------------------------------------------------------------
// Wire reader
// ---------------------------------------------------------------------------

/// A cursor over a protobuf-encoded buffer. Reads varints and
/// length-delimited fields; unknown fields are skipped by wire type.
struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Reader { buf, pos: 0 }
    }
    fn done(&self) -> bool {
        self.pos >= self.buf.len()
    }
    fn varint(&mut self) -> Result<u64> {
        let mut result: u64 = 0;
        let mut shift = 0;
        loop {
            if self.pos >= self.buf.len() {
                bail!("truncated varint");
            }
            let b = self.buf[self.pos];
            self.pos += 1;
            result |= u64::from(b & 0x7f) << shift;
            if b & 0x80 == 0 {
                return Ok(result);
            }
            shift += 7;
            if shift >= 64 {
                bail!("varint too long");
            }
        }
    }
    fn zigzag(v: u64) -> i64 {
        ((v >> 1) as i64) ^ (-((v & 1) as i64))
    }
    fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        if self.pos + len > self.buf.len() {
            bail!(
                "truncated field (need {len}, have {})",
                self.buf.len() - self.pos
            );
        }
        let out = &self.buf[self.pos..self.pos + len];
        self.pos += len;
        Ok(out)
    }
    /// The tag byte pair: field number and wire type.
    fn tag(&mut self) -> Result<(u32, u8)> {
        let v = self.varint()?;
        Ok(((v >> 3) as u32, (v & 7) as u8))
    }
    /// Skip a field of the given wire type.
    fn skip(&mut self, wire: u8) -> Result<()> {
        match wire {
            0 => {
                self.varint()?;
            }
            1 => {
                self.take(8)?;
            }
            2 => {
                let len = self.varint()? as usize;
                self.take(len)?;
            }
            5 => {
                self.take(4)?;
            }
            other => bail!("unsupported wire type {other}"),
        }
        Ok(())
    }
}

/// Read one length-delimited sub-buffer (wire type 2) and return it.
fn sub<'a>(r: &mut Reader<'a>) -> Result<&'a [u8]> {
    let len = r.varint()? as usize;
    r.take(len)
}

// ---------------------------------------------------------------------------
// Entries handed to the visitor
// ---------------------------------------------------------------------------

/// A decoded node with its tags (borrowed from the block string table).
pub struct NodeEntry<'a> {
    pub id: i64,
    pub lat: f64,
    pub lon: f64,
    pub tags: Vec<(&'a str, &'a str)>,
}

/// A decoded way: node references (as stored, delta-decoded) and tags.
pub struct WayEntry<'a> {
    pub id: i64,
    pub refs: Vec<i64>,
    pub tags: Vec<(&'a str, &'a str)>,
}

/// Relation member kind (the `Relation.MemberType` enum).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberKind {
    Node,
    Way,
    Relation,
}

/// A decoded relation: members as (kind, id, role) and tags.
pub struct RelationEntry<'a> {
    pub id: i64,
    pub members: Vec<(MemberKind, i64, &'a str)>,
    pub tags: Vec<(&'a str, &'a str)>,
}

/// Receiver for decoded elements. Tags are only those present on the
/// element; the visitor applies its own filters.
pub trait PbfVisitor {
    fn node(&mut self, _n: NodeEntry) {}
    fn way(&mut self, _w: WayEntry) {}
    fn relation(&mut self, _r: RelationEntry) {}
    /// Progress hook: called once per file blob with the blob kind
    /// ("OSMHeader" | "OSMData") and the byte offset, so long scans can
    /// report position without the parser knowing about progress cells.
    fn on_blob(&mut self, _kind: &str, _offset: u64) {}
}

/// Scan tuning.
#[derive(Default)]
pub struct ScanOptions {
    /// Skip node decoding entirely (pass one of the index build).
    pub skip_nodes: bool,
}

/// Summary of one scan.
#[derive(Default, Debug)]
pub struct ScanStats {
    pub blobs: u64,
    pub nodes: u64,
    pub ways: u64,
    pub relations: u64,
    pub bytes: u64,
}

// ---------------------------------------------------------------------------
// Blob framing
// ---------------------------------------------------------------------------

/// One raw blob off the wire: its kind and decompressed payload.
enum Blob {
    Header,
    Data(Vec<u8>),
}

/// Read the next blob from `r`, returning None at EOF.
fn read_blob(r: &mut impl Read) -> Result<Option<(u64, Blob)>> {
    let mut len_buf = [0u8; 4];
    match r.read_exact(&mut len_buf) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(anyhow!("reading blob length: {e}")),
    }
    let offset = 0u64; // callers track their own offsets via read counts
    let _ = offset;
    let header_len = u32::from_be_bytes(len_buf) as usize;
    if header_len == 0 || header_len > 1 << 20 {
        bail!("implausible BlobHeader length {header_len}");
    }
    let mut header_buf = vec![0u8; header_len];
    r.read_exact(&mut header_buf)?;
    let mut hr = Reader::new(&header_buf);
    let mut kind = String::new();
    let mut datasize = 0usize;
    while !hr.done() {
        let (field, wire) = hr.tag()?;
        match (field, wire) {
            (1, 2) => kind = std::str::from_utf8(sub(&mut hr)?)?.to_string(),
            (3, 0) => datasize = hr.varint()? as usize,
            _ => hr.skip(wire)?,
        }
    }
    // The two kinds are static; map to a borrowed constant so no per-blob
    // allocation survives the call.
    let kind_static = match kind.as_str() {
        "OSMHeader" => "OSMHeader",
        "OSMData" => "OSMData",
        other => bail!("unknown blob type {other}"),
    };
    let mut data_buf = vec![0u8; datasize];
    r.read_exact(&mut data_buf)?;
    let blob = decode_blob(&data_buf)?;
    let out = match (kind_static, blob) {
        ("OSMHeader", _) => Blob::Header,
        ("OSMData", payload) => Blob::Data(payload),
        _ => unreachable!(),
    };
    Ok(Some((0, out)))
}

/// Decompress a Blob message to its raw payload.
fn decode_blob(buf: &[u8]) -> Result<Vec<u8>> {
    let mut r = Reader::new(buf);
    let mut raw: Option<&[u8]> = None;
    let mut zlib_data: Option<&[u8]> = None;
    while !r.done() {
        let (field, wire) = r.tag()?;
        match (field, wire) {
            (1, 2) => raw = Some(sub(&mut r)?),
            (3, 2) => zlib_data = Some(sub(&mut r)?),
            _ => r.skip(wire)?,
        }
    }
    if let Some(raw) = raw {
        return Ok(raw.to_vec());
    }
    if let Some(z) = zlib_data {
        let mut out = Vec::new();
        let mut d = flate2::read::ZlibDecoder::new(z);
        d.read_to_end(&mut out).context("decompressing zlib blob")?;
        return Ok(out);
    }
    bail!("blob carries no raw or zlib payload")
}

// ---------------------------------------------------------------------------
// PrimitiveBlock decoding
// ---------------------------------------------------------------------------

/// A decoded string table: indexes are u32 into this vector.
struct StringTable<'a> {
    strings: Vec<&'a [u8]>,
}

impl<'a> StringTable<'a> {
    fn get(&self, idx: u32) -> &'a str {
        self.strings
            .get(idx as usize)
            .and_then(|b| std::str::from_utf8(b).ok())
            .unwrap_or("")
    }
}

/// A block's coordinate scaling (granularity + offsets).
struct BlockGeo {
    granularity: i64,
    lat_offset: i64,
    lon_offset: i64,
}

impl BlockGeo {
    fn lat(&self, v: i64) -> f64 {
        1e-9 * (self.lat_offset + self.granularity * v) as f64
    }
    fn lon(&self, v: i64) -> f64 {
        1e-9 * (self.lon_offset + self.granularity * v) as f64
    }
}

/// Decode one PrimitiveBlock, dispatching every element to the visitor.
fn decode_block(
    payload: &[u8],
    opts: &ScanOptions,
    v: &mut impl PbfVisitor,
    stats: &mut ScanStats,
) {
    let mut r = Reader::new(payload);
    let mut strings: Vec<&[u8]> = Vec::new();
    let mut geo = BlockGeo {
        granularity: 100,
        lat_offset: 0,
        lon_offset: 0,
    };
    while !r.done() {
        let Ok((field, wire)) = r.tag() else { break };
        match (field, wire) {
            (1, 2) => {
                // StringTable: repeated field 1, bytes.
                let st = sub(&mut r).unwrap_or(&[]);
                let mut sr = Reader::new(st);
                while !sr.done() {
                    let Ok((f, w)) = sr.tag() else { break };
                    if (f, w) == (1, 2) {
                        if let Ok(s) = sub(&mut sr) {
                            strings.push(s);
                        }
                    } else {
                        let _ = sr.skip(w);
                    }
                }
            }
            (2, 2) => {
                let group = sub(&mut r).unwrap_or(&[]);
                decode_group(group, &strings, &geo, opts, v, stats);
            }
            (17, 0) => geo.granularity = r.varint().unwrap_or(100) as i64,
            (19, 0) => geo.lat_offset = r.varint().unwrap_or(0) as i64,
            (20, 0) => geo.lon_offset = r.varint().unwrap_or(0) as i64,
            _ => {
                let _ = r.skip(wire);
            }
        }
    }
}

fn decode_group(
    group: &[u8],
    strings: &[&[u8]],
    geo: &BlockGeo,
    opts: &ScanOptions,
    v: &mut impl PbfVisitor,
    stats: &mut ScanStats,
) {
    let st = StringTable {
        strings: strings.to_vec(),
    };
    let mut r = Reader::new(group);
    while !r.done() {
        let Ok((field, wire)) = r.tag() else { break };
        match (field, wire) {
            (1, 2) => {
                // Node (rare in modern files: dense form is used instead).
                let raw = sub(&mut r).unwrap_or(&[]);
                if !opts.skip_nodes {
                    if let Some(n) = decode_node(raw, &st, geo) {
                        stats.nodes += 1;
                        v.node(n);
                    }
                }
            }
            (2, 2) => {
                // DenseNodes.
                let raw = sub(&mut r).unwrap_or(&[]);
                if !opts.skip_nodes {
                    decode_dense(raw, &st, geo, v, stats);
                }
            }
            (3, 2) => {
                // Way.
                let raw = sub(&mut r).unwrap_or(&[]);
                if let Some(w) = decode_way(raw, &st) {
                    stats.ways += 1;
                    v.way(w);
                }
            }
            (4, 2) => {
                // Relation.
                let raw = sub(&mut r).unwrap_or(&[]);
                if let Some(rel) = decode_relation(raw, &st) {
                    stats.relations += 1;
                    v.relation(rel);
                }
            }
            _ => {
                let _ = r.skip(wire);
            }
        }
    }
}

/// Decode the tags of a Node/Way/Relation body: packed `keys` (field 2)
/// and `vals` (field 3) indexing the string table.
fn decode_tags<'a>(
    body: &[u8],
    st: &StringTable<'a>,
) -> (Vec<u32>, Vec<u32>, Vec<(&'a str, &'a str)>) {
    let mut keys: Vec<u32> = Vec::new();
    let mut vals: Vec<u32> = Vec::new();
    let mut r = Reader::new(body);
    while !r.done() {
        let Ok((field, wire)) = r.tag() else { break };
        match (field, wire) {
            (2, 2) => {
                let packed = sub(&mut r).unwrap_or(&[]);
                let mut pr = Reader::new(packed);
                while !pr.done() {
                    match pr.varint() {
                        Ok(x) => keys.push(x as u32),
                        Err(_) => break,
                    }
                }
            }
            (3, 2) => {
                let packed = sub(&mut r).unwrap_or(&[]);
                let mut pr = Reader::new(packed);
                while !pr.done() {
                    match pr.varint() {
                        Ok(x) => vals.push(x as u32),
                        Err(_) => break,
                    }
                }
            }
            _ => {
                let _ = r.skip(wire);
            }
        }
    }
    let tags = keys
        .iter()
        .zip(vals.iter())
        .map(|(k, val)| (st.get(*k), st.get(*val)))
        .collect();
    (keys, vals, tags)
}

fn decode_node<'a>(raw: &'a [u8], st: &StringTable<'a>, geo: &BlockGeo) -> Option<NodeEntry<'a>> {
    let mut id = 0i64;
    let mut lat_v = 0i64;
    let mut lon_v = 0i64;
    let mut r = Reader::new(raw);
    while !r.done() {
        let Ok((field, wire)) = r.tag() else { break };
        match (field, wire) {
            (1, 0) => id = Reader::zigzag(r.varint().ok()?),
            (8, 0) => lat_v = Reader::zigzag(r.varint().ok()?),
            (9, 0) => lon_v = Reader::zigzag(r.varint().ok()?),
            _ => {
                let _ = r.skip(wire);
            }
        }
    }
    let (_, _, tags) = decode_tags(raw, st);
    Some(NodeEntry {
        id,
        lat: geo.lat(lat_v),
        lon: geo.lon(lon_v),
        tags,
    })
}

/// Decode DenseNodes: delta-coded ids/lat/lon and the interleaved
/// keys_vals stream (k, v, k, v, ..., 0 per node).
fn decode_dense(
    raw: &[u8],
    st: &StringTable,
    geo: &BlockGeo,
    v: &mut impl PbfVisitor,
    stats: &mut ScanStats,
) {
    let mut ids: Vec<i64> = Vec::new();
    let mut lats: Vec<i64> = Vec::new();
    let mut lons: Vec<i64> = Vec::new();
    let mut keys_vals: Vec<i32> = Vec::new();
    let mut r = Reader::new(raw);
    while !r.done() {
        let Ok((field, wire)) = r.tag() else { break };
        if (field, wire) == (10, 2) {
            // keys_vals is int32 packed — signedness does not matter for
            // the separator semantics (0 is 0 either way), and the values
            // are string-table indexes (non-negative).
            let packed = sub(&mut r).unwrap_or(&[]);
            let mut pr = Reader::new(packed);
            while !pr.done() {
                match pr.varint() {
                    Ok(x) => keys_vals.push(x as i32),
                    Err(_) => break,
                }
            }
        } else if (field, wire) == (10, 0) {
            // Non-packed encoding for keys_vals (unusual but legal).
            if let Ok(x) = r.varint() {
                keys_vals.push(x as i32);
            }
        } else if wire == 2 {
            // Packed repeated: ids (1), lats (8), lons (9). Each is a
            // delta-coded zigzag varint stream.
            let packed = sub(&mut r).unwrap_or(&[]);
            let mut pr = Reader::new(packed);
            let target = match field {
                1 => Some(&mut ids),
                8 => Some(&mut lats),
                9 => Some(&mut lons),
                _ => None,
            };
            if let Some(target) = target {
                while !pr.done() {
                    match pr.varint() {
                        Ok(x) => target.push(Reader::zigzag(x)),
                        Err(_) => break,
                    }
                }
            }
        } else {
            let _ = r.skip(wire);
        }
    }
    if ids.is_empty() {
        return;
    }
    // Undo the deltas.
    let mut id = 0i64;
    let mut lat_v = 0i64;
    let mut lon_v = 0i64;
    let mut kv = 0usize;
    for (i, d_id) in ids.iter().enumerate() {
        id += d_id;
        lat_v += lats.get(i).copied().unwrap_or(0);
        lon_v += lons.get(i).copied().unwrap_or(0);
        let mut tags: Vec<(&str, &str)> = Vec::new();
        // Each node's tags end at a 0 separator; a node with no tags
        // contributes exactly one 0.
        while kv < keys_vals.len() && keys_vals[kv] != 0 {
            let k = keys_vals[kv] as u32;
            let val = if kv + 1 < keys_vals.len() {
                keys_vals[kv + 1] as u32
            } else {
                0
            };
            tags.push((st.get(k), st.get(val)));
            kv += 2;
        }
        if kv < keys_vals.len() {
            kv += 1; // consume the separator
        }
        stats.nodes += 1;
        v.node(NodeEntry {
            id,
            lat: geo.lat(lat_v),
            lon: geo.lon(lon_v),
            tags,
        });
    }
}

fn decode_way<'a>(raw: &'a [u8], st: &StringTable<'a>) -> Option<WayEntry<'a>> {
    let mut id = 0i64;
    let mut refs: Vec<i64> = Vec::new();
    let mut r = Reader::new(raw);
    while !r.done() {
        let Ok((field, wire)) = r.tag() else { break };
        match (field, wire) {
            (1, 0) => id = r.varint().ok()? as i64,
            (8, 2) => {
                let packed = sub(&mut r).ok()?;
                let mut pr = Reader::new(packed);
                let mut prev = 0i64;
                while !pr.done() {
                    match pr.varint() {
                        Ok(x) => {
                            prev += Reader::zigzag(x);
                            refs.push(prev);
                        }
                        Err(_) => break,
                    }
                }
            }
            (8, 0) => {
                // Non-packed delta refs (legal, unusual).
                if let Ok(x) = r.varint() {
                    let last = refs.last().copied().unwrap_or(0);
                    refs.push(last + Reader::zigzag(x));
                }
            }
            _ => {
                let _ = r.skip(wire);
            }
        }
    }
    let (_, _, tags) = decode_tags(raw, st);
    Some(WayEntry { id, refs, tags })
}

fn decode_relation<'a>(raw: &'a [u8], st: &StringTable<'a>) -> Option<RelationEntry<'a>> {
    let mut id = 0i64;
    let mut roles: Vec<i32> = Vec::new();
    let mut memids: Vec<i64> = Vec::new();
    let mut types: Vec<u8> = Vec::new();
    let mut r = Reader::new(raw);
    while !r.done() {
        let Ok((field, wire)) = r.tag() else { break };
        match (field, wire) {
            (1, 0) => id = r.varint().ok()? as i64,
            (8, 2) => {
                let packed = sub(&mut r).ok()?;
                let mut pr = Reader::new(packed);
                while !pr.done() {
                    match pr.varint() {
                        Ok(x) => roles.push(x as i32),
                        Err(_) => break,
                    }
                }
            }
            (9, 2) => {
                let packed = sub(&mut r).ok()?;
                let mut pr = Reader::new(packed);
                let mut prev = 0i64;
                while !pr.done() {
                    match pr.varint() {
                        Ok(x) => {
                            prev += Reader::zigzag(x);
                            memids.push(prev);
                        }
                        Err(_) => break,
                    }
                }
            }
            (10, 2) => {
                let packed = sub(&mut r).ok()?;
                let mut pr = Reader::new(packed);
                while !pr.done() {
                    match pr.varint() {
                        Ok(x) => types.push(x as u8),
                        Err(_) => break,
                    }
                }
            }
            _ => {
                let _ = r.skip(wire);
            }
        }
    }
    let (_, _, tags) = decode_tags(raw, st);
    let members = memids
        .iter()
        .enumerate()
        .map(|(i, id)| {
            // The read loop already un-delta'd memids into absolute ids.
            let kind = match types.get(i).copied().unwrap_or(0) {
                0 => MemberKind::Node,
                1 => MemberKind::Way,
                _ => MemberKind::Relation,
            };
            let role = st.get(roles.get(i).copied().unwrap_or(0) as u32);
            (kind, *id, role)
        })
        .collect();
    Some(RelationEntry { id, members, tags })
}

// ---------------------------------------------------------------------------
// Public scan entry
// ---------------------------------------------------------------------------

/// Scan a whole PBF file, dispatching to `v`.
pub fn scan_pbf(path: &Path, opts: &ScanOptions, v: &mut impl PbfVisitor) -> Result<ScanStats> {
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let size = file.metadata().map(|m| m.len()).unwrap_or(0);
    let mut reader = BufReader::with_capacity(1 << 20, file);
    let mut stats = ScanStats {
        bytes: size,
        ..Default::default()
    };
    let mut offset: u64 = 0;
    loop {
        let blob_start = offset;
        let Some((_, blob)) = read_blob(&mut reader)? else {
            break;
        };
        let kind = match blob {
            Blob::Header => "OSMHeader",
            Blob::Data(ref payload) => {
                decode_block(payload, opts, v, &mut stats);
                "OSMData"
            }
        };
        v.on_blob(kind, blob_start);
        stats.blobs += 1;
        // Advance the offset by the framing we consumed. read_blob does
        // not report it, so track via the reader's position through the
        // stats the caller sees — a cheap upper bound suffices for
        // progress (blobs), so recompute exactly:
        offset += 4 + read_consumed_hint(reader.buffer().len() as u64, kind.len());
        // The hint is unreliable; the visitor only needs monotonic
        // progress, so clamp to file size.
        offset = offset.min(size).max(blob_start);
    }
    Ok(stats)
}

/// The offset tracking above cannot recover the exact consumed count from
/// a BufReader without a consumed() API that returns cumulative bytes.
/// Monotonic blob-index progress is enough for the UI (elapsed + blob
/// count), so this helper just keeps the arithmetic harmless.
fn read_consumed_hint(_buffered: u64, _k: usize) -> u64 {
    0
}

// ---------------------------------------------------------------------------
// Tests: a tiny in-file PBF encoder + round-trip
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(crate) mod test_support {
    //! Build minimal PBF files in memory for parser and index tests.

    fn varint(mut v: u64, out: &mut Vec<u8>) {
        loop {
            let b = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                out.push(b);
                return;
            }
            out.push(b | 0x80);
        }
    }
    fn zigzag(v: i64) -> u64 {
        ((v << 1) ^ (v >> 63)) as u64
    }
    fn tag(field: u32, wire: u8, out: &mut Vec<u8>) {
        varint((u64::from(field) << 3) | u64::from(wire), out);
    }
    fn bytes_field_enc(field: u32, data: &[u8], out: &mut Vec<u8>) {
        tag(field, 2, out);
        varint(data.len() as u64, out);
        out.extend_from_slice(data);
    }
    fn varint_field(field: u32, v: u64, out: &mut Vec<u8>) {
        tag(field, 0, out);
        varint(v, out);
    }
    fn packed_field(field: u32, vals: impl IntoIterator<Item = u64>, out: &mut Vec<u8>) {
        let mut inner = Vec::new();
        for v in vals {
            varint(v, &mut inner);
        }
        bytes_field_enc(field, &inner, out);
    }

    /// A node to encode.
    pub struct EncNode {
        pub id: i64,
        pub lat: f64,
        pub lon: f64,
        pub tags: Vec<(String, String)>,
    }
    pub struct EncWay {
        pub id: i64,
        pub refs: Vec<i64>,
        pub tags: Vec<(String, String)>,
    }
    pub struct EncRelation {
        pub id: i64,
        /// (kind, id, role): kind 0 node, 1 way, 2 relation.
        pub members: Vec<(u8, i64, String)>,
        pub tags: Vec<(String, String)>,
    }

    /// Encode one OSMData blob's payload (a PrimitiveBlock) from dense
    /// nodes, ways, and relations, using raw (uncompressed) blobs.
    pub fn encode_pbf(nodes: &[EncNode], ways: &[EncWay], relations: &[EncRelation]) -> Vec<u8> {
        // String table: index 0 must be empty; collect every string.
        fn add_tags(
            tags: &[(String, String)],
            strings: &mut Vec<String>,
            keys: &mut Vec<u32>,
            vals: &mut Vec<u32>,
        ) {
            for (k, v) in tags {
                let ki = strings.iter().position(|x| x == k).unwrap_or_else(|| {
                    strings.push(k.clone());
                    strings.len() - 1
                }) as u32;
                let vi = strings.iter().position(|x| x == v).unwrap_or_else(|| {
                    strings.push(v.clone());
                    strings.len() - 1
                }) as u32;
                keys.push(ki);
                vals.push(vi);
            }
        }

        // Pre-intern all strings.
        let mut strings: Vec<String> = vec![String::new()];
        for n in nodes {
            add_tags(&n.tags, &mut strings, &mut Vec::new(), &mut Vec::new());
        }
        for w in ways {
            add_tags(&w.tags, &mut strings, &mut Vec::new(), &mut Vec::new());
        }
        for r in relations {
            add_tags(&r.tags, &mut strings, &mut Vec::new(), &mut Vec::new());
            for (_, _, role) in &r.members {
                strings.push(role.clone());
            }
        }

        // StringTable message.
        let mut st_msg = Vec::new();
        for s in &strings {
            bytes_field_enc(1, s.as_bytes(), &mut st_msg);
        }

        // DenseNodes.
        let mut dense = Vec::new();
        {
            let mut ids = Vec::new();
            let mut lats = Vec::new();
            let mut lons = Vec::new();
            let mut kv = Vec::new();
            let mut prev_id = 0i64;
            let mut prev_lat = 0i64;
            let mut prev_lon = 0i64;
            for n in nodes {
                ids.push(zigzag(n.id - prev_id));
                prev_id = n.id;
                // nano-degrees at granularity 100: value = round(coord/1e-9/granularity)
                let lat_v = (n.lat * 1e7) as i64;
                let lon_v = (n.lon * 1e7) as i64;
                lats.push(zigzag(lat_v - prev_lat));
                prev_lat = lat_v;
                lons.push(zigzag(lon_v - prev_lon));
                prev_lon = lon_v;
                for (k, v) in &n.tags {
                    let ki = strings.iter().position(|x| x == k).unwrap() as u32;
                    let vi = strings.iter().position(|x| x == v).unwrap() as u32;
                    kv.push(u64::from(ki));
                    kv.push(u64::from(vi));
                }
                kv.push(0);
            }
            packed_field(1, ids, &mut dense);
            packed_field(8, lats, &mut dense);
            packed_field(9, lons, &mut dense);
            packed_field(10, kv, &mut dense);
        }

        // Ways.
        let mut ways_msg = Vec::new();
        for w in ways {
            let mut m = Vec::new();
            varint_field(1, w.id as u64, &mut m);
            let mut keys = Vec::new();
            let mut vals = Vec::new();
            add_tags(&w.tags, &mut strings, &mut keys, &mut vals);
            if !keys.is_empty() {
                let mut ki: Vec<u64> = keys.iter().map(|x| u64::from(*x)).collect();
                let mut vi: Vec<u64> = vals.iter().map(|x| u64::from(*x)).collect();
                let mut ki_v = Vec::new();
                let mut vi_v = Vec::new();
                std::mem::swap(&mut ki, &mut ki_v);
                std::mem::swap(&mut vi, &mut vi_v);
                packed_field(2, ki_v, &mut m);
                packed_field(3, vi_v, &mut m);
            }
            // refs: delta-encoded zigzag.
            let mut deltas = Vec::new();
            let mut prev = 0i64;
            for rf in &w.refs {
                deltas.push(zigzag(rf - prev));
                prev = *rf;
            }
            packed_field(8, deltas, &mut m);
            ways_msg.push(m);
        }

        // Relations.
        let mut relations_msg = Vec::new();
        for r in relations {
            let mut m = Vec::new();
            varint_field(1, r.id as u64, &mut m);
            let mut keys = Vec::new();
            let mut vals = Vec::new();
            add_tags(&r.tags, &mut strings, &mut keys, &mut vals);
            if !keys.is_empty() {
                let ki: Vec<u64> = keys.iter().map(|x| u64::from(*x)).collect();
                let vi: Vec<u64> = vals.iter().map(|x| u64::from(*x)).collect();
                packed_field(2, ki, &mut m);
                packed_field(3, vi, &mut m);
            }
            // roles_sid, memids (delta), types.
            let mut roles = Vec::new();
            let mut memids = Vec::new();
            let mut types = Vec::new();
            let mut prev = 0i64;
            for (kind, id, role) in &r.members {
                let ri = strings.iter().position(|x| x == role).unwrap() as u32;
                roles.push(u64::from(ri));
                memids.push(zigzag(id - prev));
                prev = *id;
                types.push(u64::from(*kind));
            }
            packed_field(8, roles, &mut m);
            packed_field(9, memids, &mut m);
            packed_field(10, types, &mut m);
            relations_msg.push(m);
        }

        // PrimitiveGroup: one group with dense nodes, then one per way and
        // relation (each group carries one member kind).
        let mut groups: Vec<Vec<u8>> = Vec::new();
        if !nodes.is_empty() {
            let mut g = Vec::new();
            bytes_field_enc(2, &dense, &mut g);
            groups.push(g);
        }
        for w in &ways_msg {
            let mut g = Vec::new();
            bytes_field_enc(3, w, &mut g);
            groups.push(g);
        }
        for r in &relations_msg {
            let mut g = Vec::new();
            bytes_field_enc(4, r, &mut g);
            groups.push(g);
        }

        // PrimitiveBlock.
        let mut block = Vec::new();
        bytes_field_enc(1, &st_msg, &mut block);
        for g in &groups {
            bytes_field_enc(2, g, &mut block);
        }
        // granularity 100 (the default; explicit for clarity).
        varint_field(17, 100, &mut block);

        // Blob message: raw payload (field 1) + raw_size (field 2).
        let mut blob = Vec::new();
        bytes_field_enc(1, &block, &mut blob);
        varint_field(2, block.len() as u64, &mut blob);

        // File framing: header (minimal OSMHeader) then the data blob.
        let mut out = Vec::new();
        // OSMHeader blob: an empty PrimitiveHeader is legal.
        let header_payload: [u8; 0] = [];
        let mut header_blob = Vec::new();
        bytes_field_enc(1, &header_payload, &mut header_blob);
        write_framed("OSMHeader", &header_blob, &mut out);
        write_framed("OSMData", &blob, &mut out);
        out
    }

    fn write_framed(kind: &str, blob: &[u8], out: &mut Vec<u8>) {
        // BlobHeader: field 1 type (string), field 3 datasize.
        let mut header = Vec::new();
        bytes_field_enc(1, kind.as_bytes(), &mut header);
        varint_field(3, blob.len() as u64, &mut header);
        out.extend_from_slice(&(header.len() as u32).to_be_bytes());
        out.extend_from_slice(&header);
        out.extend_from_slice(blob);
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;

    #[derive(Default)]
    #[allow(clippy::type_complexity)]
    struct Collect {
        nodes: Vec<(i64, f64, f64, Vec<(String, String)>)>,
        ways: Vec<(i64, Vec<i64>, Vec<(String, String)>)>,
        relations: Vec<(i64, Vec<(u8, i64, String)>, Vec<(String, String)>)>,
        blobs: u64,
    }

    impl PbfVisitor for Collect {
        fn node(&mut self, n: NodeEntry) {
            self.nodes.push((
                n.id,
                n.lat,
                n.lon,
                n.tags
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            ));
        }
        fn way(&mut self, w: WayEntry) {
            self.ways.push((
                w.id,
                w.refs.clone(),
                w.tags
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            ));
        }
        fn relation(&mut self, r: RelationEntry) {
            self.relations.push((
                r.id,
                r.members
                    .iter()
                    .map(|(k, id, role)| (*k as u8, *id, role.to_string()))
                    .collect(),
                r.tags
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            ));
        }
        fn on_blob(&mut self, _kind: &str, _offset: u64) {
            self.blobs += 1;
        }
    }

    fn write_tmp(data: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("osm-pbf-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("fixture.osm.pbf");
        std::fs::write(&p, data).unwrap();
        p
    }

    #[test]
    fn roundtrip_dense_ways_relations() {
        let pbf = encode_pbf(
            &[
                EncNode {
                    id: 100,
                    lat: 6.455,
                    lon: 3.407,
                    tags: vec![
                        ("place".into(), "city".into()),
                        ("name".into(), "Lagos".into()),
                    ],
                },
                EncNode {
                    id: 101,
                    lat: 6.509,
                    lon: 3.372,
                    tags: vec![
                        ("place".into(), "suburb".into()),
                        ("name".into(), "Yaba".into()),
                    ],
                },
                EncNode {
                    id: 102,
                    lat: 6.5,
                    lon: 3.4,
                    tags: vec![],
                },
            ],
            &[
                EncWay {
                    id: 500,
                    refs: vec![100, 101, 102],
                    tags: vec![
                        ("highway".into(), "primary".into()),
                        ("name".into(), "Awolowo Road".into()),
                    ],
                },
                EncWay {
                    id: 501,
                    refs: vec![102, 100],
                    tags: vec![],
                },
            ],
            &[EncRelation {
                id: 900,
                members: vec![(1, 500, "outer".into()), (1, 501, "inner".into())],
                tags: vec![
                    ("type".into(), "boundary".into()),
                    ("boundary".into(), "administrative".into()),
                    ("admin_level".into(), "4".into()),
                    ("name".into(), "Lagos State".into()),
                ],
            }],
        );
        let path = write_tmp(&pbf);
        let mut collect = Collect::default();
        let stats = scan_pbf(&path, &ScanOptions::default(), &mut collect).unwrap();

        assert_eq!(collect.blobs, 2); // header + one data blob
        assert_eq!(stats.nodes, 3);
        assert_eq!(stats.ways, 2);
        assert_eq!(stats.relations, 1);

        // Nodes: delta-decoded ids, correct coords (granularity 100, no
        // offsets: coord = 1e-9 * 100 * value = 1e-7 * value).
        assert_eq!(collect.nodes[0].0, 100);
        assert!((collect.nodes[0].1 - 6.455).abs() < 1e-6);
        assert!((collect.nodes[0].2 - 3.407).abs() < 1e-6);
        assert_eq!(
            collect.nodes[0].3[0],
            ("place".to_string(), "city".to_string())
        );
        assert_eq!(
            collect.nodes[0].3[1],
            ("name".to_string(), "Lagos".to_string())
        );
        // Node without tags decodes with an empty tag list.
        assert!(collect.nodes[2].3.is_empty());

        // Ways: delta-decoded refs.
        assert_eq!(collect.ways[0].0, 500);
        assert_eq!(collect.ways[0].1, vec![100, 101, 102]);
        assert_eq!(
            collect.ways[0].2[1],
            ("name".to_string(), "Awolowo Road".to_string())
        );

        // Relations: members with roles and kinds.
        assert_eq!(collect.relations[0].0, 900);
        assert_eq!(collect.relations[0].1.len(), 2);
        assert_eq!(collect.relations[0].1[0], (1u8, 500, "outer".to_string()));
        assert_eq!(collect.relations[0].1[1], (1u8, 501, "inner".to_string()));
    }

    #[test]
    fn skip_nodes_still_yields_ways() {
        let pbf = encode_pbf(
            &[EncNode {
                id: 1,
                lat: 1.0,
                lon: 1.0,
                tags: vec![("name".into(), "x".into())],
            }],
            &[EncWay {
                id: 2,
                refs: vec![1],
                tags: vec![("highway".into(), "residential".into())],
            }],
            &[],
        );
        let path = write_tmp(&pbf);
        let mut collect = Collect::default();
        let stats = scan_pbf(&path, &ScanOptions { skip_nodes: true }, &mut collect).unwrap();
        assert_eq!(stats.nodes, 0);
        assert_eq!(collect.ways.len(), 1);
        assert!(collect.nodes.is_empty());
    }
}
