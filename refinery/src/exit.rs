//! The exit codes a failed run reports.
//!
//! Every failure exits non-zero, but a caller needs more than "it failed" from
//! one of them: a run that stopped because the target volume filled up is worth
//! retrying once space is freed, and no other failure is. Refinery therefore
//! reports POSIX `ENOSPC` — exit 28 — for an out-of-space failure and exit 1
//! for everything else, so an orchestrator can gate a retry on the code alone
//! rather than by matching on a message it does not own.
//!
//! The classification reads the error chain rather than its text: every
//! transform keeps the originating [`std::io::Error`] as the source of the
//! error it reports, so a full volume is recognised however deeply the failure
//! is wrapped.
//!
//! ```
//! use std::io;
//! use std::path::PathBuf;
//!
//! use neat_ai_refinery::cli::CliError;
//! use neat_ai_refinery::exit::{code_for, STORAGE_FULL};
//! use neat_ai_refinery::sample::SampleError;
//!
//! let error = CliError::Sample(SampleError::Io {
//!     path: PathBuf::from("trainData-binary-sampler/sample-5.bin"),
//!     source: io::Error::from(io::ErrorKind::StorageFull),
//! });
//! assert_eq!(code_for(&error), STORAGE_FULL);
//! ```

use std::error::Error;
use std::io;

/// The exit code a run reports when the target volume is full: POSIX `ENOSPC`.
pub const STORAGE_FULL: u8 = 28;

/// The exit code every other failed run reports.
pub const FAILURE: u8 = 1;

/// The `errno` a POSIX platform raises when a volume is full.
const ENOSPC: i32 = 28;

/// The exit code that reports `error`.
///
/// [`STORAGE_FULL`] when the failure — or anything it wraps — is an
/// out-of-space write, [`FAILURE`] otherwise.
#[must_use]
pub fn code_for(error: &(dyn Error + 'static)) -> u8 {
    if is_storage_full(error) {
        STORAGE_FULL
    } else {
        FAILURE
    }
}

/// Whether `error`, or any error it wraps, failed because the volume is full.
#[must_use]
pub fn is_storage_full(error: &(dyn Error + 'static)) -> bool {
    let filesystem = error.downcast_ref::<io::Error>();
    if filesystem.is_some_and(out_of_space) {
        return true;
    }

    // Both ways a failure carries another: the source chain every error type in
    // this crate keeps, and the payload an `io::Error` can be built around.
    let payload = filesystem.and_then(io::Error::get_ref);
    payload.is_some_and(|payload| is_storage_full(payload))
        || error.source().is_some_and(is_storage_full)
}

/// Whether one filesystem failure is an out-of-space one.
///
/// The raw `errno` is checked as well as the mapped kind: a platform that does
/// not map `ENOSPC` to [`io::ErrorKind::StorageFull`] still reports the number.
/// It is only consulted on Unix, where 28 is `ENOSPC` — the number means
/// something else elsewhere, and a full disk on Windows is already mapped.
fn out_of_space(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::StorageFull
        || (cfg!(unix) && error.raw_os_error() == Some(ENOSPC))
}

#[cfg(test)]
mod tests {
    use std::fmt;
    use std::path::PathBuf;

    use super::*;
    use crate::corpus::CorpusError;
    use crate::sample::SampleError;

    /// An application error that keeps no source, standing in for a failure
    /// that has nothing to do with the filesystem.
    #[derive(Debug)]
    struct Opaque;

    impl fmt::Display for Opaque {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("an opaque failure")
        }
    }

    impl Error for Opaque {}

    /// A sampling failure wrapping `source` as its corpus I/O failure.
    fn sample_io(source: io::Error) -> SampleError {
        SampleError::Corpus(CorpusError::Io {
            path: PathBuf::from("trainData-binary-sampler/sample-5.bin"),
            source,
        })
    }

    #[test]
    fn the_mapped_kind_is_out_of_space() {
        let error = sample_io(io::Error::from(io::ErrorKind::StorageFull));

        assert_eq!(code_for(&error), STORAGE_FULL);
    }

    #[test]
    fn the_raw_enospc_number_is_out_of_space() {
        let error = sample_io(io::Error::from_raw_os_error(ENOSPC));

        assert_eq!(code_for(&error), STORAGE_FULL);
    }

    #[test]
    fn an_io_error_wrapping_an_out_of_space_one_is_found() {
        let error = sample_io(io::Error::other(io::Error::from_raw_os_error(ENOSPC)));

        assert_eq!(code_for(&error), STORAGE_FULL);
    }

    #[test]
    fn another_filesystem_failure_is_an_ordinary_one() {
        let error = sample_io(io::Error::from(io::ErrorKind::PermissionDenied));

        assert_eq!(code_for(&error), FAILURE);
        assert!(!is_storage_full(&error));
    }

    #[test]
    fn a_failure_carrying_no_filesystem_error_is_an_ordinary_one() {
        assert_eq!(code_for(&Opaque), FAILURE);
    }
}
