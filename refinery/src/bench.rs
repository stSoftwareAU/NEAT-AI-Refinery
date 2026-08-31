//! Benchmark evidence — what Refinery costs, measured rather than claimed.
//!
//! A benchmark run builds one synthetic corpus and drives the real
//! `neat_ai_refinery` binary over it, case by case, reporting the figures
//! issue #14 asks for and no adjective beyond them:
//!
//! - wall-clock, from the fastest of `repeats` runs;
//! - input GiB/s and records/s over the corpus the run actually read;
//! - peak resident memory, the worst sampled across those runs;
//! - published output size, read back off the corpus that was written;
//! - the sample rate or pipeline each case ran, and the host it ran on.
//!
//! The Deno sampler GRQ shipped is measured over the same corpus by the same
//! code — [`crate::soak::MeasuredCommand`], so the soak and the benchmark
//! time and sample memory identically — and the comparison is a ratio taken
//! within a single run on a single host, which is the only form of it that
//! survives being read on somebody else's hardware.
//!
//! ```no_run
//! use std::path::PathBuf;
//! use neat_ai_refinery::bench::{bench, BenchCase, BenchConfig};
//! use neat_ai_refinery::corpus::RecordShape;
//!
//! let report = bench(&BenchConfig {
//!     workspace: PathBuf::from("/tmp/refinery-bench"),
//!     binary: PathBuf::from("neat_ai_refinery"),
//!     shape: RecordShape::new(2511, 1)?,
//!     shards: 8,
//!     records_per_shard: 20_000,
//!     repeats: 3,
//!     cases: BenchCase::standard_suite(0.05),
//!     reference: None,
//! })?;
//! println!("{}", report.to_markdown());
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # Regressions
//!
//! Two gates, because two different things can rot:
//!
//! 1. [`Comparison`] holds a run against a committed baseline report from the
//!    same corpus — the repeatable manual benchmark, run on one host against
//!    its own previous numbers.
//! 2. [`BenchReport::check_speedup`] holds Refinery against the Deno sampler
//!    measured beside it in the same run — the gate CI enforces, because a
//!    ratio measured on the runner it is judged on does not care how loaded
//!    that runner was.
//!
//! Both fail loud. A regression is an error, never a line inside a report that
//! otherwise reads as a pass.
//!
//! Correctness is not this module's to trade: every case is measured through
//! the published corpus, and a run that did not read the whole corpus, or
//! whose published bytes disagree with its own manifest, is a failure rather
//! than an impressively fast result.

mod baseline;
mod case;
mod error;
mod report;
mod run;

pub use baseline::{CaseDelta, Comparison};
pub use case::{BenchCase, BenchTransform};
pub use error::BenchError;
pub use report::{BenchReport, CaseResult, CorpusFacts, Reference};
pub use run::{bench, BenchConfig, DenoReference};
