//! Golden parity harness — Refinery's sampler against GRQ's `Sampler.ts`.
//!
//! Every test here builds one fixed corpus and runs **both** implementations
//! over it: the Rust [`neat_ai_refinery::sample`] API, and the golden
//! reference in `parity/grq_sampler.ts` — the GRQ algorithm extracted so it
//! can run without GRQ's creature and version state. The invariants asserted
//! are the ones issue #5 lists, and each is asserted against both outputs, so
//! a divergence fails the build rather than a production run.
//!
//! Byte-for-byte equality is deliberately **not** the target: the Deno sampler
//! draws from `Math.random()` with no seam to seed it, so identical bytes are
//! unobtainable without changing GRQ. What is compared is the contract a
//! caller depends on — record validity, provenance, the sampled share, output
//! ordering, naming, source immutability and publication semantics.
//!
//! The last two tests close the loop: NEAT-AI's `Creature.evolveDir` opens a
//! corpus Refinery published, unchanged, and scores creatures against it.
//!
//! The Deno half needs `deno` on `PATH`. Without it the Deno-dependent tests
//! print a skip notice and pass; set `REFINERY_PARITY_REQUIRED=1` — as
//! `.github/workflows/parity.yml` does — to make a missing `deno` fail loud
//! instead.

// The shared helpers serve several test binaries; not every one uses all of them.
#[allow(dead_code)]
mod common;

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use common::{encode, TempDir};
use neat_ai_refinery::corpus::RecordShape;
use neat_ai_refinery::sample::{sample, SampleRate, SampleRequest};

/// Two inputs and one output — twelve bytes a record, as the unit fixtures use.
const INPUTS: usize = 2;
const OUTPUTS: usize = 1;
const BYTES_PER_RECORD: usize = (INPUTS + OUTPUTS) * 4;

/// Which sampler produced an output — both are held to the same invariants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sampler {
    /// The Rust port under test.
    Refinery,
    /// The golden reference: GRQ's `Sampler.ts` algorithm.
    Grq,
}

impl Sampler {
    fn label(self) -> &'static str {
        match self {
            Self::Refinery => "refinery (rust)",
            Self::Grq => "grq reference (deno)",
        }
    }
}

fn shape() -> RecordShape {
    RecordShape::new(INPUTS, OUTPUTS).expect("valid shape")
}

/// The `parity/` directory holding the Deno half of the harness.
fn parity_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the crate sits inside the workspace")
        .join("parity")
}

/// Is `deno` runnable?
///
/// A missing `deno` is a skip locally and a failure wherever
/// `REFINERY_PARITY_REQUIRED` is set, so the harness can never be quietly
/// absent from a gate that is meant to enforce it.
fn deno_available(test: &str) -> bool {
    let found = Command::new("deno")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success());

    if !found {
        assert!(
            std::env::var_os("REFINERY_PARITY_REQUIRED").is_none(),
            "{test}: REFINERY_PARITY_REQUIRED is set but `deno` is not on PATH — \
             the parity harness cannot run"
        );
        eprintln!("SKIPPED {test}: `deno` is not on PATH — install Deno to run the parity harness");
    }
    found
}

/// Runs a harness script and returns its stdout, failing loud on a non-zero
/// exit with everything the script wrote.
fn run_deno(script: &str, permissions: &[&str], args: &[&str]) -> String {
    let mut command = Command::new("deno");
    command.current_dir(parity_dir()).arg("run");
    command.args(permissions);
    command.arg(script).args(args);

    let output = command
        .output()
        .unwrap_or_else(|e| panic!("run {script}: {e}"));
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "{script} exited {:?}\n--- stdout ---\n{stdout}\n--- stderr ---\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    stdout
}

/// Runs the golden GRQ reference sampler over `source`, publishing `output`.
fn run_grq_sampler(source: &Path, output: &Path, rate: f64) {
    run_deno(
        "grq_sampler.ts",
        &["--allow-read", "--allow-write"],
        &[
            "--source",
            &source.to_string_lossy(),
            "--output",
            &output.to_string_lossy(),
            "--inputs",
            &INPUTS.to_string(),
            "--outputs",
            &OUTPUTS.to_string(),
            "--rate",
            &rate.to_string(),
        ],
    );
}

/// Runs the Rust sampler over `source`, publishing `output`.
fn run_refinery_sampler(source: &Path, output: &Path, rate: f64, seed: u64) {
    let request = SampleRequest {
        source: source.to_path_buf(),
        output: output.to_path_buf(),
        shape: shape(),
        rate: SampleRate::new(rate).expect("valid rate"),
        seed: Some(seed),
    };
    sample(&request).expect("the refinery sampler succeeds");
}

/// Runs one sampler and returns the published directory.
fn run_sampler(which: Sampler, source: &Path, output: &Path, rate: f64, seed: u64) {
    match which {
        Sampler::Refinery => run_refinery_sampler(source, output, rate, seed),
        Sampler::Grq => run_grq_sampler(source, output, rate),
    }
}

/// The fixed corpus every test samples.
///
/// Records are generated by a documented rule rather than committed as a blob,
/// so the fixture is identical on every machine and readable in the diff:
/// record `i` of `total` holds `[i / total, ((i * 7) % 97) / 97, i / total *
/// ((i * 7) % 97) / 97]` — values inside `[0, 1)`, distinct per record, and
/// with an output that is a real function of the inputs so a consumer has
/// something to learn.
fn build_corpus(dir: &Path, shards: usize, records_per_shard: usize) -> Vec<Vec<u8>> {
    fs::create_dir_all(dir).expect("create the source corpus");
    let total = (shards * records_per_shard) as f32;

    let mut all = Vec::with_capacity(shards * records_per_shard);
    for shard in 0..shards {
        let mut bytes = Vec::with_capacity(records_per_shard * BYTES_PER_RECORD);
        for index in 0..records_per_shard {
            let position = (shard * records_per_shard + index) as f32;
            let first = position / total;
            let second = ((shard * records_per_shard + index) as u32 * 7 % 97) as f32 / 97.0;
            let record = encode(&[first, second, first * second]);
            bytes.extend_from_slice(&record);
            all.push(record);
        }
        let name = format!("shard-{}.bin", (b'a' + shard as u8) as char);
        fs::write(dir.join(name), bytes).expect("write a shard");
    }
    all
}

/// Splits a published corpus file into whole records, failing loud on a
/// trailing partial record — the first invariant the harness checks.
fn published_records(which: Sampler, file: &Path) -> Vec<Vec<u8>> {
    let bytes = fs::read(file).unwrap_or_else(|e| panic!("{}: read {file:?}: {e}", which.label()));
    assert_eq!(
        bytes.len() % BYTES_PER_RECORD,
        0,
        "{}: {file:?} holds a partial record — {} bytes is not a multiple of {BYTES_PER_RECORD}",
        which.label(),
        bytes.len()
    );
    bytes.chunks(BYTES_PER_RECORD).map(<[u8]>::to_vec).collect()
}

/// The single corpus file a published directory must hold, and its name.
fn published_file(which: Sampler, dir: &Path) -> (String, PathBuf) {
    let names = entries(dir);
    assert_eq!(
        names.len(),
        1,
        "{}: a published corpus holds exactly one file, found {names:?}",
        which.label()
    );
    let name = names.into_iter().next().expect("one entry");
    let path = dir.join(&name);
    (name, path)
}

/// Every file name directly inside `dir`.
fn entries(dir: &Path) -> BTreeSet<String> {
    fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("read {dir:?}: {e}"))
        .map(|entry| entry.expect("read the entry").file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .collect()
}

/// Every source file name paired with its bytes — the immutability snapshot.
fn snapshot(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files: Vec<(String, Vec<u8>)> = fs::read_dir(dir)
        .expect("read the source directory")
        .map(|entry| entry.expect("read the entry").path())
        .map(|path| {
            let name = path
                .file_name()
                .expect("a named file")
                .to_string_lossy()
                .into_owned();
            (name, fs::read(&path).expect("read the source file"))
        })
        .collect();
    files.sort();
    files
}

/// Asserts every published record came from the source corpus, no record was
/// invented, and none was duplicated beyond its source multiplicity.
fn assert_records_originate(which: Sampler, published: &[Vec<u8>], source: &[Vec<u8>]) {
    let mut available: HashMap<&[u8], usize> = HashMap::new();
    for record in source {
        *available.entry(record.as_slice()).or_insert(0) += 1;
    }

    for record in published {
        let count = available.get_mut(record.as_slice()).unwrap_or_else(|| {
            panic!(
                "{}: published a record that is not in the source corpus: {record:?}",
                which.label()
            )
        });
        assert!(
            *count > 0,
            "{}: published a record more often than the source holds it",
            which.label()
        );
        *count -= 1;
    }
}

/// Both samplers, so every invariant test states its subject once.
const BOTH: [Sampler; 2] = [Sampler::Refinery, Sampler::Grq];

#[test]
fn both_samplers_publish_whole_records_that_came_from_the_source() {
    if !deno_available("both_samplers_publish_whole_records_that_came_from_the_source") {
        return;
    }
    let temp = TempDir::new("parity-provenance");
    let source = temp.path().join("trainData-binary");
    let records = build_corpus(&source, 2, 500);

    for which in BOTH {
        let output = temp.path().join(format!("out-{which:?}"));
        run_sampler(which, &source, &output, 0.5, 20_260_831);

        let (_, file) = published_file(which, &output);
        let published = published_records(which, &file);
        assert!(
            !published.is_empty(),
            "{}: a rate of 0.5 over 1000 records must keep some",
            which.label()
        );
        assert_records_originate(which, &published, &records);
    }
}

#[test]
fn both_samplers_keep_close_to_the_requested_share() {
    if !deno_available("both_samplers_keep_close_to_the_requested_share") {
        return;
    }
    let temp = TempDir::new("parity-rate");
    let source = temp.path().join("trainData-binary");
    let total = 20_000;
    build_corpus(&source, 2, total / 2);

    // Binomial(20 000, 0.05): mean 1000, sd ≈ 30.8. Five standard deviations
    // either way is a bound a correct sampler clears essentially always, and
    // one a wrong rate — 0.04 or 0.06 — misses every time.
    let (low, high) = (846, 1_154);

    for which in BOTH {
        let output = temp.path().join(format!("out-{which:?}"));
        run_sampler(which, &source, &output, 0.05, 4_242);

        let (_, file) = published_file(which, &output);
        let kept = published_records(which, &file).len();
        assert!(
            (low..=high).contains(&kept),
            "{}: kept {kept} of {total} records at rate 0.05 — outside [{low}, {high}]",
            which.label()
        );
    }
}

#[test]
fn both_samplers_randomise_the_output_order_without_losing_a_record() {
    if !deno_available("both_samplers_randomise_the_output_order_without_losing_a_record") {
        return;
    }
    let temp = TempDir::new("parity-order");
    let source = temp.path().join("trainData-binary");
    let records = build_corpus(&source, 2, 400);

    for which in BOTH {
        let output = temp.path().join(format!("out-{which:?}"));
        // A rate of 1 keeps everything, so ordering is the only thing left to
        // differ: the sample is a permutation of the source, not a copy of it.
        run_sampler(which, &source, &output, 1.0, 7);

        let (name, file) = published_file(which, &output);
        assert_eq!(name, "sample-100.bin", "{}", which.label());

        let published = published_records(which, &file);
        assert_eq!(
            published.len(),
            records.len(),
            "{}: a rate of 1 keeps every record",
            which.label()
        );
        assert_records_originate(which, &published, &records);
        assert_ne!(
            published,
            records,
            "{}: the published order must be randomised, not the source order",
            which.label()
        );
    }
}

#[test]
fn both_samplers_name_the_published_file_the_same_way() {
    if !deno_available("both_samplers_name_the_published_file_the_same_way") {
        return;
    }
    let temp = TempDir::new("parity-naming");
    let source = temp.path().join("trainData-binary");
    build_corpus(&source, 1, 200);

    // The name is what a caller resolves, so the rounding rule matters more
    // than the bytes behind it: whole percent, half away from zero, and a rate
    // under half a percent naming `sample-0.bin` in both implementations.
    for (rate, expected) in [
        (1.0, "sample-100.bin"),
        (0.5, "sample-50.bin"),
        (0.125, "sample-13.bin"),
        (0.05, "sample-5.bin"),
        (0.004, "sample-0.bin"),
    ] {
        let mut names = Vec::new();
        for which in BOTH {
            let output = temp.path().join(format!("out-{which:?}-{expected}"));
            run_sampler(which, &source, &output, rate, 99);
            let (name, _) = published_file(which, &output);
            names.push(name);
        }
        assert_eq!(names[0], expected, "refinery named rate {rate}");
        assert_eq!(
            names[0], names[1],
            "rate {rate}: refinery published {}, the GRQ reference published {}",
            names[0], names[1]
        );
    }
}

#[test]
fn neither_sampler_changes_the_source_corpus() {
    if !deno_available("neither_sampler_changes_the_source_corpus") {
        return;
    }
    let temp = TempDir::new("parity-immutable");
    let source = temp.path().join("trainData-binary");
    build_corpus(&source, 3, 200);
    let before = snapshot(&source);

    for which in BOTH {
        let output = temp.path().join(format!("out-{which:?}"));
        run_sampler(which, &source, &output, 0.5, 5);
        assert_eq!(
            snapshot(&source),
            before,
            "{}: the source corpus must be byte-identical after sampling",
            which.label()
        );
    }
}

#[test]
fn both_samplers_replace_a_live_corpus_whole_and_leave_no_scratch() {
    if !deno_available("both_samplers_replace_a_live_corpus_whole_and_leave_no_scratch") {
        return;
    }
    let temp = TempDir::new("parity-publish");
    let source = temp.path().join("trainData-binary");
    build_corpus(&source, 2, 300);

    for which in BOTH {
        // A live corpus from a previous run, holding a file the new run does
        // not write. Publication replaces the directory whole, so the stale
        // file must be gone rather than merged with the new sample.
        let output = temp.path().join(format!("live-{which:?}"));
        fs::create_dir_all(&output).expect("create the live corpus");
        fs::write(output.join("sample-99.bin"), vec![0_u8; BYTES_PER_RECORD])
            .expect("write the stale corpus");

        run_sampler(which, &source, &output, 0.5, 13);

        let (name, _) = published_file(which, &output);
        assert_eq!(
            name,
            "sample-50.bin",
            "{}: the stale corpus must be replaced, not merged",
            which.label()
        );

        // Nothing beside the published directory: no staging directory and no
        // renamed-aside `.deleting-*` left behind. The reference sampler keeps
        // GRQ's `.tmp` staging root, which must be empty once it publishes.
        for entry in fs::read_dir(temp.path()).expect("read the working directory") {
            let path = entry.expect("read the entry").path();
            let name = path
                .file_name()
                .expect("named")
                .to_string_lossy()
                .into_owned();
            assert!(
                !name.contains(".deleting-"),
                "{}: left a renamed-aside directory behind: {name}",
                which.label()
            );
            if name == ".tmp" {
                assert_eq!(
                    entries(&path),
                    BTreeSet::new(),
                    "{}: left staging scratch behind",
                    which.label()
                );
            }
        }
    }
}

#[test]
fn evolve_dir_consumes_a_refinery_published_corpus() {
    if !deno_available("evolve_dir_consumes_a_refinery_published_corpus") {
        return;
    }
    let temp = TempDir::new("parity-evolve");
    let source = temp.path().join("trainData-binary");
    build_corpus(&source, 2, 128);
    let output = temp.path().join("trainData-binary-sampler");

    run_refinery_sampler(&source, &output, 1.0, 20_260_831);

    // The directory is handed to NEAT-AI exactly as Refinery published it —
    // same path, same `sample-100.bin` name, same fixed-width records.
    let stdout = run_deno(
        "evolve_dir.ts",
        &[
            "--allow-read",
            "--allow-write",
            "--allow-env",
            "--allow-run",
            "--allow-sys",
        ],
        &[
            "--corpus",
            &output.to_string_lossy(),
            "--inputs",
            &INPUTS.to_string(),
            "--outputs",
            &OUTPUTS.to_string(),
        ],
    );

    assert!(
        stdout.contains("\"consumed\":true"),
        "evolveDir did not consume the published corpus: {stdout}"
    );
}

#[test]
fn evolve_dir_rejects_a_corpus_refinery_would_never_publish() {
    if !deno_available("evolve_dir_rejects_a_corpus_refinery_would_never_publish") {
        return;
    }
    let temp = TempDir::new("parity-evolve-control");
    let corpus = temp.path().join("trainData-binary-sampler");
    fs::create_dir_all(&corpus).expect("create the corpus directory");

    // The control for the test above: a file ending mid-record — the one thing
    // Refinery's writer cannot emit. If NEAT-AI accepted this too, consuming a
    // Refinery corpus would prove nothing about the records inside it.
    fs::write(
        corpus.join("sample-100.bin"),
        vec![1_u8; BYTES_PER_RECORD + 4],
    )
    .expect("write the malformed corpus");

    let stdout = run_deno(
        "evolve_dir.ts",
        &[
            "--allow-read",
            "--allow-write",
            "--allow-env",
            "--allow-run",
            "--allow-sys",
        ],
        &[
            "--corpus",
            &corpus.to_string_lossy(),
            "--inputs",
            &INPUTS.to_string(),
            "--outputs",
            &OUTPUTS.to_string(),
            "--expect-failure",
        ],
    );

    assert!(
        stdout.contains("\"consumed\":false"),
        "evolveDir accepted a corpus holding a partial record: {stdout}"
    );
}
