//! Streaming reader/writer primitives, as executable tests.
//!
//! These cover what a later transform depends on: records stream out of one or
//! more files in order and in bounded memory, a truncated or empty file is
//! fatal and names itself, a corpus round-trips through the writer byte for
//! byte, and the sources are untouched afterwards.
//!
//! Committed fixtures under `tests/fixtures` are little-endian on purpose, so
//! the assertions decode them explicitly rather than depending on the host's
//! byte order.

mod common;

use common::{encode, TempDir};
use neat_ai_refinery::corpus::{
    discover_sources, CorpusError, DerivedDestination, RecordReader, RecordShape, RecordWriter,
    SourceCorpus,
};
use std::fs;
use std::path::PathBuf;

/// Two inputs and one output — twelve bytes per record.
fn shape() -> RecordShape {
    RecordShape::new(2, 1).expect("valid shape")
}

/// A committed binary fixture.
fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// Decodes a little-endian record, matching how the fixtures were written.
fn decode_le(record: &[u8]) -> Vec<f32> {
    let (values, _trailing) = record.as_chunks::<4>();
    values.iter().copied().map(f32::from_le_bytes).collect()
}

/// Drains a reader into one decoded record per entry.
fn drain(reader: &mut RecordReader) -> Result<Vec<Vec<f32>>, CorpusError> {
    let mut records = Vec::new();
    while let Some(record) = reader.next_record() {
        records.push(decode_le(record?));
    }
    Ok(records)
}

#[test]
fn streams_every_record_of_a_committed_fixture() {
    let mut reader =
        RecordReader::open(&[fixture("shard-a.bin")], shape()).expect("open the reader");

    let records = drain(&mut reader).expect("stream the fixture");

    assert_eq!(records, vec![vec![1.0, 2.0, 3.0], vec![4.0, 5.0, 6.0]]);
    assert_eq!(reader.records_read(), 2);
}

#[test]
fn streams_multiple_files_in_the_order_given() {
    let sources = vec![fixture("shard-a.bin"), fixture("shard-b.bin")];

    let mut reader = RecordReader::open(&sources, shape()).expect("open the reader");
    let records = drain(&mut reader).expect("stream both files");

    assert_eq!(
        records,
        vec![
            vec![1.0, 2.0, 3.0],
            vec![4.0, 5.0, 6.0],
            vec![7.0, 8.0, 9.0],
        ]
    );
    assert_eq!(reader.records_read(), 3);
}

#[test]
fn streams_a_record_that_lands_exactly_on_the_buffer_boundary() {
    // A one-record buffer makes every record end exactly where the buffer does.
    let sources = vec![fixture("shard-a.bin"), fixture("shard-b.bin")];
    let mut reader =
        RecordReader::with_capacity(&sources, shape(), 1).expect("open a one-record reader");

    assert_eq!(reader.buffer_bytes(), 12);
    let records = drain(&mut reader).expect("stream on the boundary");

    assert_eq!(records.len(), 3);
    assert_eq!(records[2], vec![7.0, 8.0, 9.0]);
    // Bounded memory: the buffer never grows past its stated capacity.
    assert_eq!(reader.buffer_bytes(), 12);
}

#[test]
fn streams_records_that_straddle_buffer_refills() {
    // 250 records through a three-record buffer, so most records are assembled
    // across a refill rather than read whole.
    let dir = TempDir::new("straddle");
    let values: Vec<f32> = (0..750).map(|v| v as f32).collect();
    let path = dir.write("trainData-binary", &encode(&values));

    let mut reader = RecordReader::with_capacity(&[path], shape(), 3).expect("open the reader");

    let mut seen = 0_u64;
    while let Some(record) = reader.next_record() {
        let record = record.expect("record");
        let expected = encode(&[
            seen as f32 * 3.0,
            seen as f32 * 3.0 + 1.0,
            seen as f32 * 3.0 + 2.0,
        ]);
        assert_eq!(record, expected.as_slice(), "record {seen}");
        seen += 1;
    }

    assert_eq!(seen, 250);
    assert_eq!(reader.buffer_bytes(), 36);
}

#[test]
fn rejects_a_truncated_final_record() {
    let path = fixture("truncated.bin");
    let mut reader =
        RecordReader::open(std::slice::from_ref(&path), shape()).expect("open the reader");

    let first = reader.next_record().expect("a whole first record");
    assert_eq!(decode_le(first.expect("record 0")), vec![10.0, 11.0, 12.0]);

    let error = reader
        .next_record()
        .expect("the truncated tail is reported")
        .expect_err("a partial record is fatal");

    match error {
        CorpusError::PartialRecord {
            path: reported,
            byte_len,
            bytes_per_record,
            trailing_bytes,
        } => {
            assert_eq!(reported, path);
            assert_eq!((byte_len, bytes_per_record, trailing_bytes), (20, 12, 8));
        }
        other => panic!("expected PartialRecord, got {other:?}"),
    }
}

#[test]
fn stops_the_stream_at_a_truncated_file_rather_than_reading_on() {
    let sources = vec![fixture("truncated.bin"), fixture("shard-b.bin")];
    let mut reader = RecordReader::open(&sources, shape()).expect("open the reader");

    reader
        .next_record()
        .expect("record 0")
        .expect("the whole first record");
    assert_eq!(reader.current_path(), Some(sources[0].as_path()));
    let error = reader
        .next_record()
        .expect("the truncated tail is reported")
        .expect_err("a partial record is fatal");

    assert!(
        matches!(error, CorpusError::PartialRecord { .. }),
        "{error:?}"
    );
    // The stream ends there: records after a corpus that could not be
    // interpreted are never handed out.
    assert!(reader.next_record().is_none());
    assert_eq!(reader.records_read(), 1);
}

#[test]
fn rejects_an_empty_file_in_the_stream() {
    let path = fixture("empty.bin");
    let mut reader =
        RecordReader::open(std::slice::from_ref(&path), shape()).expect("open the reader");

    let error = reader
        .next_record()
        .expect("an empty file is reported, not silently skipped")
        .expect_err("an empty source is fatal");

    match error {
        CorpusError::EmptySource { path: reported } => assert_eq!(reported, path),
        other => panic!("expected EmptySource, got {other:?}"),
    }
}

#[test]
fn rejects_an_empty_source_list() {
    let error = RecordReader::open(&[], shape()).expect_err("nothing to read is fatal");

    assert!(matches!(error, CorpusError::EmptySourceList), "{error:?}");
}

#[test]
fn reports_a_source_that_cannot_be_opened() {
    let dir = TempDir::new("absent");
    let missing = dir.path().join("absent.bin");

    let mut reader = RecordReader::open(&[missing], shape()).expect("open the reader");
    let error = reader
        .next_record()
        .expect("the missing file is reported")
        .expect_err("an unreadable source is fatal");

    assert!(matches!(error, CorpusError::Io { .. }), "{error:?}");
}

#[test]
fn round_trips_a_corpus_through_the_writer() {
    let dir = TempDir::new("round-trip");
    let a = dir.write("shard-a", &encode(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]));
    let b = dir.write("shard-b", &encode(&[7.0, 8.0, 9.0]));
    let sources = vec![a, b];
    let destination = DerivedDestination::new(dir.path().join("derived.bin"), &sources)
        .expect("a separate destination");

    let mut reader = RecordReader::with_capacity(&sources, shape(), 1).expect("open the reader");
    let mut writer =
        RecordWriter::create_with_capacity(&destination, shape(), 2).expect("create the writer");
    while let Some(record) = reader.next_record() {
        writer.write_record(record.expect("record")).expect("write");
    }
    let written = writer.finish().expect("finish the writer");

    assert_eq!(written, 3);
    let derived = SourceCorpus::open(destination.path(), shape()).expect("open the derived corpus");
    assert_eq!(derived.record_count(), 3);
    assert_eq!(
        derived.read_record(0).expect("record 0"),
        vec![1.0, 2.0, 3.0]
    );
    assert_eq!(
        derived.read_record(2).expect("record 2"),
        vec![7.0, 8.0, 9.0]
    );
    let mut expected = fs::read(&sources[0]).expect("read source a");
    expected.extend(fs::read(&sources[1]).expect("read source b"));
    assert_eq!(
        fs::read(destination.path()).expect("read derived"),
        expected
    );
}

#[test]
fn writes_values_the_way_the_corpus_stores_them() {
    let dir = TempDir::new("write-values");
    let destination =
        DerivedDestination::new(dir.path().join("derived.bin"), &[]).expect("a fresh destination");

    let mut writer = RecordWriter::create(&destination, shape()).expect("create the writer");
    writer
        .write_values(&[1.5, -2.25, 3.0])
        .expect("write values");
    assert_eq!(writer.records_written(), 1);
    writer.finish().expect("finish the writer");

    let corpus = SourceCorpus::open(destination.path(), shape()).expect("open the derived corpus");
    assert_eq!(
        corpus.read_record(0).expect("record 0"),
        vec![1.5, -2.25, 3.0]
    );
}

#[test]
fn rejects_a_record_of_the_wrong_width() {
    let dir = TempDir::new("wrong-width");
    let destination =
        DerivedDestination::new(dir.path().join("derived.bin"), &[]).expect("a fresh destination");
    let mut writer = RecordWriter::create(&destination, shape()).expect("create the writer");

    let error = writer
        .write_record(&[0_u8; 11])
        .expect_err("a short record is fatal");

    match error {
        CorpusError::RecordLengthMismatch {
            path,
            bytes_per_record,
            actual,
        } => {
            assert_eq!(path, destination.path());
            assert_eq!((bytes_per_record, actual), (12, 11));
        }
        other => panic!("expected RecordLengthMismatch, got {other:?}"),
    }
    assert_eq!(writer.records_written(), 0);
}

#[test]
fn buffers_writes_until_the_buffer_is_full() {
    let dir = TempDir::new("buffered");
    let destination =
        DerivedDestination::new(dir.path().join("derived.bin"), &[]).expect("a fresh destination");
    let mut writer =
        RecordWriter::create_with_capacity(&destination, shape(), 4).expect("create the writer");

    // Ten records through a four-record buffer: two full flushes plus a
    // remainder that only `finish` writes out.
    for record in 0..10_u32 {
        let base = record as f32 * 3.0;
        writer
            .write_values(&[base, base + 1.0, base + 2.0])
            .expect("write values");
    }
    let written = writer.finish().expect("finish the writer");

    assert_eq!(written, 10);
    let corpus = SourceCorpus::open(destination.path(), shape()).expect("open the derived corpus");
    assert_eq!(corpus.record_count(), 10);
    assert_eq!(
        corpus.read_record(9).expect("record 9"),
        vec![27.0, 28.0, 29.0]
    );
}

#[test]
fn leaves_the_source_files_unchanged_after_streaming() {
    let dir = TempDir::new("immutable-stream");
    let original_a = encode(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    let original_b = encode(&[7.0, 8.0, 9.0]);
    dir.write("shard-a", &original_a);
    dir.write("shard-b", &original_b);
    let sources = discover_sources(dir.path()).expect("discover the sources");
    let modified_before: Vec<_> = sources
        .iter()
        .map(|p| fs::metadata(p).expect("metadata").modified().ok())
        .collect();

    let mut reader = RecordReader::open(&sources, shape()).expect("open the reader");
    let records = drain(&mut reader).expect("stream the sources");

    assert_eq!(records.len(), 3);
    assert_eq!(fs::read(&sources[0]).expect("re-read a"), original_a);
    assert_eq!(fs::read(&sources[1]).expect("re-read b"), original_b);
    let modified_after: Vec<_> = sources
        .iter()
        .map(|p| fs::metadata(p).expect("metadata").modified().ok())
        .collect();
    assert_eq!(modified_after, modified_before);
}
