//! Measures what quantisation costs and what it saves, on a synthetic corpus.
//!
//! The issue this answers asks for three numbers and no claims beyond them:
//! storage reduction, read throughput, and reconstruction error. Whether a
//! quantised corpus trains a better model is a downstream experimental
//! question this says nothing about.
//!
//! ```bash
//! cargo run --release --example quantise_throughput -- [shards] [records-per-shard]
//! ```
//!
//! Defaults to 8 shards of 20 000 records at the production shape
//! (2511 inputs, 1 output — 10 048 bytes a record). The corpus is built under
//! the system temporary directory and removed again.
//!
//! Values are drawn from a spread of magnitudes rather than small integers:
//! an exactly representable corpus would report zero error and prove nothing.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use neat_ai_refinery::corpus::{RecordShape, SourceCorpus, ValueEncoding};
use neat_ai_refinery::manifest::CallerMetadata;
use neat_ai_refinery::quantise::{quantise, QuantiseRequest, QuantiseScheme};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let shards: u32 = parse(args.next(), 8)?;
    let records_per_shard: u32 = parse(args.next(), 20_000)?;

    let shape = RecordShape::new(2511, 1)?;
    let root = env::temp_dir().join(format!("refinery-quantise-bench-{}", std::process::id()));
    let source = root.join("trainData-binary");
    fs::create_dir_all(&source)?;

    let built = Instant::now();
    let corpus_bytes = build_corpus(&source, shards, records_per_shard, &shape)?;
    println!(
        "corpus: {shards} shards × {records_per_shard} records = {} MiB, built in {:.2}s",
        corpus_bytes / (1024 * 1024),
        built.elapsed().as_secs_f64()
    );

    let scheme = QuantiseScheme::BFloat16;
    let request = QuantiseRequest {
        source: source.clone(),
        output: root.join("trainData-binary-bf16"),
        shape,
        scheme,
        metadata: CallerMetadata::default(),
    };

    let started = Instant::now();
    let outcome = quantise(&request)?;
    let elapsed = started.elapsed().as_secs_f64();

    println!(
        "quantised {} records as {scheme} in {elapsed:.3}s — {:.0} records/s, {:.1} MiB/s read",
        outcome.records_read,
        outcome.records_read as f64 / elapsed,
        corpus_bytes as f64 / elapsed / (1024.0 * 1024.0)
    );
    println!(
        "storage: {:.1} MiB → {:.1} MiB, {:.1}% smaller",
        outcome.source_bytes as f64 / (1024.0 * 1024.0),
        outcome.output_bytes as f64 / (1024.0 * 1024.0),
        outcome.storage_reduction() * 100.0
    );

    let error = measure_error(
        &source,
        &outcome.output_file,
        shape,
        request.target_shape()?,
    )?;
    println!(
        "reconstruction error over {} values: max relative {:.3e}, mean relative {:.3e}, max absolute {:.3e}",
        error.values, error.max_relative, error.mean_relative, error.max_absolute
    );
    println!(
        "scheme bound: {:.3e} — {}",
        scheme.max_relative_error(),
        if f64::from(error.max_relative) <= scheme.max_relative_error() {
            "held"
        } else {
            "BREACHED"
        }
    );

    fs::remove_dir_all(&root)?;
    Ok(())
}

/// What re-encoding cost, measured value by value against the source.
struct ErrorReport {
    values: u64,
    max_relative: f32,
    mean_relative: f32,
    max_absolute: f32,
}

/// Reads both corpora back and compares them value by value.
///
/// The comparison reads the published corpus through the ordinary
/// [`SourceCorpus`] path, so it measures what a consumer would actually
/// decode rather than an internal representation.
fn measure_error(
    source: &Path,
    published: &Path,
    source_shape: RecordShape,
    target_shape: RecordShape,
) -> Result<ErrorReport, Box<dyn std::error::Error>> {
    let mut shards: Vec<PathBuf> = fs::read_dir(source)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<_, _>>()?;
    shards.sort();

    let quantised = SourceCorpus::open(published, target_shape)?;
    let mut index = 0_u64;
    let mut values = 0_u64;
    let mut max_relative = 0.0_f32;
    let mut total_relative = 0.0_f64;
    let mut max_absolute = 0.0_f32;

    for shard in &shards {
        let original = SourceCorpus::open(shard, source_shape)?;
        for record in 0..original.record_count() {
            let before = original.read_record(record)?;
            let after = quantised.read_record(index)?;
            index += 1;

            for (original, decoded) in before.iter().zip(&after) {
                let absolute = (decoded - original).abs();
                max_absolute = max_absolute.max(absolute);
                if *original != 0.0 {
                    let relative = absolute / original.abs();
                    max_relative = max_relative.max(relative);
                    total_relative += f64::from(relative);
                    values += 1;
                }
            }
        }
    }

    Ok(ErrorReport {
        values,
        max_relative,
        mean_relative: if values == 0 {
            0.0
        } else {
            (total_relative / values as f64) as f32
        },
        max_absolute,
    })
}

/// Writes `shards` corpus files of spread-out magnitudes and reports the total
/// bytes written.
fn build_corpus(
    source: &Path,
    shards: u32,
    records_per_shard: u32,
    shape: &RecordShape,
) -> Result<u64, Box<dyn std::error::Error>> {
    let mut total = 0_u64;
    for shard in 0..shards {
        let path: PathBuf = source.join(format!("shard-{shard:03}.bin"));
        let mut bytes = Vec::with_capacity(records_per_shard as usize * shape.bytes_per_record());
        for record in 0..records_per_shard {
            let seed = (shard * records_per_shard + record) as f32;
            for value in 0..shape.record_values() {
                // Awkward on purpose: a mantissa that does not fit in eight
                // bits, scaled across many exponents and both signs.
                let magnitude = (0.017 + seed * 1.0e-3) * (1.0 + value as f32).sqrt();
                let sign = if value % 3 == 0 { -1.0 } else { 1.0 };
                ValueEncoding::Float32.encode_into(sign * magnitude, &mut bytes);
            }
        }
        total += bytes.len() as u64;
        fs::write(&path, &bytes)?;
    }
    Ok(total)
}

/// Parses an optional positional argument, falling back to `default`.
fn parse<T: std::str::FromStr>(argument: Option<String>, default: T) -> Result<T, T::Err> {
    argument.map_or(Ok(default), |value| value.parse())
}
