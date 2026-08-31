//! The fixed-width corpus contract: record layout, read-only sources, source
//! discovery and derived-destination separation.
//!
//! Every type here is read-only with respect to the source corpus: the one
//! writer, [`RecordWriter`], only ever creates the checked
//! [`DerivedDestination`] it is handed. See the crate-level documentation for
//! the immutable-source rule.

mod bfloat16;
mod derived;
mod discovery;
mod error;
mod reader;
mod shape;
mod source;
mod writer;

pub use derived::DerivedDestination;
pub use discovery::discover_sources;
pub use error::CorpusError;
pub use reader::RecordReader;
pub use shape::{RecordShape, ValueEncoding};
pub use source::SourceCorpus;
pub use writer::RecordWriter;
