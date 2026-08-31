//! Synthetic corpora for the measuring harnesses.
//!
//! The soak and the benchmark both need a corpus of a stated shape and size
//! that no committed fixture could supply — a production-shaped run reads
//! gigabytes — and both must read the *same* fixture for their numbers to be
//! comparable. This is that one builder.
//!
//! Nothing here writes to a source corpus a caller supplied: the directory is
//! one the harness owns and removes again.

use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::Path;

use super::{CorpusError, RecordShape};

/// Writes `shards` corpus files of `records_per_shard` records into
/// `directory`, and reports the total bytes written.
///
/// Values are distinct per record, so a corpus is never accidentally
/// self-similar, and every file holds whole records only. Records are written
/// one at a time: a production-shaped shard is hundreds of megabytes, and a
/// harness that needed all of it in memory would be measuring its own fixture.
///
/// # Errors
///
/// Returns [`CorpusError::EmptySource`] when the request describes a corpus
/// with no records at all — an empty corpus is fatal to every reader here, so
/// it is refused where it is written rather than surfacing later as a puzzling
/// read failure — and [`CorpusError::Io`] when a file cannot be written.
pub fn write_synthetic_corpus(
    directory: impl AsRef<Path>,
    shards: usize,
    records_per_shard: usize,
    shape: RecordShape,
) -> Result<u64, CorpusError> {
    let directory = directory.as_ref();
    if shards == 0 || records_per_shard == 0 {
        return Err(CorpusError::EmptySource {
            path: directory.to_path_buf(),
        });
    }
    fs::create_dir_all(directory).map_err(|e| CorpusError::io(directory, e))?;

    let mut total = 0_u64;
    let mut record = Vec::with_capacity(shape.bytes_per_record());
    for shard in 0..shards {
        let path = directory.join(format!("shard-{shard:03}.bin"));
        let file = File::create(&path).map_err(|e| CorpusError::io(&path, e))?;
        let mut writer = BufWriter::new(file);
        for index in 0..records_per_shard {
            record.clear();
            let base = (shard * records_per_shard + index) as f32;
            for value in 0..shape.record_values() {
                shape
                    .encoding()
                    .encode_into(base + value as f32, &mut record);
            }
            writer
                .write_all(&record)
                .map_err(|e| CorpusError::io(&path, e))?;
            total += record.len() as u64;
        }
        writer.flush().map_err(|e| CorpusError::io(&path, e))?;
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corpus::ValueEncoding;

    /// A throwaway directory for one test.
    fn scratch(label: &str) -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("refinery-synthetic-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        path
    }

    #[test]
    fn writes_whole_records_into_every_shard() {
        let directory = scratch("shards");
        let shape = RecordShape::new(2, 1).expect("valid shape");

        let bytes = write_synthetic_corpus(&directory, 3, 5, shape).expect("write the corpus");

        assert_eq!(bytes, 3 * 5 * shape.bytes_per_record() as u64);
        for shard in 0..3 {
            let path = directory.join(format!("shard-{shard:03}.bin"));
            let written = fs::metadata(&path).expect("the shard exists").len();
            assert_eq!(written, 5 * shape.bytes_per_record() as u64);
        }
        fs::remove_dir_all(&directory).expect("clean up");
    }

    #[test]
    fn writes_each_record_at_the_requested_encoding() {
        let directory = scratch("encoding");
        let shape = RecordShape::with_encoding(2, 1, ValueEncoding::BFloat16).expect("valid shape");

        let bytes = write_synthetic_corpus(&directory, 1, 4, shape).expect("write the corpus");

        assert_eq!(bytes, 4 * shape.bytes_per_record() as u64);
        assert_eq!(shape.bytes_per_record(), 6, "bfloat16 is two bytes a value");
        fs::remove_dir_all(&directory).expect("clean up");
    }

    #[test]
    fn distinct_records_so_a_corpus_is_never_self_similar() {
        let directory = scratch("distinct");
        let shape = RecordShape::new(2, 1).expect("valid shape");
        write_synthetic_corpus(&directory, 1, 2, shape).expect("write the corpus");

        let bytes = fs::read(directory.join("shard-000.bin")).expect("read the shard");
        let values = shape.encoding().decode(&bytes);

        assert_eq!(values, vec![0.0, 1.0, 2.0, 1.0, 2.0, 3.0]);
        fs::remove_dir_all(&directory).expect("clean up");
    }

    #[test]
    fn refuses_a_corpus_that_would_hold_no_records() {
        let directory = scratch("empty");
        let shape = RecordShape::new(2, 1).expect("valid shape");

        for (shards, records) in [(0, 10), (10, 0)] {
            let error = write_synthetic_corpus(&directory, shards, records, shape)
                .expect_err("an empty corpus is fatal, not an empty success");
            assert!(
                matches!(error, CorpusError::EmptySource { .. }),
                "{error:?}"
            );
        }
    }
}
