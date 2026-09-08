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

use crate::sample::SampleError;

/// The name every reported failure is prefixed with, as the binary is invoked.
const PROGRAM: &str = "neat_ai_refinery";

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

/// How many bytes a fresh attempt needs, when the failure knows.
///
/// Only a full volume carries a requirement, and only the sampler computes one
/// — so the chain is searched for a [`SampleError::StorageFull`], however
/// deeply a pipeline stage has wrapped it. `None` is the honest answer
/// everywhere else: a caller gating a retry on the figure refuses rather than
/// acting on a guess.
#[must_use]
pub fn required_bytes(error: &(dyn Error + 'static)) -> Option<u64> {
    error
        .downcast_ref::<SampleError>()
        .and_then(SampleError::required_bytes)
        .or_else(|| error.source().and_then(required_bytes))
}

/// The stderr report of a failed run, ready to print.
///
/// The failure itself is always the first line. A full volume that knows what
/// a fresh attempt costs adds a second, `required_bytes=<n>` — the whole pass
/// a retry writes, in ASCII digits — so a caller frees that much space and
/// retries on evidence instead of on hope. Nothing is added when the
/// requirement is unknown, and nothing is added for any other failure: a
/// figure beside an exit 1 would invite a retry that cannot succeed.
///
/// ```
/// use std::io;
/// use std::path::PathBuf;
///
/// use neat_ai_refinery::exit::failure_report;
/// use neat_ai_refinery::sample::SampleError;
///
/// let full = SampleError::Io {
///     path: PathBuf::from("trainData-binary-sampler/sample-5.bin"),
///     source: io::Error::from(io::ErrorKind::StorageFull),
/// };
/// let report = failure_report(&full.with_required_bytes(4_096));
///
/// assert!(report.lines().any(|line| line.ends_with("required_bytes=4096")));
/// ```
#[must_use]
pub fn failure_report(error: &(dyn Error + 'static)) -> String {
    let mut report = format!("{PROGRAM}: {error}\n");

    // The figure is only ever printed beside the one code it explains.
    let requirement = required_bytes(error).filter(|_| code_for(error) == STORAGE_FULL);
    if let Some(bytes) = requirement {
        report.push_str(&format!("{PROGRAM}: required_bytes={bytes}\n"));
    }
    report
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

    #[test]
    fn a_full_volume_reports_what_a_fresh_attempt_needs() {
        let error =
            sample_io(io::Error::from(io::ErrorKind::StorageFull)).with_required_bytes(1_234_567);

        let report = failure_report(&error);

        assert_eq!(required_bytes(&error), Some(1_234_567));
        assert_eq!(
            report.lines().last(),
            Some("neat_ai_refinery: required_bytes=1234567"),
            "the token is the last thing on its own line, in ASCII digits: {report}"
        );
        assert_eq!(
            report
                .lines()
                .filter(|line| line.contains("required_bytes="))
                .count(),
            1,
            "a caller reading the log must find one figure, not several: {report}"
        );
    }

    #[test]
    fn a_full_volume_that_knows_no_requirement_reports_none() {
        // Every other transform writes without an estimate. Printing a guess
        // would have a caller free the wrong amount and retry into the same
        // wall; saying nothing has it refuse the retry, as it does today.
        let error = sample_io(io::Error::from(io::ErrorKind::StorageFull));

        let report = failure_report(&error);

        assert_eq!(required_bytes(&error), None);
        assert_eq!(code_for(&error), STORAGE_FULL);
        assert!(!report.contains("required_bytes="), "{report}");
    }

    #[test]
    fn an_ordinary_failure_never_reports_a_requirement() {
        // A requirement beside an exit 1 would invite a retry that cannot
        // succeed however much space is freed.
        let error =
            sample_io(io::Error::from(io::ErrorKind::PermissionDenied)).with_required_bytes(4_096);

        let report = failure_report(&error);

        assert_eq!(code_for(&error), FAILURE);
        assert!(!report.contains("required_bytes="), "{report}");
        assert_eq!(
            report,
            format!("neat_ai_refinery: {error}\n"),
            "the failure itself is still reported in full"
        );
    }

    #[test]
    fn a_requirement_is_found_however_deeply_the_failure_is_wrapped() {
        let error = crate::cli::CliError::Sample(
            sample_io(io::Error::from(io::ErrorKind::StorageFull)).with_required_bytes(64),
        );

        assert_eq!(required_bytes(&error), Some(64));
        assert!(failure_report(&error).ends_with("required_bytes=64\n"));
    }

    #[test]
    fn a_requirement_is_added_once_and_not_replaced() {
        // The sampler is the only estimator; a pipeline wrapping its failure
        // must not overwrite the figure with one of its own.
        let error = sample_io(io::Error::from(io::ErrorKind::StorageFull))
            .with_required_bytes(64)
            .with_required_bytes(999);

        assert_eq!(required_bytes(&error), Some(64));
    }
}
