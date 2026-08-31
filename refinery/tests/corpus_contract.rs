//! The fixed-width corpus and immutable-source contract, as executable tests.
//!
//! These cover the behaviour a caller depends on: a valid corpus reads back
//! record for record, a partial trailing record is fatal, and opening plus
//! reading a source leaves the bytes on disk untouched.

mod common;

use common::{encode, TempDir};
use neat_ai_refinery::corpus::{
    discover_sources, CorpusError, DerivedDestination, RecordShape, SourceCorpus,
};
use std::fs;

/// Two inputs and one output — the smallest shape that proves the layout.
fn shape() -> RecordShape {
    RecordShape::new(2, 1).expect("valid shape")
}

#[test]
fn reads_back_every_record_of_a_well_formed_corpus() {
    let dir = TempDir::new("valid");
    let values = [1.0_f32, 2.0, 3.0, 4.0, 5.0, 6.0];
    let path = dir.write("trainData-binary", &encode(&values));

    let corpus = SourceCorpus::open(&path, shape()).expect("open corpus");

    assert_eq!(corpus.byte_len(), 24);
    assert_eq!(corpus.record_count(), 2);
    assert_eq!(corpus.shape().bytes_per_record(), 12);
    assert_eq!(
        corpus.read_record(0).expect("record 0"),
        vec![1.0, 2.0, 3.0]
    );
    assert_eq!(
        corpus.read_record(1).expect("record 1"),
        vec![4.0, 5.0, 6.0]
    );
}

#[test]
fn rejects_a_partial_trailing_record() {
    let dir = TempDir::new("partial");
    // Twenty bytes is one whole 12-byte record plus eight trailing bytes.
    let path = dir.write("trainData-binary", &[0_u8; 20]);

    let error = SourceCorpus::open(&path, shape()).expect_err("partial record is fatal");

    match error {
        CorpusError::PartialRecord {
            byte_len,
            bytes_per_record,
            trailing_bytes,
            ..
        } => {
            assert_eq!((byte_len, bytes_per_record, trailing_bytes), (20, 12, 8));
        }
        other => panic!("expected PartialRecord, got {other:?}"),
    }
}

#[test]
fn rejects_an_empty_source() {
    let dir = TempDir::new("empty");
    let path = dir.write("trainData-binary", &[]);

    let error = SourceCorpus::open(&path, shape()).expect_err("an empty corpus is fatal");

    assert!(
        matches!(error, CorpusError::EmptySource { .. }),
        "{error:?}"
    );
}

#[test]
fn rejects_a_record_index_past_the_end() {
    let dir = TempDir::new("out-of-range");
    let path = dir.write("trainData-binary", &encode(&[1.0, 2.0, 3.0]));
    let corpus = SourceCorpus::open(&path, shape()).expect("open corpus");

    let error = corpus.read_record(1).expect_err("index 1 is past the end");

    match error {
        CorpusError::RecordIndexOutOfRange {
            index,
            record_count,
        } => assert_eq!((index, record_count), (1, 1)),
        other => panic!("expected RecordIndexOutOfRange, got {other:?}"),
    }
}

#[test]
fn opening_and_reading_leaves_the_source_bytes_untouched() {
    let dir = TempDir::new("immutable");
    let original = encode(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    let path = dir.write("trainData-binary", &original);
    let before = fs::metadata(&path).expect("metadata").modified().ok();

    let corpus = SourceCorpus::open(&path, shape()).expect("open corpus");
    for index in 0..corpus.record_count() {
        corpus.read_record(index).expect("read record");
    }
    drop(corpus);

    assert_eq!(fs::read(&path).expect("re-read source"), original);
    assert_eq!(
        fs::metadata(&path).expect("metadata").modified().ok(),
        before
    );
}

#[cfg(unix)]
#[test]
fn opens_a_read_only_source() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new("read-only");
    let path = dir.write("trainData-binary", &encode(&[1.0, 2.0, 3.0]));
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).expect("make read-only");

    let corpus = SourceCorpus::open(&path, shape()).expect("read-only sources are openable");

    assert_eq!(
        corpus.read_record(0).expect("record 0"),
        vec![1.0, 2.0, 3.0]
    );
}

#[test]
fn discovers_a_single_file_as_itself() {
    let dir = TempDir::new("single");
    let path = dir.write("trainData-binary", &encode(&[1.0, 2.0, 3.0]));

    assert_eq!(discover_sources(&path).expect("discover"), vec![path]);
}

#[test]
fn discovers_directory_entries_in_byte_wise_name_order() {
    let dir = TempDir::new("directory");
    // Written out of order, including names that sort differently under a
    // case-insensitive or numeric collation.
    dir.write("shard-10", &[0_u8; 12]);
    dir.write("shard-2", &[0_u8; 12]);
    dir.write("Shard-1", &[0_u8; 12]);
    dir.write(".hidden", &[0_u8; 12]);
    fs::create_dir(dir.path().join("nested")).expect("create nested directory");

    let found = discover_sources(dir.path()).expect("discover");

    let names: Vec<String> = found
        .iter()
        .map(|p| p.file_name().expect("name").to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["Shard-1", "shard-10", "shard-2"]);
}

#[test]
fn rejects_a_directory_with_no_sources() {
    let dir = TempDir::new("no-sources");

    let error = discover_sources(dir.path()).expect_err("an empty directory is fatal");

    assert!(
        matches!(error, CorpusError::NoSources { .. }),
        "expected NoSources, got {error:?}"
    );
}

#[test]
fn reports_a_missing_source_path() {
    let dir = TempDir::new("missing");

    let error = discover_sources(dir.path().join("absent")).expect_err("a missing path is fatal");

    assert!(matches!(error, CorpusError::Io { .. }), "{error:?}");
}

#[test]
fn rejects_a_derived_destination_that_is_a_source() {
    let dir = TempDir::new("destination-collides");
    let source = dir.write("trainData-binary", &encode(&[1.0, 2.0, 3.0]));

    let error = DerivedDestination::new(&source, std::slice::from_ref(&source))
        .expect_err("writing over a source is fatal");

    assert!(
        matches!(error, CorpusError::DestinationIsSource { .. }),
        "{error:?}"
    );
}

#[test]
fn accepts_a_derived_destination_separate_from_the_sources() {
    let dir = TempDir::new("destination-separate");
    let source = dir.write("trainData-binary", &encode(&[1.0, 2.0, 3.0]));
    let output = dir.path().join("trainData-binary-sampler");

    let destination = DerivedDestination::new(&output, std::slice::from_ref(&source))
        .expect("a separate destination is allowed");

    assert_eq!(destination.path().file_name(), output.file_name());
    assert!(
        !output.exists(),
        "the destination is not created by the check"
    );
}
