//! The manifest itself: what is recorded, and how it is written and read.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{time, CallerMetadata, Checksum, ManifestError};
use crate::corpus::RecordShape;

/// The file name a manifest is published under, inside the derived corpus.
pub const MANIFEST_FILE_NAME: &str = "manifest.json";

/// The schema version of the manifests this crate writes.
///
/// A reader should refuse a manifest whose version it does not know rather
/// than guess at fields that may have moved.
pub const MANIFEST_VERSION: u32 = 1;

/// How the source corpus was identified — see the module documentation.
const IDENTITY_STRATEGY: &str = "path+bytes";

/// The provenance record of one derived corpus.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    /// The schema version — [`MANIFEST_VERSION`] for a manifest written here.
    pub manifest_version: u32,
    /// The tool that produced the corpus.
    pub tool: ToolIdentity,
    /// When the manifest was written, RFC 3339 UTC.
    pub created_at: String,
    /// The same instant in seconds since the Unix epoch.
    pub created_at_unix: u64,
    /// The transform, its parameters and its seed.
    pub transform: TransformRecord,
    /// The record layout of the **published** corpus — what a reader of this
    /// directory must decode with.
    pub record_shape: RecordGeometry,
    /// The record layout of the source, recorded only when a representation
    /// transform changed it; absent when both corpora share one layout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_record_shape: Option<RecordGeometry>,
    /// What was read.
    pub source: SourceIdentity,
    /// What was published.
    pub output: OutputArtefact,
    /// Caller-supplied metadata, stored verbatim.
    pub metadata: CallerMetadata,
}

impl Manifest {
    /// Builds a manifest for a completed run, stamped with the current tool
    /// version and the current time.
    #[must_use]
    pub fn new(
        transform: TransformRecord,
        record_shape: RecordGeometry,
        source: SourceIdentity,
        output: OutputArtefact,
        metadata: CallerMetadata,
    ) -> Self {
        let (created_at_unix, created_at) = time::now();
        Self {
            manifest_version: MANIFEST_VERSION,
            tool: ToolIdentity::current(),
            created_at,
            created_at_unix,
            transform,
            record_shape,
            source_record_shape: None,
            source,
            output,
            metadata,
        }
    }

    /// Records the source layout a representation transform read, when it
    /// differs from the published one.
    ///
    /// A transform that leaves the layout alone must not call this: an absent
    /// `source_record_shape` is how a reader knows both corpora share a layout.
    #[must_use]
    pub fn with_source_record_shape(mut self, shape: RecordGeometry) -> Self {
        self.source_record_shape = Some(shape);
        self
    }

    /// Writes the manifest as `manifest.json` inside `directory`, returning the
    /// path written.
    ///
    /// The bytes are flushed and synced before the call returns, so a manifest
    /// that this call reports as written is on the volume — a derived corpus is
    /// only published once its provenance is durable.
    ///
    /// # Errors
    ///
    /// Returns [`ManifestError::Json`] when the manifest cannot be encoded —
    /// a source path that is not valid UTF-8, for instance — and
    /// [`ManifestError::Io`] when it cannot be written.
    pub fn write_into(&self, directory: impl AsRef<Path>) -> Result<PathBuf, ManifestError> {
        let path = directory.as_ref().join(MANIFEST_FILE_NAME);
        let mut json = serde_json::to_vec_pretty(self)?;
        json.push(b'\n');

        let mut file = File::create(&path).map_err(|e| ManifestError::io(&path, e))?;
        file.write_all(&json)
            .map_err(|e| ManifestError::io(&path, e))?;
        file.sync_all().map_err(|e| ManifestError::io(&path, e))?;

        Ok(path)
    }

    /// Reads a manifest back from `path`.
    ///
    /// # Errors
    ///
    /// Returns [`ManifestError::Io`] when the file cannot be read and
    /// [`ManifestError::Json`] when it is not a manifest.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ManifestError> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(|e| ManifestError::io(path, e))?;
        Ok(serde_json::from_slice(&bytes)?)
    }
}

/// The tool that produced a derived corpus.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolIdentity {
    /// The crate name.
    pub name: String,
    /// The crate version.
    pub version: String,
}

impl ToolIdentity {
    /// The identity of this build.
    #[must_use]
    pub fn current() -> Self {
        Self {
            name: env!("CARGO_PKG_NAME").to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

/// The transform that was run, with everything needed to run it again.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransformRecord {
    /// The transform name — `sample`, for instance.
    pub name: String,
    /// Its parameters, keyed by flag name.
    pub parameters: BTreeMap<String, serde_json::Value>,
    /// The seed the run used, when the transform takes one.
    pub seed: Option<u64>,
}

impl TransformRecord {
    /// Records `name`, its `parameters` and the `seed` actually used.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        parameters: BTreeMap<String, serde_json::Value>,
        seed: Option<u64>,
    ) -> Self {
        Self {
            name: name.into(),
            parameters,
            seed,
        }
    }
}

/// The record layout of both corpora.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordGeometry {
    /// Input values per record.
    pub inputs: usize,
    /// Output values per record.
    pub outputs: usize,
    /// Values per record — `inputs + outputs`.
    pub record_values: usize,
    /// Bytes per record.
    pub bytes_per_record: usize,
    /// How each value is encoded on disk.
    pub encoding: String,
}

impl From<RecordShape> for RecordGeometry {
    fn from(shape: RecordShape) -> Self {
        Self {
            inputs: shape.inputs(),
            outputs: shape.outputs(),
            record_values: shape.record_values(),
            bytes_per_record: shape.bytes_per_record(),
            encoding: shape.encoding().name().to_string(),
        }
    }
}

/// What the run read, and how those files are identified.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceIdentity {
    /// The canonical source path.
    pub path: PathBuf,
    /// How a file is identified — `path+bytes`, never a content hash.
    pub identity_strategy: String,
    /// How many files were read.
    pub file_count: usize,
    /// How many records were read across them.
    pub record_count: u64,
    /// The files, in the order the run read them.
    pub files: Vec<SourceFile>,
}

impl SourceIdentity {
    /// Records a source `path` read as `files`, holding `record_count` records.
    #[must_use]
    pub fn new(path: PathBuf, files: Vec<SourceFile>, record_count: u64) -> Self {
        Self {
            path,
            identity_strategy: IDENTITY_STRATEGY.to_string(),
            file_count: files.len(),
            record_count,
            files,
        }
    }
}

/// One source file, identified by name and byte length.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceFile {
    /// The file name inside the source directory.
    pub name: String,
    /// Its byte length when it was read.
    pub bytes: u64,
}

/// The derived corpus that was published.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputArtefact {
    /// The corpus file name inside the published directory.
    pub file: String,
    /// Records written.
    pub record_count: u64,
    /// Bytes written.
    pub bytes: u64,
    /// The fingerprint of those bytes.
    pub checksum: Checksum,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corpus::ValueEncoding;

    fn geometry() -> RecordGeometry {
        RecordShape::new(2, 1).expect("valid shape").into()
    }

    fn manifest() -> Manifest {
        Manifest::new(
            TransformRecord::new("sample", BTreeMap::new(), Some(7)),
            geometry(),
            SourceIdentity::new(
                PathBuf::from("/data/trainData-binary"),
                vec![SourceFile {
                    name: "shard-a.bin".to_string(),
                    bytes: 120,
                }],
                10,
            ),
            OutputArtefact {
                file: "sample-100.bin".to_string(),
                record_count: 10,
                bytes: 120,
                checksum: Checksum {
                    algorithm: "sha256".to_string(),
                    value: "00".repeat(32),
                },
            },
            CallerMetadata::default(),
        )
    }

    #[test]
    fn derives_the_geometry_from_the_record_shape() {
        let geometry = geometry();

        assert_eq!(geometry.inputs, 2);
        assert_eq!(geometry.outputs, 1);
        assert_eq!(geometry.record_values, 3);
        assert_eq!(geometry.bytes_per_record, 12);
        assert_eq!(geometry.encoding, "float32");
    }

    #[test]
    fn leaves_the_source_layout_out_when_a_transform_does_not_change_it() {
        let manifest = manifest();
        let json = serde_json::to_string(&manifest).expect("encode the manifest");

        assert_eq!(manifest.source_record_shape, None);
        assert!(
            !json.contains("source_record_shape"),
            "a layout-preserving transform writes no source_record_shape: {json}"
        );
    }

    #[test]
    fn records_the_source_layout_when_a_transform_changes_it() {
        let narrow: RecordGeometry = RecordShape::with_encoding(2, 1, ValueEncoding::BFloat16)
            .expect("valid shape")
            .into();
        let manifest = Manifest::new(
            TransformRecord::new("quantise", BTreeMap::new(), None),
            narrow,
            SourceIdentity::new(PathBuf::from("/data"), Vec::new(), 0),
            OutputArtefact {
                file: "quantise-bfloat16.bin".to_string(),
                record_count: 0,
                bytes: 0,
                checksum: Checksum {
                    algorithm: "sha256".to_string(),
                    value: "00".repeat(32),
                },
            },
            CallerMetadata::default(),
        )
        .with_source_record_shape(geometry());

        assert_eq!(manifest.record_shape.encoding, "bfloat16");
        assert_eq!(manifest.record_shape.bytes_per_record, 6);
        let source = manifest
            .source_record_shape
            .as_ref()
            .expect("the source layout is recorded");
        assert_eq!(source.encoding, "float32");
        assert_eq!(source.bytes_per_record, 12);
    }

    #[test]
    fn counts_the_source_files_it_was_given() {
        let source = SourceIdentity::new(PathBuf::from("/data"), Vec::new(), 0);

        assert_eq!(source.file_count, 0);
        assert_eq!(source.identity_strategy, "path+bytes");
    }

    #[test]
    fn round_trips_through_json() {
        let directory = std::env::temp_dir().join(format!(
            "neat-ai-refinery-manifest-{}-{}",
            std::process::id(),
            line!()
        ));
        std::fs::create_dir_all(&directory).expect("create the directory");
        let original = manifest();

        let path = original.write_into(&directory).expect("write the manifest");
        let loaded = Manifest::load(&path).expect("read the manifest back");

        assert_eq!(path, directory.join(MANIFEST_FILE_NAME));
        assert_eq!(loaded, original);
        std::fs::remove_dir_all(&directory).expect("remove the directory");
    }

    #[test]
    fn fails_loud_when_the_manifest_cannot_be_written() {
        let error = manifest()
            .write_into("/does/not/exist")
            .expect_err("an unwritable manifest is fatal");

        assert!(matches!(error, ManifestError::Io { .. }), "{error:?}");
    }

    #[test]
    fn fails_loud_on_a_file_that_is_not_a_manifest() {
        let path = std::env::temp_dir().join(format!(
            "neat-ai-refinery-not-a-manifest-{}-{}.json",
            std::process::id(),
            line!()
        ));
        std::fs::write(&path, b"{\"manifest_version\":1}").expect("write the fixture");

        let error = Manifest::load(&path).expect_err("an incomplete manifest is rejected");

        assert!(matches!(error, ManifestError::Json { .. }), "{error:?}");
        std::fs::remove_file(&path).expect("remove the fixture");
    }
}
