//! The predefined, non-overlapping region set (prize appendix A.4).
//!
//! The canonical table lives in `osm_core::set` so the RISC0 guest embeds the
//! exact same list the SDK serves (one source of truth); this module
//! re-exports it as part of the SDK's public API. See `osm-core/src/set.rs`
//! for the region model, the frozen A.4.2 list, and the non-overlap
//! invariant.

pub use osm_core::set::*;
