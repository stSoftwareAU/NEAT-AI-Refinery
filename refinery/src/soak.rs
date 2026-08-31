//! Production-soak evidence — the measurements the GRQ cut-over is gated on.
//!
//! Step 4 of the [migration principle](../index.html): before Refinery becomes
//! the sampler GRQ reaches for by default, a host has to show it behaves. This
//! module produces that evidence, and produces it the same way on every host
//! so two reports can be compared.
//!
//! One soak run, over a synthetic corpus at the caller's record shape:
//!
//! 1. sample the corpus `rounds` times through the real `neat_ai_refinery`
//!    binary, timing each run and sampling its peak resident memory;
//! 2. re-verify the published corpus every round — geometry, counts and
//!    checksum, read back off the published `manifest.json`;
//! 3. digest the source corpus before and after, so a run that wrote to the
//!    source cannot pass;
//! 4. measure the Deno sampler over the same corpus, for a throughput and
//!    peak-RSS comparison, and optionally re-check NEAT-AI's `evolveDir`
//!    consumes what Refinery published;
//! 5. force a run to fail and prove the previously published corpus is
//!    untouched and no staging directory was left behind.
//!
//! Every invariant is fatal: [`soak`] returns an error rather than a report
//! with a failure recorded inside it, because evidence that reports its own
//! breach as data is evidence somebody skims past.
//!
//! ```no_run
//! use std::path::PathBuf;
//! use neat_ai_refinery::corpus::RecordShape;
//! use neat_ai_refinery::sample::SampleRate;
//! use neat_ai_refinery::soak::{soak, SoakConfig};
//!
//! let report = soak(&SoakConfig {
//!     workspace: PathBuf::from("/tmp/refinery-soak"),
//!     binary: PathBuf::from("neat_ai_refinery"),
//!     shape: RecordShape::new(2511, 1)?,
//!     shards: 8,
//!     records_per_shard: 20_000,
//!     rate: SampleRate::new(0.05)?,
//!     rounds: 3,
//!     reference: None,
//! })?;
//! println!("{}", report.to_markdown());
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! The seed is deliberately *not* supplied: production omits it, so the soak
//! exercises the production path and each round draws a fresh sample.

mod digest;
mod error;
mod host;
mod measure;
mod report;
mod run;
mod verify;

pub use digest::{CorpusDigest, FileDigest};
pub use error::SoakError;
pub use host::HostFacts;
pub use measure::{MeasuredCommand, RunMeasurement};
pub use report::{AtomicPublicationEvidence, ConsumerCheck, CorpusFacts, SoakReport, SoakRound};
pub use run::{soak, DenoReference, SoakConfig};
pub use verify::PublishedCorpus;
