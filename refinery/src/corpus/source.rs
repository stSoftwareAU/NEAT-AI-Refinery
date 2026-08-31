//! Read-only access to a source corpus.
//!
//! The source file is opened with [`File::open`], which requests read access
//! only. Nothing in this module — or anywhere else in the crate — opens a
//! source for writing, truncates it, appends to it, renames it or removes it.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use super::{CorpusError, RecordShape};

/// A fixed-width corpus opened read-only.
///
/// Opening validates the whole-file invariant up front: the corpus must hold
/// at least one record and its size must be an exact multiple of
/// [`RecordShape::bytes_per_record`]. A partial trailing record is fatal.
#[derive(Debug)]
pub struct SourceCorpus {
    path: PathBuf,
    file: File,
    shape: RecordShape,
    byte_len: u64,
    record_count: u64,
}

impl SourceCorpus {
    /// Opens `path` read-only and validates it against `shape`.
    ///
    /// # Errors
    ///
    /// Returns [`CorpusError::EmptySource`] for a corpus with no records,
    /// [`CorpusError::PartialRecord`] when the size is not a whole number of
    /// records, and [`CorpusError::Io`] when the file cannot be opened or
    /// inspected.
    pub fn open(path: impl AsRef<Path>, shape: RecordShape) -> Result<Self, CorpusError> {
        let path = path.as_ref();
        // Read-only by construction — the immutable-source rule.
        let file = File::open(path).map_err(|e| CorpusError::io(path, e))?;
        let byte_len = file.metadata().map_err(|e| CorpusError::io(path, e))?.len();

        let bytes_per_record = shape.bytes_per_record() as u64;
        if byte_len == 0 {
            return Err(CorpusError::EmptySource {
                path: path.to_path_buf(),
            });
        }
        let trailing_bytes = byte_len % bytes_per_record;
        if trailing_bytes != 0 {
            return Err(CorpusError::PartialRecord {
                path: path.to_path_buf(),
                byte_len,
                bytes_per_record,
                trailing_bytes,
            });
        }

        Ok(Self {
            path: path.to_path_buf(),
            file,
            shape,
            byte_len,
            record_count: byte_len / bytes_per_record,
        })
    }

    /// The source path this corpus was opened from.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The record layout this corpus was validated against.
    #[must_use]
    pub const fn shape(&self) -> &RecordShape {
        &self.shape
    }

    /// Total size of the source in bytes.
    #[must_use]
    pub const fn byte_len(&self) -> u64 {
        self.byte_len
    }

    /// Whole records the corpus holds.
    #[must_use]
    pub const fn record_count(&self) -> u64 {
        self.record_count
    }

    /// Reads record `index`, decoded from the corpus encoding.
    ///
    /// The returned vector holds [`RecordShape::record_values`] values: the
    /// inputs first, then the outputs, in corpus order.
    ///
    /// # Errors
    ///
    /// Returns [`CorpusError::RecordIndexOutOfRange`] past the last record and
    /// [`CorpusError::Io`] when the read fails.
    pub fn read_record(&self, index: u64) -> Result<Vec<f32>, CorpusError> {
        if index >= self.record_count {
            return Err(CorpusError::RecordIndexOutOfRange {
                index,
                record_count: self.record_count,
            });
        }

        let bytes_per_record = self.shape.bytes_per_record();
        let offset = index * bytes_per_record as u64;
        let mut bytes = vec![0_u8; bytes_per_record];
        // `&File` implements `Read`/`Seek`, so reads need no mutable handle.
        let mut handle = &self.file;
        handle
            .seek(SeekFrom::Start(offset))
            .map_err(|e| CorpusError::io(&self.path, e))?;
        handle
            .read_exact(&mut bytes)
            .map_err(|e| CorpusError::io(&self.path, e))?;

        Ok(decode_float32(&bytes))
    }
}

/// Decodes native-endian IEEE-754 `f32` values from `bytes`.
///
/// `bytes.len()` is always a multiple of four here: it is one whole record,
/// whose width was validated when the corpus was opened.
fn decode_float32(bytes: &[u8]) -> Vec<f32> {
    let (values, _trailing) = bytes.as_chunks::<4>();
    values.iter().copied().map(f32::from_ne_bytes).collect()
}

#[cfg(test)]
mod tests {
    use super::decode_float32;

    #[test]
    fn decodes_native_endian_values() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&1.5_f32.to_ne_bytes());
        bytes.extend_from_slice(&(-2.25_f32).to_ne_bytes());

        assert_eq!(decode_float32(&bytes), vec![1.5, -2.25]);
    }

    #[test]
    fn decodes_an_empty_slice_to_no_values() {
        assert!(decode_float32(&[]).is_empty());
    }
}
