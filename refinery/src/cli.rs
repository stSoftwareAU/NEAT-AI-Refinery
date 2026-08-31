//! The `neat_ai_refinery` command-line surface.
//!
//! The record shape is supplied by the caller — Refinery never infers it — so
//! `--inputs` and `--outputs` are global, and each transform is a subcommand:
//!
//! ```text
//! neat_ai_refinery \
//!   --source /path/to/trainData-binary \
//!   --output /path/to/trainData-binary-sampler \
//!   --inputs 2511 \
//!   --outputs 1 \
//!   [--metadata grq_observation_version=42] \
//!   sample --rate 0.05 [--seed 20260831]
//! ```

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::corpus::RecordShape;
use crate::manifest::CallerMetadata;
use crate::sample::{SampleError, SampleRate, SampleRequest};

/// Produce a derived training corpus from an immutable source corpus.
#[derive(Debug, Parser)]
#[command(name = "neat_ai_refinery", version, about, long_about = None)]
pub struct Cli {
    /// Source corpus directory; read-only, never modified.
    #[arg(long, value_name = "DIR")]
    pub source: PathBuf,

    /// Derived corpus directory, published atomically.
    #[arg(long, value_name = "DIR")]
    pub output: PathBuf,

    /// Input values per record.
    #[arg(long, value_name = "N")]
    pub inputs: usize,

    /// Output values per record.
    #[arg(long, value_name = "N")]
    pub outputs: usize,

    /// Caller metadata recorded verbatim in the manifest; repeatable.
    ///
    /// Refinery never interprets it — it is how an application keeps its own
    /// facts, such as an observation version, with the derived corpus.
    #[arg(long, value_name = "KEY=VALUE")]
    pub metadata: Vec<String>,

    /// The transform to run.
    #[command(subcommand)]
    pub command: Command,
}

/// The transforms Refinery can run.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Materialised sampling: keep each record with probability `--rate`.
    Sample(SampleArgs),
}

/// Arguments of the `sample` transform.
#[derive(Debug, Args)]
pub struct SampleArgs {
    /// Probability each record is kept, in `(0, 1]`.
    #[arg(long, value_name = "0..1")]
    pub rate: f64,

    /// Seed for a reproducible run; omitted, the run seeds from the operating
    /// system as production does.
    #[arg(long, value_name = "N")]
    pub seed: Option<u64>,
}

impl Cli {
    /// Validates the parsed arguments into a sampling request.
    ///
    /// # Errors
    ///
    /// Returns [`SampleError::InvalidRate`] for a rate outside `(0, 1]`,
    /// [`SampleError::Corpus`] for an impossible record shape, and
    /// [`SampleError::Manifest`] for caller metadata that is not a valid
    /// `KEY=VALUE` pair.
    pub fn request(&self) -> Result<SampleRequest, SampleError> {
        let Command::Sample(args) = &self.command;
        let shape = RecordShape::new(self.inputs, self.outputs)?;

        Ok(SampleRequest {
            source: self.source.clone(),
            output: self.output.clone(),
            shape,
            rate: SampleRate::new(args.rate)?,
            seed: args.seed,
            metadata: CallerMetadata::parse(&self.metadata)?,
        })
    }
}
