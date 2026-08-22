//! Reusable core types and operations for Herdr Workspacer.

mod candidate;
mod fuzzy;
mod mru;
mod zoxide;

/// Workspace and directory candidates plus merge and path operations.
pub use candidate::{Candidate, CandidateKind, Workspace, merge_candidates, normalize_path};
/// Fuzzy filtering that preserves the caller's candidate order.
pub use fuzzy::filter_indices;
/// Persistent most-recently-used workspace state.
pub use mru::{MruState, MruStore};
/// zoxide candidate loading and parsed records.
pub use zoxide::{ZoxideEntry, ZoxideSource, load as load_zoxide};
