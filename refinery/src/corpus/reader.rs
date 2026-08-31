//! Streaming, bounded-memory reads over one or more corpus files.
//!
//! The reader holds a single fixed-size buffer no matter how large the corpus
//! is, and hands out one record at a time as a borrowed slice, so a transform
//! can process a corpus far larger than memory without copying each record.
//!
//! Files are opened with [`File::open`] — read access only — and are never
//! written to, renamed or removed, per the immutable-source rule.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};

use super::{CorpusError, RecordShape};

/// Bytes the default buffer aims for; the record width rounds it down.
const DEFAULT_BUFFER_BYTES: usize = 256 * 1024;

/// The file currently being streamed, and how much of it has been read.
#[derive(Debug)]
struct CurrentSource {
    path: PathBuf,
    file: File,
    byte_len: u64,
}

/// A streaming reader over the records of one or more corpus files.
///
/// Records are yielded in path order, and within a file in offset order, so a
/// given source list always produces the same sequence. Each file is validated
/// as it is consumed: a file that ends mid-record fails with
/// [`CorpusError::PartialRecord`], and one holding no records at all fails with
/// [`CorpusError::EmptySource`] — an unreadable corpus is never quietly
/// shortened.
///
/// ```no_run
/// use neat_ai_refinery::corpus::{RecordReader, RecordShape};
///
/// let shape = RecordShape::new(2511, 1)?;
/// let mut reader = RecordReader::open(&["trainData-binary".into()], shape)?;
/// while let Some(record) = reader.next_record() {
///     let bytes: &[u8] = record?;
///     assert_eq!(bytes.len(), shape.bytes_per_record());
/// }
/// # Ok::<(), neat_ai_refinery::corpus::CorpusError>(())
/// ```
#[derive(Debug)]
pub struct RecordReader {
    remaining: VecDeque<PathBuf>,
    current: Option<CurrentSource>,
    shape: RecordShape,
    buffer: Vec<u8>,
    /// Bytes of `buffer` holding data read from disk.
    filled: usize,
    /// Bytes of `buffer` already handed out.
    cursor: usize,
    records_read: u64,
    exhausted: bool,
}

impl RecordReader {
    /// Opens a reader over `paths`, in the order given.
    ///
    /// The buffer holds as many whole records as fit in 256 KiB, and at least
    /// one record however wide the shape is. No file is opened until the first
    /// record is requested, so a huge source list costs one file handle at a
    /// time rather than one per path.
    ///
    /// # Errors
    ///
    /// Returns [`CorpusError::EmptySourceList`] when `paths` is empty — a
    /// stream with nothing to read is a caller mistake, not an empty result.
    pub fn open(paths: &[PathBuf], shape: RecordShape) -> Result<Self, CorpusError> {
        let records = (DEFAULT_BUFFER_BYTES / shape.bytes_per_record()).max(1);
        Self::with_capacity(paths, shape, records)
    }

    /// Opens a reader whose buffer holds `records_per_fill` whole records.
    ///
    /// A capacity of zero is raised to one: the buffer must always be able to
    /// hold a whole record.
    ///
    /// # Errors
    ///
    /// As [`RecordReader::open`].
    pub fn with_capacity(
        paths: &[PathBuf],
        shape: RecordShape,
        records_per_fill: usize,
    ) -> Result<Self, CorpusError> {
        if paths.is_empty() {
            return Err(CorpusError::EmptySourceList);
        }

        let buffer_bytes = records_per_fill
            .max(1)
            .checked_mul(shape.bytes_per_record())
            .ok_or(CorpusError::RecordWidthOverflow {
                inputs: shape.inputs(),
                outputs: shape.outputs(),
            })?;

        Ok(Self {
            remaining: paths.iter().cloned().collect(),
            current: None,
            shape,
            buffer: vec![0_u8; buffer_bytes],
            filled: 0,
            cursor: 0,
            records_read: 0,
            exhausted: false,
        })
    }

    /// The record layout every yielded record conforms to.
    #[must_use]
    pub const fn shape(&self) -> &RecordShape {
        &self.shape
    }

    /// Records handed out so far.
    #[must_use]
    pub const fn records_read(&self) -> u64 {
        self.records_read
    }

    /// The file currently being streamed, if one is open.
    #[must_use]
    pub fn current_path(&self) -> Option<&Path> {
        self.current.as_ref().map(|source| source.path.as_path())
    }

    /// Bytes of buffer the reader occupies — its whole working set, whatever
    /// the corpus size.
    #[must_use]
    pub fn buffer_bytes(&self) -> usize {
        self.buffer.len()
    }

    /// Yields the next whole record, or `None` once every source is consumed.
    ///
    /// The slice borrows the reader's buffer and is valid until the next call;
    /// copy it if it must outlive that.
    ///
    /// # Errors
    ///
    /// Yields [`CorpusError::PartialRecord`] for a file ending mid-record,
    /// [`CorpusError::EmptySource`] for a file holding no records, and
    /// [`CorpusError::Io`] when a source cannot be opened or read. An error
    /// ends the stream: the reader yields `None` afterwards rather than
    /// skipping past a corpus it could not interpret.
    pub fn next_record(&mut self) -> Option<Result<&[u8], CorpusError>> {
        match self.fill_to_record() {
            Ok(false) => None,
            Err(error) => {
                self.exhausted = true;
                Some(Err(error))
            }
            Ok(true) => {
                let start = self.cursor;
                self.cursor += self.shape.bytes_per_record();
                self.records_read += 1;
                Some(Ok(&self.buffer[start..self.cursor]))
            }
        }
    }

    /// Reads until a whole record is buffered.
    ///
    /// Returns `Ok(true)` when one is available and `Ok(false)` when every
    /// source has been consumed cleanly.
    fn fill_to_record(&mut self) -> Result<bool, CorpusError> {
        let bytes_per_record = self.shape.bytes_per_record();
        loop {
            if self.exhausted {
                return Ok(false);
            }
            if self.filled - self.cursor >= bytes_per_record {
                return Ok(true);
            }

            // Move the partial record to the front so the read always has the
            // rest of the buffer to fill — this is what keeps memory bounded.
            self.buffer.copy_within(self.cursor..self.filled, 0);
            self.filled -= self.cursor;
            self.cursor = 0;

            let Some(source) = self.current.as_mut() else {
                if !self.open_next()? {
                    self.exhausted = true;
                    return Ok(false);
                }
                continue;
            };

            let read = read_some(&mut source.file, &mut self.buffer[self.filled..])
                .map_err(|e| CorpusError::io(&source.path, e))?;
            if read == 0 {
                self.close_current()?;
                continue;
            }
            source.byte_len += read as u64;
            self.filled += read;
        }
    }

    /// Opens the next source, skipping nothing; returns `false` when the list
    /// is spent.
    fn open_next(&mut self) -> Result<bool, CorpusError> {
        let Some(path) = self.remaining.pop_front() else {
            return Ok(false);
        };
        // Read-only by construction — the immutable-source rule.
        let file = File::open(&path).map_err(|e| CorpusError::io(&path, e))?;
        self.current = Some(CurrentSource {
            path,
            file,
            byte_len: 0,
        });
        Ok(true)
    }

    /// Retires the current source, rejecting one that ended mid-record or held
    /// no records at all.
    fn close_current(&mut self) -> Result<(), CorpusError> {
        let Some(source) = self.current.take() else {
            return Ok(());
        };
        let bytes_per_record = self.shape.bytes_per_record() as u64;

        if source.byte_len == 0 {
            return Err(CorpusError::EmptySource { path: source.path });
        }
        let trailing_bytes = source.byte_len % bytes_per_record;
        if trailing_bytes != 0 {
            return Err(CorpusError::PartialRecord {
                path: source.path,
                byte_len: source.byte_len,
                bytes_per_record,
                trailing_bytes,
            });
        }
        Ok(())
    }
}

/// Reads once into `buffer`, retrying an interrupted read.
///
/// A short read is normal and left to the caller's refill loop; only `Ok(0)`
/// means the file is at its end.
fn read_some(file: &mut File, buffer: &mut [u8]) -> std::io::Result<usize> {
    loop {
        match file.read(buffer) {
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            other => return other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape() -> RecordShape {
        RecordShape::new(2, 1).expect("valid shape")
    }

    #[test]
    fn rejects_an_empty_source_list() {
        let error = RecordReader::open(&[], shape()).expect_err("nothing to read is fatal");

        assert!(matches!(error, CorpusError::EmptySourceList), "{error:?}");
    }

    #[test]
    fn sizes_the_default_buffer_in_whole_records() {
        let reader =
            RecordReader::open(&[PathBuf::from("absent")], shape()).expect("open the reader");

        assert_eq!(reader.buffer_bytes() % 12, 0);
        assert!(reader.buffer_bytes() <= DEFAULT_BUFFER_BYTES);
        assert!(reader.buffer_bytes() >= 12);
    }

    #[test]
    fn holds_at_least_one_record_however_wide_the_shape() {
        let wide = RecordShape::new(DEFAULT_BUFFER_BYTES, 1).expect("valid shape");
        let reader = RecordReader::open(&[PathBuf::from("absent")], wide).expect("open the reader");

        assert_eq!(reader.buffer_bytes(), wide.bytes_per_record());
    }

    #[test]
    fn raises_a_zero_capacity_to_one_record() {
        let reader = RecordReader::with_capacity(&[PathBuf::from("absent")], shape(), 0)
            .expect("open the reader");

        assert_eq!(reader.buffer_bytes(), 12);
    }
}
