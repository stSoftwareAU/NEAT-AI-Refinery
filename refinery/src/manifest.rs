//! Transformation manifests — the provenance record of a derived corpus.
//!
//! Every derived corpus Refinery publishes carries a `manifest.json` beside it
//! recording how it was made: the source identity, the record geometry, the
//! transform and its parameters, the seed, the counts on both sides, the tool
//! version, the time, and a checksum of the published corpus. A corpus
//! produced by a [`pipeline`](crate::pipeline) records the ordered stages
//! beside that, first to last, because transforms do not generally commute.
//!
//! ```text
//! trainData-binary-sampler/
//! ├── manifest.json      ← provenance
//! └── sample-5.bin       ← the derived corpus
//! ```
//!
//! Three rules shape the design:
//!
//! 1. **The manifest travels with the corpus.** It is written into the staging
//!    directory before the publishing rename, so the atomic swap either brings
//!    both across or neither. A corpus is never published without provenance.
//! 2. **Nothing application-specific is invented.** Refinery records what it
//!    did. Anything a caller wants preserved — a GRQ observation version, a
//!    run label — arrives as [`CallerMetadata`] and is stored verbatim,
//!    uninterpreted.
//! 3. **Provenance that cannot be recorded faithfully fails the run.** A
//!    manifest that cannot be serialised or written is fatal; it is never
//!    downgraded to a warning.
//!
//! Source identity is `path+bytes`: the canonical source path plus each file's
//! name and byte length. Hashing a multi-gigabyte source on every run would
//! cost more than it proves, so the strategy is named in the manifest rather
//! than assumed by a reader.

mod checksum;
mod error;
mod metadata;
mod model;
mod time;

pub use checksum::Checksum;
pub use error::ManifestError;
pub use metadata::CallerMetadata;
pub use model::{
    Manifest, OutputArtefact, RecordGeometry, SourceFile, SourceIdentity, ToolIdentity,
    TransformRecord, MANIFEST_FILE_NAME, MANIFEST_VERSION,
};
