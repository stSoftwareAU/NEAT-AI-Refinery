//! Which host produced a soak report.
//!
//! The cut-over needs runs from representative macOS and Linux hosts, so a
//! report that does not say where it came from is not evidence. The facts are
//! taken from the build target and the running process — nothing is asked of
//! the operator, who would have to be trusted to answer accurately.

use serde::{Deserialize, Serialize};

/// The host a soak run was captured on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostFacts {
    /// The operating system — `linux`, `macos`, and so on.
    pub os: String,
    /// The processor architecture — `aarch64`, `x86_64`.
    pub arch: String,
    /// The operating system family — `unix` or `windows`.
    pub family: String,
    /// Logical processors visible to this process, when they can be read.
    pub cpu_count: Option<usize>,
}

impl HostFacts {
    /// The host this process is running on.
    #[must_use]
    pub fn detect() -> Self {
        Self {
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            family: std::env::consts::FAMILY.to_string(),
            cpu_count: std::thread::available_parallelism()
                .ok()
                .map(std::num::NonZeroUsize::get),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifies_the_host_it_runs_on() {
        let host = HostFacts::detect();

        assert!(!host.os.is_empty());
        assert!(!host.arch.is_empty());
        assert_eq!(host.family, std::env::consts::FAMILY);
    }
}
