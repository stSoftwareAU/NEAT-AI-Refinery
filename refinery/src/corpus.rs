//! The fixed-width corpus contract: record layout, read-only sources, source
//! discovery and derived-destination separation.
//!
//! Every type here is read-only with respect to the source corpus. See the
//! crate-level documentation for the immutable-source rule.

mod derived;
mod discovery;
mod error;
mod shape;
mod source;

pub use derived::DerivedDestination;
pub use discovery::discover_sources;
pub use error::CorpusError;
pub use shape::{RecordShape, ValueEncoding};
pub use source::SourceCorpus;
