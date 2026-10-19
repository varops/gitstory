//! GitStory formats.
//!
//! - The CLI envelope, error codes and exit codes: the automation contract
//!   (PRD §9.8), the same shape as ghf's.
//! - The `gitstory/v1` story schema and evidence events (PRD §7): the store
//!   format, export format and what agents read.
//!
//! Adapted from ghf's `ghf-proto`, not shared with it: GitStory works
//! without ghf (PRD §8.11).

mod envelope;
mod error;
pub mod evidence;
pub mod id;
pub mod record;
pub mod story;

pub use envelope::{ErrorBody, Response, SCHEMA_VERSION};
pub use error::{ErrorCode, ExitCode};
