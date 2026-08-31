//! Staging and atomic publication of a derived corpus — the machinery every
//! transform shares.
//!
//! Readers of the live directory must only ever see the previous corpus or the
//! new one — never an empty or half-built slot — and a failed publish must
//! leave no scratch behind.

// The shared helpers serve several test binaries; not every one uses all of them.
#[allow(dead_code)]
mod common;

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use common::TempDir;
use neat_ai_refinery::transform::{StagedCorpus, TransformError};

/// Every file name in `dir`, sorted.
fn entries(dir: &Path) -> BTreeSet<String> {
    fs::read_dir(dir)
        .expect("read the directory")
        .map(|entry| entry.expect("read the entry").file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .collect()
}

#[test]
fn stages_outside_the_live_directory_and_publishes_it_whole() {
    let temp = TempDir::new("publish-whole");
    let live = temp.path().join("derived");

    let staged = StagedCorpus::create(&live).expect("stage the corpus");
    assert_ne!(staged.path(), live, "the corpus is built out of the way");
    assert!(!live.exists(), "the live path stays absent until publish");
    fs::write(staged.path().join("sample-5.bin"), b"fresh").expect("write the sample");

    staged.publish().expect("publish the corpus");

    assert_eq!(entries(&live), BTreeSet::from(["sample-5.bin".into()]));
    assert_eq!(
        entries(temp.path()),
        BTreeSet::from(["derived".into()]),
        "no staging or aside directory survives a publish"
    );
}

#[test]
fn replaces_a_live_directory_without_emptying_it_in_place() {
    let temp = TempDir::new("publish-replace");
    let live = temp.path().join("derived");
    fs::create_dir_all(&live).expect("create the live directory");
    fs::write(live.join("sample-99.bin"), b"stale").expect("write the stale sample");

    let staged = StagedCorpus::create(&live).expect("stage the corpus");
    fs::write(staged.path().join("sample-5.bin"), b"fresh").expect("write the sample");
    staged.publish().expect("publish the corpus");

    assert_eq!(entries(&live), BTreeSet::from(["sample-5.bin".into()]));
    assert_eq!(entries(temp.path()), BTreeSet::from(["derived".into()]));
}

#[test]
fn restores_the_previous_corpus_when_the_publish_fails() {
    let temp = TempDir::new("publish-rollback");
    let live = temp.path().join("derived");
    fs::create_dir_all(&live).expect("create the live directory");
    fs::write(live.join("sample-99.bin"), b"stale").expect("write the stale sample");

    let staged = StagedCorpus::create(&live).expect("stage the corpus");
    fs::write(staged.path().join("sample-5.bin"), b"fresh").expect("write the sample");
    // The staging directory vanishing underneath the publisher is the loudest
    // available stand-in for a rename that cannot complete.
    fs::remove_dir_all(staged.path()).expect("remove the staging directory");

    let error = staged.publish().expect_err("the publish must fail loud");

    assert!(matches!(error, TransformError::Publish { .. }), "{error:?}");
    assert_eq!(
        entries(&live),
        BTreeSet::from(["sample-99.bin".into()]),
        "the previous corpus is rolled back into place"
    );
    assert_eq!(
        entries(temp.path()),
        BTreeSet::from(["derived".into()]),
        "a failed publish leaves no aside or staging directory"
    );
}

#[test]
fn removes_the_staging_directory_when_it_is_dropped_unpublished() {
    let temp = TempDir::new("publish-abandon");
    let live = temp.path().join("derived");

    let staging = {
        let staged = StagedCorpus::create(&live).expect("stage the corpus");
        fs::write(staged.path().join("sample-5.bin"), b"partial").expect("write the sample");
        staged.path().to_path_buf()
    };

    assert!(
        !staging.exists(),
        "an abandoned staging directory is removed"
    );
    assert!(!live.exists(), "nothing is published");
    assert!(entries(temp.path()).is_empty());
}

#[test]
fn refuses_to_stage_under_a_missing_parent_directory() {
    let temp = TempDir::new("publish-missing-parent");

    let error = StagedCorpus::create(temp.path().join("absent").join("derived"))
        .expect_err("a missing parent is fatal");

    assert!(matches!(error, TransformError::Io { .. }), "{error:?}");
}
