//! Quantisation — a representation transform over a derived corpus.
//!
//! Quantisation re-encodes every value of a corpus in a narrower format. It
//! changes how records are *stored*, never which records there are or what
//! order they are in, so:
//!
//! 1. the record count of the output equals the record count of the input;
//! 2. records are written in the order they were read;
//! 3. a run takes no seed and needs none — the same input always yields the
//!    same bytes.
//!
//! # Composing with sampling
//!
//! Quantise reads a directory of `.bin` files and publishes a directory of
//! `.bin` files with a manifest beside them, which is exactly what
//! [`sample`](crate::sample) produces and consumes. Sampling then quantising
//! is therefore two ordinary runs, with no shared state and no knowledge of
//! either transform on the other's part:
//!
//! ```text
//! source ──sample──▶ trainData-binary-sampler ──quantise──▶ trainData-binary-bf16
//! ```
//!
//! The source manifest, when there is one, is read only to confirm the corpus
//! really is encoded the way the caller said it is — a corpus that is already
//! quantised is refused rather than reinterpreted.
//!
//! # The scheme
//!
//! [`QuantiseScheme::BFloat16`] is the initial, deliberately conservative
//! scheme: an `f32` keeps its sign and its whole exponent and loses sixteen
//! mantissa bits, rounded to nearest with ties to even. Storage halves, the
//! representable range is unchanged, and the relative error is bounded by
//! `2^-8`. The mapping and its error characteristics are documented in
//! `docs/quantisation.md`.
//!
//! Whether a quantised corpus trains a better model is a downstream
//! experimental question. Refinery makes no claim about it: it reports the
//! storage, throughput and error consequences and nothing else.

mod error;
mod plan;
mod run;

pub use error::QuantiseError;
pub use plan::{QuantiseRequest, QuantiseScheme};
pub use run::{quantise, QuantiseOutcome};
