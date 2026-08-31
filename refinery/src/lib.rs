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
//! # Streaming primitives
//!
//! [`corpus::RecordReader`] streams records out of one or more corpus files
//! through a single fixed-size buffer, and [`corpus::RecordWriter`] buffers
//! whole records into a derived corpus. Both are transform-agnostic: they know
//! record geometry and nothing about sampling, shuffling or any application's
//! feature layout.
//!
//! ```
//! use neat_ai_refinery::corpus::RecordShape;
//!
//! let shape = RecordShape::new(2511, 1)?;
//! assert_eq!(shape.record_values(), 2512);
//! assert_eq!(shape.bytes_per_record(), 10_048);
//! # Ok::<(), neat_ai_refinery::corpus::CorpusError>(())
//! ```
//!
//! # Transforms
//!
//! [`sample`] is the first transform built on those primitives — a port of
//! GRQ's materialised sampler, keeping each record with probability `rate` and
//! publishing the result atomically. [`cli`] is the argument surface the
//! `neat_ai_refinery` binary drives it with.
//!
//! # Provenance
//!
//! Every derived corpus is published with a [`manifest`] beside it recording
//! how it was made — source identity, record geometry, transform, parameters,
//! seed, counts, tool version, timestamp and a checksum of the output. The
//! manifest is written into the staging directory before the publishing
//! rename, so a corpus is never published without it.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod cli;
pub mod corpus;
pub mod manifest;
pub mod sample;
