//! The record layout of a fixed-width corpus.

use super::{bfloat16, CorpusError};

/// How a single value is encoded on disk.
///
/// A source corpus stores native-endian IEEE-754 `f32` values. A representation
/// transform such as quantisation names its own encoding here, so the width of
/// a derived corpus follows from the shape rather than from the transform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum ValueEncoding {
    /// Native-endian IEEE-754 binary32, four bytes per value.
    #[default]
    Float32,
    /// Native-endian bfloat16 — binary32 truncated to its top sixteen bits,
    /// two bytes per value. See [`crate::corpus`] and `docs/quantisation.md`.
    BFloat16,
}

impl ValueEncoding {
    /// Bytes one value occupies on disk.
    #[must_use]
    pub const fn bytes_per_value(self) -> usize {
        match self {
            Self::Float32 => 4,
            Self::BFloat16 => 2,
        }
    }

    /// The name the manifest records this encoding under.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Float32 => "float32",
            Self::BFloat16 => "bfloat16",
        }
    }

    /// Appends `value` to `out` as this encoding stores it.
    pub fn encode_into(self, value: f32, out: &mut Vec<u8>) {
        match self {
            Self::Float32 => out.extend_from_slice(&value.to_ne_bytes()),
            Self::BFloat16 => out.extend_from_slice(&bfloat16::from_f32(value).to_ne_bytes()),
        }
    }

    /// Decodes every whole value in `bytes`, ignoring a trailing partial one.
    ///
    /// Callers hand whole records in, whose width was validated when the corpus
    /// was opened, so there is never a trailing partial value in practice.
    #[must_use]
    pub fn decode(self, bytes: &[u8]) -> Vec<f32> {
        let mut values = Vec::with_capacity(bytes.len() / self.bytes_per_value());
        self.for_each_value(bytes, |value| values.push(value));
        values
    }

    /// Re-encodes one whole record from this encoding into `target`, appending
    /// the result to `out`.
    ///
    /// No intermediate buffer is allocated: a value is decoded and immediately
    /// re-encoded, so a transcode costs one pass whatever the record width.
    pub fn transcode_into(self, record: &[u8], target: Self, out: &mut Vec<u8>) {
        self.for_each_value(record, |value| target.encode_into(value, out));
    }

    /// Calls `visit` with each whole value encoded in `bytes`, in order.
    fn for_each_value(self, bytes: &[u8], mut visit: impl FnMut(f32)) {
        match self {
            Self::Float32 => {
                let (values, _trailing) = bytes.as_chunks::<4>();
                for value in values {
                    visit(f32::from_ne_bytes(*value));
                }
            }
            Self::BFloat16 => {
                let (values, _trailing) = bytes.as_chunks::<2>();
                for value in values {
                    visit(bfloat16::to_f32(u16::from_ne_bytes(*value)));
                }
            }
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
        assert_eq!(ValueEncoding::Float32.name(), "float32");
    }

    #[test]
    fn bfloat16_halves_the_width_of_a_record() {
        let shape = RecordShape::with_encoding(2511, 1, ValueEncoding::BFloat16)
            .expect("a bfloat16 shape is valid");

        assert_eq!(ValueEncoding::BFloat16.bytes_per_value(), 2);
        assert_eq!(ValueEncoding::BFloat16.name(), "bfloat16");
        assert_eq!(shape.record_values(), 2512);
        assert_eq!(shape.bytes_per_record(), 5024);
        assert_eq!(
            shape.bytes_per_record() * 2,
            RecordShape::new(2511, 1)
                .expect("valid shape")
                .bytes_per_record()
        );
    }

    #[test]
    fn round_trips_a_record_through_the_float32_encoding() {
        let values = [1.5_f32, -2.25, 0.0];
        let mut bytes = Vec::new();
        for value in values {
            ValueEncoding::Float32.encode_into(value, &mut bytes);
        }

        assert_eq!(bytes.len(), 12);
        assert_eq!(ValueEncoding::Float32.decode(&bytes), values);
    }

    #[test]
    fn transcodes_a_record_into_the_narrower_encoding() {
        let values = [1.5_f32, -2.25, 0.0, 256.0];
        let mut wide = Vec::new();
        for value in values {
            ValueEncoding::Float32.encode_into(value, &mut wide);
        }

        let mut narrow = Vec::new();
        ValueEncoding::Float32.transcode_into(&wide, ValueEncoding::BFloat16, &mut narrow);

        assert_eq!(narrow.len(), wide.len() / 2);
        // Every one of these is exactly representable, so the round trip is
        // lossless and the narrow record decodes back to the same values.
        assert_eq!(ValueEncoding::BFloat16.decode(&narrow), values);
    }

    #[test]
    fn decodes_an_empty_record_to_no_values() {
        assert!(ValueEncoding::Float32.decode(&[]).is_empty());
        assert!(ValueEncoding::BFloat16.decode(&[]).is_empty());
    }
}
