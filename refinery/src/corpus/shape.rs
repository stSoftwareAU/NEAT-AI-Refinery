//! The record layout of a fixed-width corpus.

use super::CorpusError;

/// How a single value is encoded on disk.
///
/// The current corpus format stores native-endian IEEE-754 `f32` values; the
/// enum exists so a future representation transform (quantisation) can name
/// its own encoding without changing the shape API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum ValueEncoding {
    /// Native-endian IEEE-754 binary32, four bytes per value.
    #[default]
    Float32,
}

impl ValueEncoding {
    /// Bytes one value occupies on disk.
    #[must_use]
    pub const fn bytes_per_value(self) -> usize {
        match self {
            Self::Float32 => 4,
        }
    }
}

/// The fixed record layout of a corpus: `inputs + outputs` values per record,
/// each encoded identically.
///
/// The shape is always supplied by the caller. Refinery does not infer it from
/// the corpus and does not read application state to obtain it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordShape {
    inputs: usize,
    outputs: usize,
    encoding: ValueEncoding,
    record_values: usize,
    bytes_per_record: usize,
}

impl RecordShape {
    /// Builds a shape for the current [`ValueEncoding::Float32`] corpus.
    ///
    /// # Errors
    ///
    /// Returns [`CorpusError::InvalidRecordShape`] when either side is zero,
    /// and [`CorpusError::RecordWidthOverflow`] when the resulting width does
    /// not fit in a `usize`.
    pub fn new(inputs: usize, outputs: usize) -> Result<Self, CorpusError> {
        Self::with_encoding(inputs, outputs, ValueEncoding::Float32)
    }

    /// Builds a shape with an explicit value encoding.
    ///
    /// # Errors
    ///
    /// As [`RecordShape::new`].
    pub fn with_encoding(
        inputs: usize,
        outputs: usize,
        encoding: ValueEncoding,
    ) -> Result<Self, CorpusError> {
        if inputs == 0 || outputs == 0 {
            return Err(CorpusError::InvalidRecordShape { inputs, outputs });
        }

        let overflow = || CorpusError::RecordWidthOverflow { inputs, outputs };
        let record_values = inputs.checked_add(outputs).ok_or_else(overflow)?;
        let bytes_per_record = record_values
            .checked_mul(encoding.bytes_per_value())
            .ok_or_else(overflow)?;

        Ok(Self {
            inputs,
            outputs,
            encoding,
            record_values,
            bytes_per_record,
        })
    }

    /// Input values per record.
    #[must_use]
    pub const fn inputs(&self) -> usize {
        self.inputs
    }

    /// Output values per record.
    #[must_use]
    pub const fn outputs(&self) -> usize {
        self.outputs
    }

    /// Values per record — `inputs + outputs`.
    #[must_use]
    pub const fn record_values(&self) -> usize {
        self.record_values
    }

    /// Bytes per record — `record_values * bytes_per_value`.
    #[must_use]
    pub const fn bytes_per_record(&self) -> usize {
        self.bytes_per_record
    }

    /// How each value is encoded on disk.
    #[must_use]
    pub const fn encoding(&self) -> ValueEncoding {
        self.encoding
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_the_width_of_a_realistic_shape() {
        let shape = RecordShape::new(2511, 1).expect("valid shape");

        assert_eq!(shape.inputs(), 2511);
        assert_eq!(shape.outputs(), 1);
        assert_eq!(shape.record_values(), 2512);
        assert_eq!(shape.bytes_per_record(), 10_048);
        assert_eq!(shape.encoding(), ValueEncoding::Float32);
    }

    #[test]
    fn accepts_the_smallest_possible_record() {
        let shape = RecordShape::new(1, 1).expect("valid shape");

        assert_eq!(shape.record_values(), 2);
        assert_eq!(shape.bytes_per_record(), 8);
    }

    #[test]
    fn rejects_zero_inputs() {
        let error = RecordShape::new(0, 1).expect_err("zero inputs is invalid");

        assert!(
            matches!(
                error,
                CorpusError::InvalidRecordShape {
                    inputs: 0,
                    outputs: 1
                }
            ),
            "{error:?}"
        );
    }

    #[test]
    fn rejects_zero_outputs() {
        let error = RecordShape::new(1, 0).expect_err("zero outputs is invalid");

        assert!(
            matches!(
                error,
                CorpusError::InvalidRecordShape {
                    inputs: 1,
                    outputs: 0
                }
            ),
            "{error:?}"
        );
    }

    #[test]
    fn rejects_a_value_count_that_overflows() {
        let error =
            RecordShape::new(usize::MAX, 1).expect_err("inputs + outputs must not overflow");

        assert!(
            matches!(error, CorpusError::RecordWidthOverflow { .. }),
            "{error:?}"
        );
    }

    #[test]
    fn rejects_a_byte_width_that_overflows() {
        // The value count fits, but multiplying by four bytes per value does not.
        let error = RecordShape::new(usize::MAX / 2, 1)
            .expect_err("record_values * bytes_per_value must not overflow");

        assert!(
            matches!(error, CorpusError::RecordWidthOverflow { .. }),
            "{error:?}"
        );
    }

    #[test]
    fn float32_occupies_four_bytes() {
        assert_eq!(ValueEncoding::Float32.bytes_per_value(), 4);
        assert_eq!(ValueEncoding::default(), ValueEncoding::Float32);
    }
}
