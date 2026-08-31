//! Buffered, whole-record writes to a derived corpus.
//!
//! The writer only ever creates the checked [`DerivedDestination`] it is handed
//! — a source corpus can never be its target, because constructing that
//! destination already proved it is not one.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use super::{CorpusError, DerivedDestination, RecordShape};

/// Bytes the default buffer aims for; the record width rounds it down.
const DEFAULT_BUFFER_BYTES: usize = 256 * 1024;

/// A buffered writer that accepts whole records only.
///
/// Records are accumulated in a fixed-size buffer and written with
/// [`Write::write_all`], so a short write is retried rather than silently
/// truncating the derived corpus. A record of the wrong width is rejected
/// before it reaches the buffer.
///
/// Call [`RecordWriter::finish`] when done: it flushes the tail and reports how
/// many records were written. Dropping a writer still flushes, and panics if
/// that flush fails, so buffered records are never lost in silence.
///
/// ```no_run
/// use neat_ai_refinery::corpus::{DerivedDestination, RecordShape, RecordWriter};
///
/// let shape = RecordShape::new(2, 1)?;
/// let sources = vec!["trainData-binary".into()];
/// let destination = DerivedDestination::new("trainData-binary-sampler", &sources)?;
///
/// let mut writer = RecordWriter::create(&destination, shape)?;
/// writer.write_values(&[1.0, 2.0, 3.0])?;
/// assert_eq!(writer.finish()?, 1);
/// # Ok::<(), neat_ai_refinery::corpus::CorpusError>(())
/// ```
#[derive(Debug)]
pub struct RecordWriter {
    path: PathBuf,
    file: File,
    shape: RecordShape,
    buffer: Vec<u8>,
    records_written: u64,
}

impl RecordWriter {
    /// Creates the derived corpus at `destination`, truncating any file already
    /// there.
    ///
    /// The buffer holds as many whole records as fit in 256 KiB, and at least
    /// one record however wide the shape is.
    ///
    /// # Errors
    ///
    /// Returns [`CorpusError::Io`] when the destination cannot be created.
    pub fn create(
        destination: &DerivedDestination,
        shape: RecordShape,
    ) -> Result<Self, CorpusError> {
        let records = (DEFAULT_BUFFER_BYTES / shape.bytes_per_record()).max(1);
        Self::create_with_capacity(destination, shape, records)
    }

    /// Creates the derived corpus with a buffer of `records_per_flush` whole
    /// records.
    ///
    /// A capacity of zero is raised to one, so every write is still buffered as
    /// a whole record.
    ///
    /// # Errors
    ///
    /// As [`RecordWriter::create`], plus [`CorpusError::RecordWidthOverflow`]
    /// when the requested buffer does not fit in a `usize`.
    pub fn create_with_capacity(
        destination: &DerivedDestination,
        shape: RecordShape,
        records_per_flush: usize,
    ) -> Result<Self, CorpusError> {
        let capacity = records_per_flush
            .max(1)
            .checked_mul(shape.bytes_per_record())
            .ok_or(CorpusError::RecordWidthOverflow {
                inputs: shape.inputs(),
                outputs: shape.outputs(),
            })?;

        let path = destination.path().to_path_buf();
        let file = File::create(&path).map_err(|e| CorpusError::io(&path, e))?;

        Ok(Self {
            path,
            file,
            shape,
            buffer: Vec::with_capacity(capacity),
            records_written: 0,
        })
    }

    /// The destination being written.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The record layout every accepted record must match.
    #[must_use]
    pub const fn shape(&self) -> &RecordShape {
        &self.shape
    }

    /// Records accepted so far, buffered ones included.
    #[must_use]
    pub const fn records_written(&self) -> u64 {
        self.records_written
    }

    /// Appends one whole record, already encoded.
    ///
    /// # Errors
    ///
    /// Returns [`CorpusError::RecordLengthMismatch`] when `record` is not
    /// exactly [`RecordShape::bytes_per_record`] bytes — a partial record is
    /// never padded or split — and [`CorpusError::Io`] when a flush fails.
    pub fn write_record(&mut self, record: &[u8]) -> Result<(), CorpusError> {
        let bytes_per_record = self.shape.bytes_per_record();
        if record.len() != bytes_per_record {
            return Err(CorpusError::RecordLengthMismatch {
                path: self.path.clone(),
                bytes_per_record,
                actual: record.len(),
            });
        }

        if self.buffer.len() + bytes_per_record > self.buffer.capacity() {
            self.flush()?;
        }
        self.buffer.extend_from_slice(record);
        self.records_written += 1;
        Ok(())
    }

    /// Appends one record from decoded values, encoded as the corpus stores
    /// them.
    ///
    /// # Errors
    ///
    /// As [`RecordWriter::write_record`]: `values` must hold exactly
    /// [`RecordShape::record_values`] values.
    pub fn write_values(&mut self, values: &[f32]) -> Result<(), CorpusError> {
        if values.len() != self.shape.record_values() {
            return Err(CorpusError::RecordLengthMismatch {
                path: self.path.clone(),
                bytes_per_record: self.shape.bytes_per_record(),
                actual: values.len() * self.shape.encoding().bytes_per_value(),
            });
        }

        let encoding = self.shape.encoding();
        let mut record = Vec::with_capacity(self.shape.bytes_per_record());
        for value in values {
            encoding.encode_into(*value, &mut record);
        }
        self.write_record(&record)
    }

    /// Writes every buffered record out to the file.
    ///
    /// # Errors
    ///
    /// Returns [`CorpusError::Io`] when the write fails.
    pub fn flush(&mut self) -> Result<(), CorpusError> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        // `write_all` loops until the whole buffer lands, so a short write
        // cannot truncate the derived corpus.
        self.file
            .write_all(&self.buffer)
            .map_err(|e| CorpusError::io(&self.path, e))?;
        self.buffer.clear();
        Ok(())
    }

    /// Flushes the tail and closes the writer, reporting the records written.
    ///
    /// # Errors
    ///
    /// Returns [`CorpusError::Io`] when the final write or flush fails.
    pub fn finish(mut self) -> Result<u64, CorpusError> {
        self.flush()?;
        self.file
            .flush()
            .map_err(|e| CorpusError::io(&self.path, e))?;
        Ok(self.records_written)
    }
}

impl Drop for RecordWriter {
    fn drop(&mut self) {
        if self.buffer.is_empty() {
            return;
        }
        // Buffered records must never vanish because `finish` was skipped:
        // flush them, and fail loud if that flush cannot be completed.
        match self.flush() {
            Ok(()) => {}
            // Panicking while already unwinding aborts the process, so report
            // the loss to stderr instead.
            Err(error) if std::thread::panicking() => eprintln!(
                "derived corpus {} lost buffered records on drop: {error}",
                self.path.display()
            ),
            Err(error) => panic!(
                "derived corpus {} lost buffered records on drop: {error}",
                self.path.display()
            ),
        }
    }
}
