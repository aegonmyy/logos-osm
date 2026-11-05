//! Live progress reporting for long-running host/fetch jobs.
//!
//! A [`JobProgress`] is a cheap shared cell (`Arc` over atomics) that the
//! download/upload streams tick as bytes flow. The FFI job layer attaches one
//! to a host job and the `job_status` op snapshots it back to the UI, so the
//! task panel can show "downloading 45 / 80 MB" instead of a bare
//! "running" + elapsed timer.
//!
//! The cell is `Option`-al everywhere: the CLI and tests build clients
//! without one (the default), so nothing here is on the hot path of a
//! non-instrumented run.

use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;

/// Coarse lifecycle phase of a host job, surfaced to the UI as a label.
///
/// The integer is stable over the wire (the JSON encodes it as a number) so
/// the client and the QML stay in sync without string matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Phase {
    /// Pre-flight (resolving the region, fetching the published MD5).
    Starting = 0,
    /// Streaming the `.osm.pbf` from Geofabrik to the local cache.
    Downloading = 1,
    /// Streaming the verified file into Logos Storage.
    Storing = 2,
}

impl Phase {
    pub fn as_u8(self) -> u8 {
        self as u8
    }
    pub fn label(self) -> &'static str {
        match self {
            Phase::Starting => "starting",
            Phase::Downloading => "downloading",
            Phase::Storing => "storing",
        }
    }
}

/// Label for a raw phase byte (the form `job_status` stores). Falls back to
/// "starting" for any value not in the enum, so a future-added phase never
/// panics the UI.
pub fn phase_label(phase: u8) -> &'static str {
    match phase {
        1 => Phase::Downloading.label(),
        2 => Phase::Storing.label(),
        _ => Phase::Starting.label(),
    }
}

/// A shared, lock-free progress cell. `done`/`total` are bytes; `total` is `0`
/// when unknown (e.g. a chunked transfer with no `Content-Length`).
#[derive(Default, Debug)]
pub struct JobProgress {
    phase: AtomicU8,
    done: AtomicU64,
    total: AtomicU64,
}

impl JobProgress {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
    /// Set the phase and reset the byte counters (a new phase starts at 0).
    pub fn begin(&self, phase: Phase, total: u64) {
        self.phase.store(phase.as_u8(), Ordering::Relaxed);
        self.total.store(total, Ordering::Relaxed);
        self.done.store(0, Ordering::Relaxed);
    }
    /// Add `n` bytes to the running counter. Cheap: a relaxed atomic store.
    pub fn add(&self, n: u64) {
        self.done.fetch_add(n, Ordering::Relaxed);
    }
    /// Read everything at once for a status snapshot.
    pub fn snapshot(&self) -> (u8, u64, u64) {
        (
            self.phase.load(Ordering::Relaxed),
            self.done.load(Ordering::Relaxed),
            self.total.load(Ordering::Relaxed),
        )
    }
}

/// The optional progress handle a streaming client carries. Set via the
/// `with_progress` builders; absent by default.
pub type Progress = Arc<JobProgress>;
