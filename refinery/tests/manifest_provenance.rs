//! The transformation manifest published beside a derived corpus.
//!
//! Every test drives the public API against a real corpus on disk and asserts
//! on the manifest that was published, so the checks survive a change of
//! implementation.

// The shared helpers serve several test binaries; not every one uses all of them.
#[allow(dead_code)]
mod common;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use common::{encode, TempDir};
use neat_ai_refinery::corpus::RecordShape;
use neat_ai_refinery::manifest::{CallerMetadata, Manifest, ManifestError, MANIFEST_FILE_NAME};
use neat_ai_refinery::sample::{sample, SampleError, SampleRate, SampleRequest};

/// Two inputs and one output — twelve bytes a record.
fn shape() -> RecordShape {
    RecordShape::new(2, 1).expect("valid shape")
}

/// Writes `count` records starting at `first` into `dir/name`.
fn write_shard(dir: &Path, name: &str, first: u32, count: u32) {
    let bytes: Vec<u8> = (first..first + count)
        .flat_map(|index| {
            let value = index as f32;
            encode(&[value, value * 2.0, value * 3.0])
        })
        .collect();
    fs::write(dir.join(name), bytes).expect("write shard");
}

/// A source directory holding one shard of `count` records.
fn source_with(root: &Path, count: u32) -> PathBuf {
    let source = root.join("trainData-binary");
    fs::create_dir_all(&source).expect("create the source directory");
    write_shard(&source, "shard-a.bin", 0, count);
    source
}

/// A request with no caller metadata.
fn request(source: &Path, output: &Path, rate: f64, seed: Option<u64>) -> SampleRequest {
    SampleRequest {
        source: source.to_path_buf(),
        output: output.to_path_buf(),
        shape: shape(),
        rate: SampleRate::new(rate).expect("valid rate"),
        seed,
        metadata: CallerMetadata::default(),
    }
}

/// Every file name directly inside `dir`.
fn entries(dir: &Path) -> BTreeSet<String> {
    fs::read_dir(dir)
        .expect("read the directory")
        .map(|entry| entry.expect("read the entry").file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .collect()
}

#[test]
fn publishes_a_manifest_beside_the_derived_corpus() {
    let temp = TempDir::new("manifest-beside");
    let source = source_with(temp.path(), 64);
    let output = temp.path().join("trainData-binary-sampler");

    let outcome = sample(&request(&source, &output, 1.0, Some(20_260_831))).expect("sampling");

    assert_eq!(
        entries(&output),
        BTreeSet::from(["sample-100.bin".into(), MANIFEST_FILE_NAME.to_string()]),
        "the manifest is published inside the derived corpus directory"
    );
    assert_eq!(outcome.manifest_file, output.join(MANIFEST_FILE_NAME));

    let manifest = Manifest::load(&outcome.manifest_file).expect("the manifest parses");
    assert_eq!(manifest, outcome.manifest, "the run reports what it wrote");
}

#[test]
fn records_everything_needed_to_reproduce_and_audit_the_run() {
    let temp = TempDir::new("manifest-fields");
    let source = temp.path().join("trainData-binary");
    fs::create_dir_all(&source).expect("create the source directory");
    write_shard(&source, "shard-a.bin", 0, 40);
    write_shard(&source, "shard-b.bin", 40, 60);
    let output = temp.path().join("derived");

    let outcome = sample(&request(&source, &output, 1.0, Some(42))).expect("sampling");
    let manifest = Manifest::load(output.join(MANIFEST_FILE_NAME)).expect("the manifest parses");

    assert_eq!(manifest.manifest_version, 1);
    assert_eq!(manifest.tool.name, "neat-ai-refinery");
    assert_eq!(manifest.tool.version, env!("CARGO_PKG_VERSION"));

    assert_eq!(manifest.transform.name, "sample");
    assert_eq!(manifest.transform.seed, Some(42));
    assert_eq!(
        manifest
            .transform
            .parameters
            .get("rate")
            .and_then(|v| v.as_f64()),
        Some(1.0),
        "the transform parameters are recorded: {:?}",
        manifest.transform.parameters
    );

    assert_eq!(manifest.record_shape.inputs, 2);
    assert_eq!(manifest.record_shape.outputs, 1);
    assert_eq!(manifest.record_shape.record_values, 3);
    assert_eq!(manifest.record_shape.bytes_per_record, 12);
    assert_eq!(manifest.record_shape.encoding, "float32");

    assert_eq!(
        manifest.source.path,
        fs::canonicalize(&source).expect("canonical source")
    );
    assert_eq!(manifest.source.identity_strategy, "path+bytes");
    assert_eq!(manifest.source.file_count, 2);
    assert_eq!(manifest.source.record_count, 100);
    assert_eq!(
        manifest
            .source
            .files
            .iter()
            .map(|file| (file.name.clone(), file.bytes))
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            ("shard-a.bin".to_string(), 480),
            ("shard-b.bin".to_string(), 720)
        ]),
        "each source file is identified by name and byte length"
    );

    assert_eq!(manifest.output.file, "sample-100.bin");
    assert_eq!(manifest.output.record_count, 100);
    assert_eq!(manifest.output.bytes, 1_200);
    assert_eq!(manifest.output.checksum.algorithm, "sha256");
    assert_eq!(
        manifest.output.checksum.value,
        independent_sha256(&output.join("sample-100.bin")),
        "the recorded checksum covers the published corpus"
    );

    assert!(
        manifest.created_at_unix > 1_700_000_000,
        "a plausible timestamp: {}",
        manifest.created_at_unix
    );
    assert!(
        manifest.created_at.ends_with('Z') && manifest.created_at.len() == 20,
        "an RFC 3339 UTC timestamp: {}",
        manifest.created_at
    );
    assert_eq!(outcome.records_written, manifest.output.record_count);
}

/// SHA-256 of `path`, computed straight from the whole file rather than
/// through the streaming digest the manifest uses.
fn independent_sha256(path: &Path) -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(fs::read(path).expect("read the published corpus"));
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[test]
fn reproduces_the_same_corpus_and_checksum_for_the_same_seed() {
    let temp = TempDir::new("manifest-reproducible");
    let source = source_with(temp.path(), 256);

    let first = temp.path().join("first");
    let again = temp.path().join("again");
    let other = temp.path().join("other");

    sample(&request(&source, &first, 0.5, Some(7))).expect("first run");
    sample(&request(&source, &again, 0.5, Some(7))).expect("repeat run");
    sample(&request(&source, &other, 0.5, Some(8))).expect("other seed");

    let manifest_of = |dir: &Path| Manifest::load(dir.join(MANIFEST_FILE_NAME)).expect("manifest");
    let (first, again, other) = (
        manifest_of(&first),
        manifest_of(&again),
        manifest_of(&other),
    );

    assert_eq!(
        first.output.checksum, again.output.checksum,
        "the same input, seed and transform config reproduce the corpus"
    );
    assert_eq!(first.output.record_count, again.output.record_count);
    assert_eq!(first.transform, again.transform);
    assert_eq!(first.source, again.source);
    assert_ne!(
        first.output.checksum, other.output.checksum,
        "a different seed produces a different corpus"
    );
}

#[test]
fn carries_opaque_caller_metadata_without_interpreting_it() {
    let temp = TempDir::new("manifest-metadata");
    let source = source_with(temp.path(), 16);
    let output = temp.path().join("derived");

    let metadata = CallerMetadata::parse(&[
        "grq_observation_version=42".to_string(),
        "run.label=nightly".to_string(),
    ])
    .expect("valid metadata");

    let mut request = request(&source, &output, 1.0, Some(3));
    request.metadata = metadata;
    sample(&request).expect("sampling");

    let manifest = Manifest::load(output.join(MANIFEST_FILE_NAME)).expect("the manifest parses");
    assert_eq!(
        manifest.metadata.get("grq_observation_version"),
        Some("42"),
        "caller metadata is stored verbatim"
    );
    assert_eq!(manifest.metadata.get("run.label"), Some("nightly"));
    assert_eq!(manifest.metadata.len(), 2, "and nothing else is invented");
}

#[test]
fn rejects_caller_metadata_that_cannot_be_recorded_faithfully() {
    let cases = [
        vec!["novalue".to_string()],
        vec!["=empty-key".to_string()],
        vec!["bad key=value".to_string()],
        vec!["key=line\nbreak".to_string()],
        vec!["dup=1".to_string(), "dup=2".to_string()],
        vec![format!("key={}", "x".repeat(1_025))],
        vec![format!("{}=value", "k".repeat(65))],
    ];

    for entries in cases {
        let error =
            CallerMetadata::parse(&entries).expect_err(&format!("{entries:?} must be rejected"));
        assert!(
            matches!(error, ManifestError::InvalidMetadata { .. }),
            "{entries:?} — {error:?}"
        );
    }
}

#[test]
fn accepts_an_empty_metadata_value() {
    let metadata = CallerMetadata::parse(&["note=".to_string()]).expect("an empty value is fine");

    assert_eq!(metadata.entries().get("note"), Some(&String::new()));
}

#[cfg(unix)]
#[test]
fn publishes_nothing_when_the_manifest_cannot_be_written() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let temp = TempDir::new("manifest-failure");
    // A source directory whose name is not valid UTF-8 cannot be recorded in a
    // JSON manifest. Provenance that cannot be written faithfully fails the run
    // rather than publishing a corpus that claims a path it did not read.
    let name = OsString::from_vec(b"trainData-\xff-binary".to_vec());
    let source = temp.path().join(name);
    fs::create_dir_all(&source).expect("create the source directory");
    write_shard(&source, "shard-a.bin", 0, 8);

    let output = temp.path().join("derived");
    fs::create_dir_all(&output).expect("create the live corpus");
    fs::write(output.join("sample-50.bin"), b"previous").expect("write the previous corpus");

    let error = sample(&request(&source, &output, 1.0, Some(1)))
        .expect_err("an unwritable manifest is fatal");

    assert!(
        matches!(error, SampleError::Manifest(_)),
        "the failure names the manifest: {error:?}"
    );
    assert_eq!(
        entries(&output),
        BTreeSet::from(["sample-50.bin".into()]),
        "the previously published corpus is left exactly as it was"
    );
    assert!(
        !entries(temp.path())
            .iter()
            .any(|name| name.contains("staging") || name.contains("deleting")),
        "the staging directory is reclaimed: {:?}",
        entries(temp.path())
    );
}

#[test]
fn writes_no_manifest_into_the_source_corpus() {
    let temp = TempDir::new("manifest-source-untouched");
    let source = source_with(temp.path(), 32);

    sample(&request(
        &source,
        &temp.path().join("derived"),
        1.0,
        Some(2),
    ))
    .expect("sampling");

    assert_eq!(
        entries(&source),
        BTreeSet::from(["shard-a.bin".into()]),
        "the source corpus gains nothing, manifest included"
    );
}
