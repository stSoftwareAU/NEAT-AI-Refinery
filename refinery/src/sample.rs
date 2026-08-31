//! Materialised sampling — the first Refinery transform.
//!
//! This is a port of GRQ's `src/train/Sampler.ts`, not a redesign. The
//! behaviour it reproduces, in order:
//!
//! 1. scan the source directory for `.bin` corpus files;
//! 2. shuffle that file list, so input files are processed in random order;
//! 3. stream each file and keep every record independently with probability
//!    `rate`;
//! 4. shuffle the records kept from that file and append them to the output;
//! 5. write the result as `sample-<percent>.bin` inside a staging directory;
//! 6. publish the staging directory over the live one with an atomic rename,
//!    so a reader never sees an empty or half-built directory.
//!
//! A run is deterministic when [`SampleRequest::seed`] is supplied and seeded
//! from the operating system otherwise — the production default is unchanged.
//! The seed actually used is always reported in [`SampleOutcome::seed`].
//!
//! Failure is loud: a malformed record, a missing corpus file or a failed
//! write aborts the run, the staging directory is removed, and the previously
//! published corpus is left exactly as it was.

mod error;
mod plan;
mod publish;
mod run;

pub use error::SampleError;
pub use plan::{SampleRate, SampleRequest};
pub use publish::StagedCorpus;
pub use run::{sample, SampleOutcome};
