//! Opaque caller metadata.
//!
//! Refinery is application-agnostic, so it records nothing it does not do
//! itself. A caller that needs an application fact preserved — a GRQ
//! observation version, a run label — supplies it here and Refinery stores it
//! verbatim without interpreting it.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::ManifestError;

/// Longest accepted metadata key.
const MAX_KEY_BYTES: usize = 64;
/// Longest accepted metadata value.
const MAX_VALUE_BYTES: usize = 1024;

/// Caller-supplied key/value pairs recorded verbatim in the manifest.
///
/// Keys are `[A-Za-z0-9_.-]`, at most 64 bytes, and unique. Values are at most
/// 1024 bytes and hold no control characters. The bounds keep a manifest
/// readable and machine-parsable, and reject a caller smuggling framing
/// characters into a provenance record.
///
/// ```
/// use neat_ai_refinery::manifest::CallerMetadata;
///
/// let metadata = CallerMetadata::parse(&["grq_observation_version=42".to_string()])?;
/// assert_eq!(metadata.get("grq_observation_version"), Some("42"));
/// # Ok::<(), neat_ai_refinery::manifest::ManifestError>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CallerMetadata(BTreeMap<String, String>);

impl CallerMetadata {
    /// Parses `KEY=VALUE` entries, as the `--metadata` flag supplies them.
    ///
    /// # Errors
    ///
    /// Returns [`ManifestError::InvalidMetadata`] for an entry with no `=`, an
    /// out-of-range or duplicated key, or a value holding a control character.
    pub fn parse(entries: &[String]) -> Result<Self, ManifestError> {
        let mut metadata = Self::default();
        for entry in entries {
            let (key, value) = entry
                .split_once('=')
                .ok_or_else(|| ManifestError::invalid_metadata(entry, "expected KEY=VALUE"))?;
            metadata.insert(key, value)?;
        }
        Ok(metadata)
    }

    /// Records one key/value pair.
    ///
    /// # Errors
    ///
    /// Returns [`ManifestError::InvalidMetadata`] when the key or value is
    /// outside the accepted shape, or the key is already recorded.
    pub fn insert(&mut self, key: &str, value: &str) -> Result<(), ManifestError> {
        let entry = || format!("{key}={value}");

        if key.is_empty() || key.len() > MAX_KEY_BYTES {
            return Err(ManifestError::invalid_metadata(
                entry(),
                "the key must be 1 to 64 bytes",
            ));
        }
        if !key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b == b'-')
        {
            return Err(ManifestError::invalid_metadata(
                entry(),
                "the key may hold only letters, digits, '_', '.' and '-'",
            ));
        }
        if value.len() > MAX_VALUE_BYTES {
            return Err(ManifestError::invalid_metadata(
                entry(),
                "the value must be at most 1024 bytes",
            ));
        }
        if value.chars().any(char::is_control) {
            return Err(ManifestError::invalid_metadata(
                entry(),
                "the value may not hold control characters",
            ));
        }
        if self.0.contains_key(key) {
            return Err(ManifestError::invalid_metadata(
                entry(),
                "the key is already recorded",
            ));
        }

        self.0.insert(key.to_string(), value.to_string());
        Ok(())
    }

    /// The value recorded under `key`, if any.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }

    /// Every pair, ordered by key.
    #[must_use]
    pub const fn entries(&self) -> &BTreeMap<String, String> {
        &self.0
    }

    /// How many pairs are recorded.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether no pair is recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_pairs_ordered_by_key() {
        let metadata = CallerMetadata::parse(&[
            "zebra=last".to_string(),
            "alpha=first".to_string(),
            "middle=second".to_string(),
        ])
        .expect("valid metadata");

        assert_eq!(
            metadata.entries().keys().collect::<Vec<_>>(),
            vec!["alpha", "middle", "zebra"],
            "ordering is stable so two runs produce comparable manifests"
        );
    }

    #[test]
    fn keeps_a_value_holding_an_equals_sign_whole() {
        let metadata =
            CallerMetadata::parse(&["expression=a=b+c".to_string()]).expect("valid metadata");

        assert_eq!(metadata.get("expression"), Some("a=b+c"));
    }

    #[test]
    fn accepts_the_longest_allowed_key_and_value() {
        let mut metadata = CallerMetadata::default();

        metadata
            .insert(&"k".repeat(MAX_KEY_BYTES), &"v".repeat(MAX_VALUE_BYTES))
            .expect("the boundary is inside the range");

        assert_eq!(metadata.len(), 1);
    }

    #[test]
    fn reports_no_entries_as_empty() {
        assert!(CallerMetadata::default().is_empty());
        assert!(!CallerMetadata::parse(&["a=1".to_string()])
            .expect("valid")
            .is_empty());
    }
}
