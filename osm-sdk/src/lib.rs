//! Reusable SDK for the OpenStreetMap distribution system (LP-0018 / PR #71).
//!
//! The SDK covers the full lifecycle the prize asks for:
//!
//! 1. **Discover** regions from the predefined, non-overlapping set
//!    ([`regions`], appendix A.4 of the prize) with live metadata from
//!    Geofabrik's index;
//! 2. **Host**: download a region's `.osm.pbf` from Geofabrik, verify it
//!    against Geofabrik's published MD5, and store it in Logos Storage;
//! 3. **Register** the snapshot on the LEZ registry program (single or
//!    batch), making it queryable by region / parent / CID;
//! 4. **Consume**: download from Logos Storage (Geofabrik fallback),
//!    re-verify the MD5, import locally, and check for updates.
//!
//! Layout (filled in as the build proceeds):
//! - [`regions`] — the frozen region set + Geofabrik URL layout;
//! - `geofabrik` — index fetch/parse, `.md5` fetch/parse (hash + version),
//!   streaming PBF download;
//! - `verify` — streaming MD5 of a downloaded file;
//! - `storage` — Logos Storage (Codex) streaming put/get;
//! - `registry` — the LEZ program client (deploy, register, query).
//!
//! Map data is public — nothing here encrypts; the integrity story is the
//! Geofabrik-published MD5 recorded on-chain next to the storage CID.

#![forbid(unsafe_code)]

pub mod geofabrik;
pub mod osm;
pub mod regions;
pub mod registry;
pub mod retry;
pub mod storage;
pub mod verify;

pub use osm::{BulkHostReport, HostedSnapshot, ImportSummary, OsmClient, UpdateStatus};
