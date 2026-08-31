//! The machinery every derived-corpus transform shares.
//!
//! A transform reads an immutable source corpus and publishes a derived one.
//! Everything that is true of *all* of them — how sources are discovered, how a
//! destination is proved separate from them, how the result is staged and
//! swapped in atomically, and how those steps fail — lives here rather than
//! inside any one transform.
//!
//! That is what makes transforms composable: [`sample`](crate::sample) and
//! [`quantise`](crate::quantise) publish corpora of the same shape, with the
//! same provenance, so one can be run over the output of the other with no
//! knowledge of either on the part of the caller.
//!
//! ```text
//! source corpus ──▶ transform ──▶ staging dir ──▶ atomic rename ──▶ derived corpus
//!                                (corpus + manifest)
//! ```

mod error;
mod scan;
mod staging;

pub use error::TransformError;
pub use scan::{corpus_files, file_bytes, resolved_source, source_file};
pub use staging::StagedCorpus;
