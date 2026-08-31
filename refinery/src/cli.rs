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
//!
//! Transforms compose by being run one after another over each other's output,
//! so quantising a sample is two invocations rather than a special mode:
//!
//! ```text
//! neat_ai_refinery --source trainData-binary --output sampled \
//!   --inputs 2511 --outputs 1 sample --rate 0.05
//! neat_ai_refinery --source sampled --output sampled-bf16 \
//!   --inputs 2511 --outputs 1 quantise --scheme bfloat16
//! ```

use std::error::Error;
use std::fmt;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::corpus::RecordShape;
use crate::manifest::CallerMetadata;
use crate::quantise::{QuantiseError, QuantiseRequest, QuantiseScheme};
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

    /// Quantisation: re-encode every value under `--scheme`, keeping every
    /// record and its order.
    Quantise(QuantiseArgs),
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

/// Arguments of the `quantise` transform.
#[derive(Debug, Args)]
pub struct QuantiseArgs {
    /// The quantisation scheme — currently `bfloat16`.
    ///
    /// There is no default: the scheme decides the error the corpus carries,
    /// so it is always stated and always recorded in the manifest.
    #[arg(long, value_name = "SCHEME")]
    pub scheme: String,
}

/// A validated transform request, ready to run.
#[derive(Debug, Clone)]
pub enum TransformRequest {
    /// A materialised sampling run.
    Sample(Box<SampleRequest>),
    /// A quantisation run.
    Quantise(Box<QuantiseRequest>),
}

/// Why a command line could not be turned into a transform request.
#[derive(Debug)]
#[non_exhaustive]
pub enum CliError {
    /// The `sample` arguments were rejected.
    Sample(SampleError),
    /// The `quantise` arguments were rejected.
    Quantise(QuantiseError),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sample(error) => write!(f, "{error}"),
            Self::Quantise(error) => write!(f, "{error}"),
        }
    }
}

impl Error for CliError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Sample(error) => Some(error),
            Self::Quantise(error) => Some(error),
        }
    }
}

impl From<SampleError> for CliError {
    fn from(error: SampleError) -> Self {
        Self::Sample(error)
    }
}

impl From<QuantiseError> for CliError {
    fn from(error: QuantiseError) -> Self {
        Self::Quantise(error)
    }
}

impl Cli {
    /// Validates the parsed arguments into a transform request.
    ///
    /// # Errors
    ///
    /// Returns [`CliError::Sample`] for a rate outside `(0, 1]`,
    /// [`CliError::Quantise`] for an unknown scheme, and either — depending on
    /// the subcommand — for an impossible record shape or caller metadata that
    /// is not a valid `KEY=VALUE` pair.
    pub fn request(&self) -> Result<TransformRequest, CliError> {
        match &self.command {
            Command::Sample(args) => Ok(TransformRequest::Sample(Box::new(
                self.sample_request(args)?,
            ))),
            Command::Quantise(args) => Ok(TransformRequest::Quantise(Box::new(
                self.quantise_request(args)?,
            ))),
        }
    }

    /// Validates the arguments of a `sample` run.
    ///
    /// # Errors
    ///
    /// Returns [`SampleError::InvalidRate`] for a rate outside `(0, 1]`,
    /// [`SampleError::Corpus`] for an impossible record shape, and
    /// [`SampleError::Manifest`] for caller metadata that is not a valid
    /// `KEY=VALUE` pair.
    pub fn sample_request(&self, args: &SampleArgs) -> Result<SampleRequest, SampleError> {
        Ok(SampleRequest {
            source: self.source.clone(),
            output: self.output.clone(),
            shape: RecordShape::new(self.inputs, self.outputs)?,
            rate: SampleRate::new(args.rate)?,
            seed: args.seed,
            metadata: CallerMetadata::parse(&self.metadata)?,
        })
    }

    /// Validates the arguments of a `quantise` run.
    ///
    /// # Errors
    ///
    /// Returns [`QuantiseError::UnknownScheme`] for a scheme Refinery does not
    /// offer, and [`QuantiseError::Transform`] for an impossible record shape
    /// or caller metadata that is not a valid `KEY=VALUE` pair.
    pub fn quantise_request(&self, args: &QuantiseArgs) -> Result<QuantiseRequest, QuantiseError> {
        Ok(QuantiseRequest {
            source: self.source.clone(),
            output: self.output.clone(),
            shape: RecordShape::new(self.inputs, self.outputs)?,
            scheme: args.scheme.parse::<QuantiseScheme>()?,
            metadata: CallerMetadata::parse(&self.metadata)?,
        })
    }
}
