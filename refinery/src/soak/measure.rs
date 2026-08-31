//! Timing and peak-memory measurement of one sampler run.
//!
//! Both implementations are measured by the *same* code, as child processes,
//! so the throughput and peak-RSS numbers in a report can be compared. Peak
//! memory is sampled rather than instrumented: on Linux the kernel's own
//! high-water mark (`VmHWM`) is read, and elsewhere `ps` is polled and the
//! largest sample kept. The method used is recorded beside the number so a
//! reader is never left guessing which it is.

use std::ffi::OsString;
use std::fs::File;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::SoakError;

/// How often the child's memory use is sampled.
const POLL_INTERVAL: Duration = Duration::from_millis(25);

/// How much of a failed run's diagnostics is carried in the error.
const STDERR_LIMIT: usize = 4096;

/// What one measured run cost.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunMeasurement {
    /// The label the run was measured under — `refinery-1`, `typescript`.
    pub label: String,
    /// Wall-clock duration of the process, in milliseconds.
    pub elapsed_ms: u64,
    /// Peak resident set size in KiB, or `None` when the process finished
    /// before the first sample could be taken.
    pub peak_rss_kib: Option<u64>,
    /// How that peak was obtained.
    pub peak_rss_method: String,
}

/// A child process to run, time and sample.
///
/// Output is captured to `<label>.out` and `<label>.err` inside the capture
/// directory rather than into a pipe, so a chatty run cannot deadlock the
/// sampling loop by filling a pipe nobody is draining.
#[derive(Debug, Clone)]
pub struct MeasuredCommand {
    label: String,
    program: PathBuf,
    args: Vec<OsString>,
    capture: PathBuf,
    working_dir: Option<PathBuf>,
}

impl MeasuredCommand {
    /// A command to be measured under `label`, capturing output into
    /// `capture`.
    #[must_use]
    pub fn new(
        label: impl Into<String>,
        program: impl Into<PathBuf>,
        capture: impl Into<PathBuf>,
    ) -> Self {
        Self {
            label: label.into(),
            program: program.into(),
            args: Vec::new(),
            capture: capture.into(),
            working_dir: None,
        }
    }

    /// Appends one argument.
    #[must_use]
    pub fn arg(mut self, arg: impl Into<OsString>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Appends several arguments.
    #[must_use]
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// Runs the child from `directory`.
    #[must_use]
    pub fn current_dir(mut self, directory: impl Into<PathBuf>) -> Self {
        self.working_dir = Some(directory.into());
        self
    }

    /// Where this run's captured standard output was written.
    #[must_use]
    pub fn stdout_path(&self) -> PathBuf {
        self.capture.join(format!("{}.out", self.label))
    }

    /// Where this run's captured standard error was written.
    #[must_use]
    pub fn stderr_path(&self) -> PathBuf {
        self.capture.join(format!("{}.err", self.label))
    }

    /// Runs the command to completion, timing it and sampling its memory.
    ///
    /// # Errors
    ///
    /// Returns [`SoakError::Spawn`] when the program cannot be started,
    /// [`SoakError::CommandFailed`] when it exits non-zero — a failed sampler
    /// is never measured as a cheap success — and [`SoakError::Io`] when the
    /// capture files cannot be created or the child cannot be waited on.
    pub fn measure(&self) -> Result<RunMeasurement, SoakError> {
        let stdout_path = self.stdout_path();
        let stderr_path = self.stderr_path();
        let stdout = File::create(&stdout_path).map_err(|e| SoakError::io(&stdout_path, e))?;
        let stderr = File::create(&stderr_path).map_err(|e| SoakError::io(&stderr_path, e))?;

        let mut command = Command::new(&self.program);
        command
            .args(&self.args)
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
        if let Some(directory) = &self.working_dir {
            command.current_dir(directory);
        }

        let started = Instant::now();
        let mut child = command.spawn().map_err(|source| SoakError::Spawn {
            program: self.program.display().to_string(),
            source,
        })?;

        let pid = child.id();
        let mut peak: Option<u64> = None;
        let status = loop {
            if let Some(sample) = sample_rss_kib(pid) {
                peak = Some(peak.map_or(sample, |seen: u64| seen.max(sample)));
            }
            match child
                .try_wait()
                .map_err(|e| SoakError::io(&self.program, e))?
            {
                Some(status) => break status,
                None => thread::sleep(POLL_INTERVAL),
            }
        };
        let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

        if !status.success() {
            return Err(SoakError::CommandFailed {
                label: self.label.clone(),
                code: status.code(),
                stderr: tail_of(&stderr_path),
            });
        }

        Ok(RunMeasurement {
            label: self.label.clone(),
            elapsed_ms,
            peak_rss_kib: peak,
            peak_rss_method: RSS_METHOD.to_string(),
        })
    }
}

/// The last of a capture file, for a failure message.
///
/// A capture that cannot be read is reported as such rather than as an empty
/// diagnostic, so a failure never looks like a silent one.
fn tail_of(path: &std::path::Path) -> String {
    match std::fs::read_to_string(path) {
        Ok(text) if text.len() <= STDERR_LIMIT => text,
        Ok(text) => text[text.len() - STDERR_LIMIT..].to_string(),
        Err(error) => format!(
            "(the captured diagnostics at {} could not be read: {error})",
            path.display()
        ),
    }
}

/// How peak resident memory is obtained on this target.
#[cfg(target_os = "linux")]
const RSS_METHOD: &str = "proc-vmhwm";

/// How peak resident memory is obtained on this target.
#[cfg(not(target_os = "linux"))]
const RSS_METHOD: &str = "ps-rss-sampled";

/// The kernel's own high-water mark for the process, in KiB.
#[cfg(target_os = "linux")]
fn sample_rss_kib(pid: u32) -> Option<u64> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmHWM:") {
            return rest.split_whitespace().next()?.parse().ok();
        }
    }
    None
}

/// The process's current resident set size in KiB, via `ps`.
#[cfg(not(target_os = "linux"))]
fn sample_rss_kib(pid: u32) -> Option<u64> {
    let output = Command::new("ps")
        .args(["-o", "rss="])
        .arg("-p")
        .arg(pid.to_string())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout).trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_the_capture_files_after_the_label() {
        let command = MeasuredCommand::new("refinery-1", "neat_ai_refinery", "/tmp/logs");

        assert_eq!(
            command.stdout_path(),
            PathBuf::from("/tmp/logs/refinery-1.out")
        );
        assert_eq!(
            command.stderr_path(),
            PathBuf::from("/tmp/logs/refinery-1.err")
        );
    }

    #[test]
    fn reports_an_unreadable_capture_rather_than_an_empty_one() {
        let tail = tail_of(std::path::Path::new("/does/not/exist/refinery.err"));

        assert!(tail.contains("could not be read"), "{tail}");
    }

    #[test]
    fn samples_the_memory_of_the_running_process() {
        let sample = sample_rss_kib(std::process::id());

        assert!(
            sample.is_none_or(|kib| kib > 0),
            "a live process reported {sample:?} KiB"
        );
    }
}
