//! Fuzzing — a seeded noise-augmentation transform over a derived corpus.
//!
//! Fuzzing perturbs the values of a corpus and changes nothing else. It is a
//! **value** transform, not a selection or a representation one, so:
//!
//! 1. the record count of the output equals the record count of the input;
//! 2. records are written in the order they were read;
//! 3. the record layout — value counts, width and encoding — is unchanged.
//!
//! ```text
//! source ──fuzz──▶ derived corpus, same shape, noisier inputs
//! ```
//!
//! # The policy is explicit
//!
//! A run states the distribution it draws from, the scale it draws at, how the
//! noise is applied, which values it reaches and what bounds hold the result.
//! Nothing is inferred and nothing is defaulted except the one default that
//! makes the transform safe: [`FuzzTargets::Inputs`]. Perturbing an expected
//! output silently changes what a corpus teaches, so reaching one is an
//! explicit request.
//!
//! The whole policy is recorded in the manifest beside the published corpus,
//! together with the seed actually used, so a derived corpus can always be
//! traced back to the perturbation that produced it — and reproduced from it.
//!
//! # Reproducibility
//!
//! Noise is drawn from a seeded generator, one draw per targeted value, in
//! record order. The draw sequence therefore depends on the policy and the
//! record shape alone, never on the values a corpus happens to hold, and the
//! same seed and policy replay the same derived corpus byte for byte. Omit the
//! seed and the run takes one from the operating system — and reports it, so
//! any run can be replayed.
//!
//! # Bounds and values noise cannot be applied to
//!
//! - **A bound** holds every perturbed value inside `[clamp-min, clamp-max]`.
//!   Either side may be absent; by default both are.
//! - **A source value that is not finite** — a `NaN` or an infinity already in
//!   the corpus — is written back exactly as it was and counted as preserved.
//!   Noise is not defined on it, and inventing a number for it would be a
//!   quieter fault than leaving it alone.
//! - **A perturbation that leaves the finite range** aborts the run. A bound
//!   does not rescue it: an overflow means the scale does not suit the corpus,
//!   and publishing a clamped value in place of it would hide that.
//!
//! # Composing with the other transforms
//!
//! Fuzz reads a directory of `.bin` files and publishes a directory of `.bin`
//! files with a manifest beside them, which is exactly what
//! [`sample`](crate::sample) and [`quantise`](crate::quantise) produce and
//! consume. Sampling then fuzzing is two ordinary runs, with no shared state
//! and no knowledge of either transform on the other's part:
//!
//! ```text
//! source ──sample──▶ trainData-binary-sampler ──fuzz──▶ trainData-binary-fuzzed
//! ```
//!
//! The source manifest, when there is one, is read only to confirm the corpus
//! really is laid out the way the caller said it is.
//!
//! # What Refinery does not claim
//!
//! Refinery makes **no claim** that a fuzzed corpus trains a better model, or
//! that noise augmentation improves fitness. It supplies a reproducible,
//! recorded transform; whether perturbing a corpus helps is a downstream
//! experimental question, and the manifest is what makes answering it possible.

mod error;
mod noise;
mod plan;
mod run;

pub use error::FuzzError;
pub use plan::{
    FuzzBounds, FuzzDistribution, FuzzMode, FuzzPolicy, FuzzRequest, FuzzTargets, Perturbed,
};
pub use run::{fuzz, FuzzOutcome};
