# Add social preview image to README and refresh it to current state

## Summary

Puts the project's social preview banner at the top of `README.md`, hot-linked
from the NEAT-AI brand directory on `Develop` — the same pattern
NEAT-AI-scorer and NEAT-AI-Ockham use, so all the sibling banners move together
when the brand assets are refreshed. Closes #29.

The rest of the README was audited against the code rather than rewritten: the
transform sections had already been kept current by the `quantise` (#11),
`fuzz` (#12), `pipeline` (#13) and benchmark (#14) PRs, so the only stale claim
left was step 5 of **Migration principle**, which now names obsolete-sampler
removal as the one outstanding step and links [#9].

What landed:

- `README.md` — the banner, and the migration step-5 status.
- `refinery/tests/readme_surface.rs` — four tests that hold the README to the
  code, so the next drift fails `cargo test` instead of being found by a reader.

The audit found nothing else to change. Recorded here so a reviewer does not
have to repeat it:

| Checked | Against | Result |
| --- | --- | --- |
| Every flag in a README invocation | `Cli::command()` — `--source`, `--output`, `--inputs`, `--outputs`, `--metadata`, and each subcommand's flags | matches |
| Every subcommand named | `sample`, `quantise`, `fuzz`, `pipeline` | all four documented |
| Every relative link | files in the tree | all resolve |
| Every `docs/*.md` page | linked from the README | all nine linked |
| Soak and benchmark tables | `docs/evidence/soak-linux-aarch64.md`, `docs/evidence/bench-linux-aarch64.md` | numbers identical |
| Workflow table | `.github/workflows/` | matches; `security.yml` is the reusable workflow `ci.yml` calls, so it is in the CI graph rather than the standalone table |
| Manifest example `tool.version` | `refinery/Cargo.toml` | `0.1.0` |

The banner uses the linked-image form
(`[![alt](raw…png)](blob…png)`), matching NEAT-AI-Ockham, so clicking it opens
the asset in the brand repository.

## Evidence

The README rendered with the banner in place — the image is hot-linked, so this
also proves the raw URL resolves:

![README rendered with the social preview banner at the top](docs/evidence/issue-29-readme-social-preview.png)

`refinery/tests/readme_surface.rs`, run against this tree:

```text
running 4 tests
test readme_opens_with_the_social_preview_banner ... ok
test readme_documents_every_transform_subcommand ... ok
test readme_relative_links_resolve_to_committed_files ... ok
test readme_shows_no_flag_the_cli_does_not_accept ... ok

test result: ok. 4 passed; 0 failed; 0 ignored
```

Each test drives real code rather than inspecting source text: the flag and
subcommand tests build the actual `clap` command from `neat_ai_refinery::cli`
and compare it with what the README shows, and the link test resolves every
non-fenced Markdown target against the working tree.

## Test Plan

- Added `refinery/tests/readme_surface.rs`:
  - `readme_opens_with_the_social_preview_banner` — the first non-blank line
    after the heading is a Markdown image carrying the brand raw URL. Fails if
    the banner is removed, demoted below the tagline, or written as a bare link.
  - `readme_relative_links_resolve_to_committed_files` — every in-repository
    link and image target exists; targets inside fenced blocks are examples and
    are skipped.
  - `readme_documents_every_transform_subcommand` — every subcommand `clap`
    exposes is named in the README, so a new transform cannot ship undocumented.
  - `readme_shows_no_flag_the_cli_does_not_accept` — every `--flag` in a README
    invocation block is a flag the CLI actually parses.
- `./quality.sh` run in full: cargo-deny, `cargo fmt --check`, clippy with
  `-D warnings`, the whole workspace test suite, and `cargo doc`.

[#9]: https://github.com/stSoftwareAU/NEAT-AI-Refinery/issues/9
