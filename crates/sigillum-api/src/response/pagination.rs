//! Offset pagination metadata for list responses.
//!
//! List endpoints accept optional `limit`/`offset` query parameters (see
//! [`crate::request::PaginationQuery`]). When either parameter is supplied,
//! the response carries this additive envelope next to the (already filtered
//! and sorted) result window; when no pagination parameter is supplied the
//! field is absent entirely, so legacy responses stay byte-identical.

use serde::{Deserialize, Serialize};

/// Pagination window metadata for one list response.
///
/// - `total` — number of items after filtering, before the window is applied.
/// - `limit` — the requested page size; when only `offset` was supplied this
///   is the size of the unpaged remainder after the offset.
/// - `offset` — the requested number of items skipped.
/// - `has_more` — true when `offset + returned window length < total`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PaginationInfo {
    pub total: u64,
    pub limit: u32,
    pub offset: u32,
    pub has_more: bool,
}
