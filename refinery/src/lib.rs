//! Fixed-width binary corpus primitives for NEAT-AI-Refinery.
//!
//! # The corpus contract
//!
//! A source corpus is a flat array of fixed-width records with no header and
//! no framing:
//!
//! ```text
//! record_values    = inputs + outputs
//! bytes_per_record = record_values * 4
//! encoding         = native-endian IEEE-754 Float32
//! ```
//!
//! The record shape is supplied by the caller — Refinery never infers it and
//! never parses application state to find it. A downstream orchestrator such
//! as GRQ derives `inputs` and `outputs` from its own creature export and
//! passes them in.
//!
//! # The immutable-source rule
//!
//! **Refinery never writes to a source corpus.** Sources are opened read-only
//! with [`std::fs::File::open`]; no code path in this crate edits, truncates,
//! appends to, renames or deletes a source file. Derived corpora are written
//! elsewhere, and [`corpus::DerivedDestination`] rejects a destination that
//! resolves to one of the sources.
//!
//! Malformed input is fatal rather than silently tolerated: a partial trailing
//! record, an empty source, an impossible record width and an arithmetic
//! overflow in the width calculation are all rejected at open time.
//!
//! ```
//! use neat_ai_refinery::corpus::RecordShape;
//!
//! let shape = RecordShape::new(2511, 1)?;
//! assert_eq!(shape.record_values(), 2512);
//! assert_eq!(shape.bytes_per_record(), 10_048);
//! # Ok::<(), neat_ai_refinery::corpus::CorpusError>(())
//! ```

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod corpus;
