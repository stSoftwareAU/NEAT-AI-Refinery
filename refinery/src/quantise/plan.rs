//! The validated inputs of a quantisation run, and the schemes it offers.

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

use super::QuantiseError;
use crate::corpus::{RecordShape, ValueEncoding};
use crate::manifest::CallerMetadata;

/// A quantisation scheme: the encoding a corpus is rewritten into, and the
/// error that costs.
///
/// One conservative scheme is offered to start with. A scheme is named
/// explicitly on the command line and recorded in the manifest, so a derived
/// corpus never leaves its mapping to be inferred.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum QuantiseScheme {
    /// `f32` → bfloat16: sign and exponent kept whole, sixteen mantissa bits
    /// dropped with round-to-nearest-even. Halves storage; relative error
    /// bounded by `2^-8`; the representable range is unchanged.
    BFloat16,
}

impl QuantiseScheme {
    /// Every scheme, for a caller listing the choices.
    pub const ALL: &'static [Self] = &[Self::BFloat16];

    /// The name the scheme is selected and recorded under.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::BFloat16 => "bfloat16",
        }
    }

    /// The encoding a corpus must already be in for this scheme to apply.
    #[must_use]
    pub const fn source_encoding(self) -> ValueEncoding {
        match self {
            Self::BFloat16 => ValueEncoding::Float32,
        }
    }

    /// The encoding the derived corpus is written in.
    #[must_use]
    pub const fn target_encoding(self) -> ValueEncoding {
        match self {
            Self::BFloat16 => ValueEncoding::BFloat16,
        }
    }

    /// The scheme's guaranteed bound on `|q(x) - x| / |x|` for a finite,
    /// normal `x` that stays in range — half an interval of the target
    /// significand.
    ///
    /// The bound is a property of the mapping, not a measurement: it holds for
    /// every value, and `refinery/tests/quantise_transform.rs` asserts a whole
    /// corpus against it.
    #[must_use]
    pub const fn max_relative_error(self) -> f64 {
        match self {
            // Eight bits of significand — one implicit, seven stored.
            Self::BFloat16 => 1.0 / 256.0,
        }
    }

    /// The derived corpus file name — `quantise-<scheme>.bin`.
    #[must_use]
    pub fn file_name(self) -> String {
        format!("quantise-{}.bin", self.name())
    }

    /// The parameters the manifest records, so a run can be repeated exactly.
    #[must_use]
    pub fn parameters(self) -> BTreeMap<String, serde_json::Value> {
        let mut parameters = BTreeMap::new();
        parameters.insert(
            "scheme".to_string(),
            serde_json::Value::from(self.name().to_string()),
        );
        parameters.insert(
            "source_encoding".to_string(),
            serde_json::Value::from(self.source_encoding().name().to_string()),
        );
        parameters.insert(
            "target_encoding".to_string(),
            serde_json::Value::from(self.target_encoding().name().to_string()),
        );
        parameters.insert(
            "rounding".to_string(),
            serde_json::Value::from("nearest-ties-to-even".to_string()),
        );
        parameters.insert(
            "max_relative_error".to_string(),
            serde_json::Value::from(self.max_relative_error()),
        );
        parameters
    }
}

impl fmt::Display for QuantiseScheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for QuantiseScheme {
    type Err = QuantiseError;

    /// Parses a scheme name, refusing an unknown one rather than defaulting.
    fn from_str(scheme: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .iter()
            .copied()
            .find(|candidate| candidate.name() == scheme)
            .ok_or_else(|| QuantiseError::UnknownScheme {
                scheme: scheme.to_string(),
            })
    }
}

/// One quantisation run, fully specified.
#[derive(Debug, Clone)]
pub struct QuantiseRequest {
    /// The source corpus directory, scanned for `.bin` files.
    pub source: PathBuf,
    /// The derived corpus directory to publish, replaced whole.
    pub output: PathBuf,
    /// The record layout of the source corpus.
    pub shape: RecordShape,
    /// The scheme every value is re-encoded with.
    pub scheme: QuantiseScheme,
    /// Opaque caller metadata to record in the manifest, uninterpreted.
    pub metadata: CallerMetadata,
}

impl QuantiseRequest {
    /// The record layout of the corpus this run will publish — the same value
    /// counts, in the scheme's narrower encoding.
    ///
    /// # Errors
    ///
    /// Returns [`QuantiseError::Transform`] wrapping a corpus error when the
    /// narrower width cannot be computed, which the source shape having been
    /// validated already makes unreachable in practice.
    pub fn target_shape(&self) -> Result<RecordShape, QuantiseError> {
        Ok(RecordShape::with_encoding(
            self.shape.inputs(),
            self.shape.outputs(),
            self.scheme.target_encoding(),
        )?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_the_published_file_after_the_scheme() {
        assert_eq!(
            QuantiseScheme::BFloat16.file_name(),
            "quantise-bfloat16.bin"
        );
    }

    #[test]
    fn parses_every_offered_scheme_by_name() {
        for scheme in QuantiseScheme::ALL {
            assert_eq!(
                scheme.name().parse::<QuantiseScheme>().expect("parses"),
                *scheme
            );
        }
    }

    #[test]
    fn refuses_an_unknown_scheme_rather_than_defaulting() {
        let error = "int4".parse::<QuantiseScheme>().expect_err("int4 is unknown");

        assert!(
            matches!(error, QuantiseError::UnknownScheme { ref scheme } if scheme == "int4"),
            "{error:?}"
        );
        // The message must name the alternatives, not just the mistake.
        assert!(error.to_string().contains("bfloat16"), "{error}");
    }

    #[test]
    fn records_the_mapping_and_its_error_bound_in_the_parameters() {
        let parameters = QuantiseScheme::BFloat16.parameters();

        assert_eq!(parameters["scheme"], "bfloat16");
        assert_eq!(parameters["source_encoding"], "float32");
        assert_eq!(parameters["target_encoding"], "bfloat16");
        assert_eq!(parameters["rounding"], "nearest-ties-to-even");
        assert_eq!(parameters["max_relative_error"], 1.0 / 256.0);
    }

    #[test]
    fn halves_the_record_width_of_the_published_corpus() {
        let request = QuantiseRequest {
            source: PathBuf::from("source"),
            output: PathBuf::from("derived"),
            shape: RecordShape::new(2511, 1).expect("valid shape"),
            scheme: QuantiseScheme::BFloat16,
            metadata: CallerMetadata::default(),
        };

        let target = request.target_shape().expect("a narrower shape exists");

        assert_eq!(target.record_values(), request.shape.record_values());
        assert_eq!(target.bytes_per_record(), 5024);
        assert_eq!(target.encoding(), ValueEncoding::BFloat16);
    }
}
