//! The fingerprint of a published artefact.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::ManifestError;

/// The read buffer used while digesting a corpus file.
const BUFFER_BYTES: usize = 256 * 1024;

/// A named digest of an artefact, so a reader knows what the value is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checksum {
    /// The digest algorithm — currently always `sha256`.
    pub algorithm: String,
    /// The digest, lower-case hexadecimal.
    pub value: String,
}

impl Checksum {
    /// Digests the file at `path` with SHA-256.
    ///
    /// The file is streamed through a fixed buffer, so a corpus far larger
    /// than memory is fingerprinted in bounded memory.
    ///
    /// # Errors
    ///
    /// Returns [`ManifestError::Io`] when the file cannot be opened or read.
    pub fn of_file(path: impl AsRef<Path>) -> Result<Self, ManifestError> {
        let path = path.as_ref();
        let mut file = File::open(path).map_err(|e| ManifestError::io(path, e))?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0_u8; BUFFER_BYTES];

        loop {
            let read = file
                .read(&mut buffer)
                .map_err(|e| ManifestError::io(path, e))?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }

        Ok(Self {
            algorithm: "sha256".to_string(),
            value: hex(&hasher.finalize()),
        })
    }
}

/// Lower-case hexadecimal for `bytes`.
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    bytes.iter().fold(String::new(), |mut out, byte| {
        // Writing into a String cannot fail.
        let _ = write!(out, "{byte:02x}");
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_bytes_as_lower_case_hexadecimal() {
        assert_eq!(hex(&[0x00, 0x0f, 0xa5, 0xff]), "000fa5ff");
        assert_eq!(hex(&[]), "");
    }

    #[test]
    fn digests_a_file_to_the_published_sha256_vector() {
        // The NIST test vector for "abc", so the streaming digest is held to a
        // value that does not come from this implementation.
        let path = std::env::temp_dir().join(format!(
            "neat-ai-refinery-checksum-{}-{}.bin",
            std::process::id(),
            line!()
        ));
        std::fs::write(&path, b"abc").expect("write the fixture");

        let checksum = Checksum::of_file(&path).expect("digest the fixture");

        assert_eq!(checksum.algorithm, "sha256");
        assert_eq!(
            checksum.value,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        std::fs::remove_file(&path).expect("remove the fixture");
    }

    #[test]
    fn fails_loud_when_the_artefact_is_missing() {
        let error = Checksum::of_file("/does/not/exist/sample-5.bin")
            .expect_err("a missing artefact cannot be fingerprinted");

        assert!(matches!(error, ManifestError::Io { .. }), "{error:?}");
    }
}
