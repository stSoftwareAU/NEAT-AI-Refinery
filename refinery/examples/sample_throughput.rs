//! Measures materialised sampling throughput on a synthetic corpus.
//!
//! Behavioural parity comes before optimisation, so this reports numbers
//! rather than asserting on them:
//!
//! ```bash
//! cargo run --release --example sample_throughput -- [shards] [records-per-shard] [rate]
//! ```
//!
//! Defaults to 8 shards of 20 000 records at the production shape
//! (2511 inputs, 1 output — 10 048 bytes a record) and a rate of 0.05. The
//! corpus is built under the system temporary directory and removed again.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use neat_ai_refinery::corpus::RecordShape;
use neat_ai_refinery::manifest::CallerMetadata;
use neat_ai_refinery::sample::{sample, SampleRate, SampleRequest};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let shards: u32 = parse(args.next(), 8)?;
    let records_per_shard: u32 = parse(args.next(), 20_000)?;
    let rate: f64 = parse(args.next(), 0.05)?;

    let shape = RecordShape::new(2511, 1)?;
    let root = env::temp_dir().join(format!("refinery-bench-{}", std::process::id()));
    let source = root.join("trainData-binary");
    fs::create_dir_all(&source)?;

    let built = Instant::now();
    let corpus_bytes = build_corpus(&source, shards, records_per_shard, &shape)?;
    println!(
        "corpus: {shards} shards × {records_per_shard} records = {} MiB, built in {:.2}s",
        corpus_bytes / (1024 * 1024),
        built.elapsed().as_secs_f64()
    );

    let request = SampleRequest {
        source: source.clone(),
        output: root.join("trainData-binary-sampler"),
        shape,
        rate: SampleRate::new(rate)?,
        seed: Some(20_260_831),
        metadata: CallerMetadata::default(),
    };

    let started = Instant::now();
    let outcome = sample(&request)?;
    let elapsed = started.elapsed().as_secs_f64();

    let published = fs::metadata(&outcome.output_file)?.len();
    println!(
        "sampled {} records at rate {rate} in {elapsed:.3}s — {:.0} records/s, {:.1} MiB/s read",
        outcome.records_read,
        outcome.records_read as f64 / elapsed,
        corpus_bytes as f64 / elapsed / (1024.0 * 1024.0)
    );
    println!(
        "published {} ({} records, {:.1} MiB)",
        outcome.output_file.display(),
        outcome.records_written,
        published as f64 / (1024.0 * 1024.0)
    );

    fs::remove_dir_all(&root)?;
    Ok(())
}

/// Writes `shards` corpus files and reports the total bytes written.
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
                bytes.extend_from_slice(&(seed + value as f32).to_ne_bytes());
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
