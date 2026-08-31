//! Staging and atomic publication of a derived corpus.
//!
//! A derived corpus is never built in the directory readers resolve. It is
//! built in a staging directory beside it and swapped in with `rename(2)`, so
//! a reader sees either the previous corpus or the new one — never an empty
//! slot or a half-written file. This is the behaviour of GRQ's
//! `publishSamplerDir`, whose in-place `emptyDirSync` predecessor raced live
//! readers into `NotFound` failures.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::SampleError;

/// A derived corpus being built out of sight of its readers.
///
/// The staging directory is created next to the destination — the same
/// filesystem, so the publishing rename is atomic — and is removed again if
/// the corpus is dropped without being published.
///
/// ```no_run
/// use neat_ai_refinery::sample::StagedCorpus;
///
/// let staged = StagedCorpus::create("trainData-binary-sampler")?;
/// std::fs::write(staged.path().join("sample-5.bin"), b"records")?;
/// staged.publish()?;                    // the whole directory swaps in at once
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug)]
pub struct StagedCorpus {
    staging: PathBuf,
    destination: PathBuf,
    published: bool,
}

impl StagedCorpus {
    /// Creates a staging directory for the corpus to be published at
    /// `destination`.
    ///
    /// # Errors
    ///
    /// Returns [`SampleError::Io`] when the destination has no file name, its
    /// parent directory does not exist, or the staging directory cannot be
    /// created.
    pub fn create(destination: impl AsRef<Path>) -> Result<Self, SampleError> {
        let destination = destination.as_ref().to_path_buf();
        let name = file_name(&destination)?;
        let parent = parent_of(&destination);

        let staging = parent.join(format!(".{name}.staging-{}", unique_suffix()));
        fs::create_dir(&staging).map_err(|e| SampleError::io(&staging, e))?;

        Ok(Self {
            staging,
            destination,
            published: false,
        })
    }

    /// The staging directory to build the corpus in.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.staging
    }

    /// The live directory the corpus will be published as.
    #[must_use]
    pub fn destination(&self) -> &Path {
        &self.destination
    }

    /// Swaps the staged corpus in as the live one.
    ///
    /// Any directory already at the destination is renamed aside first, the
    /// staging directory is renamed into its place, and only then is the aside
    /// copy removed. A rename that fails rolls the previous corpus back, so a
    /// failed publish never leaves readers without a directory.
    ///
    /// # Errors
    ///
    /// Returns [`SampleError::Publish`] when either rename fails.
    pub fn publish(mut self) -> Result<(), SampleError> {
        let aside = parent_of(&self.destination).join(format!(
            "{}.deleting-{}",
            file_name(&self.destination)?,
            unique_suffix()
        ));

        let renamed_aside = match fs::rename(&self.destination, &aside) {
            Ok(()) => true,
            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
            Err(error) => return Err(self.publish_failed(error)),
        };

        if let Err(error) = fs::rename(&self.staging, &self.destination) {
            if renamed_aside {
                // Best effort: the original failure is what the caller needs,
                // but readers must not be left staring at an empty slot.
                let _ = fs::rename(&aside, &self.destination);
            }
            return Err(self.publish_failed(error));
        }

        self.published = true;
        if renamed_aside {
            // The previous corpus is already unreachable by path; a reader
            // holding an open descriptor keeps it until it closes.
            let _ = fs::remove_dir_all(&aside);
        }
        Ok(())
    }

    /// Builds the publish failure for `error`.
    fn publish_failed(&self, error: io::Error) -> SampleError {
        SampleError::Publish {
            staging: self.staging.clone(),
            destination: self.destination.clone(),
            source: error,
        }
    }
}

impl Drop for StagedCorpus {
    fn drop(&mut self) {
        if self.published {
            return;
        }
        // An abandoned staging directory is scratch: reclaim it so a failed
        // run does not leave the volume filling up run after run.
        let _ = fs::remove_dir_all(&self.staging);
    }
}

/// The file name of `path`, as a string.
fn file_name(path: &Path) -> Result<String, SampleError> {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or_else(|| {
            SampleError::io(
                path,
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "a derived corpus directory needs a file name",
                ),
            )
        })
}

/// The directory `path` sits in, treating an empty parent as the current one.
fn parent_of(path: &Path) -> PathBuf {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// A suffix unique to this process and moment, so concurrent samplers on one
/// host cannot collide on a staging or aside directory.
fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    format!("{nanos}-{}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn treats_a_bare_name_as_living_in_the_current_directory() {
        assert_eq!(parent_of(Path::new("derived")), PathBuf::from("."));
        assert_eq!(
            parent_of(Path::new("/data/derived")),
            PathBuf::from("/data")
        );
    }

    #[test]
    fn rejects_a_destination_with_no_file_name() {
        let error = file_name(Path::new("/")).expect_err("the root has no file name");

        assert!(matches!(error, SampleError::Io { .. }), "{error:?}");
    }

    #[test]
    fn suffixes_carry_the_process_id() {
        let suffix = unique_suffix();

        assert!(
            suffix.ends_with(&format!("-{}", std::process::id())),
            "{suffix}"
        );
    }
}
