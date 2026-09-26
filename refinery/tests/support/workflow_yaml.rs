//! A line-based reader for the `uses:` / `with:` pairs in workflow YAML,
//! shared by the drift gates in `rust_setup_action.rs` (Issue #43) and
//! `corpus_runner.rs` (Issue #68).

use std::collections::BTreeMap;

/// The indentation width of `line`, in spaces.
fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// Returns the `with:` inputs of every `uses: <action>` step in `yaml`, one map
/// per invocation. A step with no `with:` block yields an empty map. The
/// `@<revision>` pin and any trailing comment are ignored, so `actions/checkout`
/// matches whichever SHA it is currently pinned to.
pub fn invocation_inputs(yaml: &str, action: &str) -> Vec<BTreeMap<String, String>> {
    let mut invocations = Vec::new();
    let mut lines = yaml.lines().peekable();

    while let Some(raw_line) = lines.next() {
        let line = raw_line.trim();
        if line.starts_with('#') {
            continue;
        }
        let stripped = line.strip_prefix("- ").unwrap_or(line);
        let Some(value) = stripped.strip_prefix("uses:") else {
            continue;
        };
        let reference = value.split('#').next().unwrap_or(value).trim();
        let name = reference.split('@').next().unwrap_or(reference);
        if name != action {
            continue;
        }

        // The `with:` block, when present, is a sibling of `uses:` — same
        // indentation, deeper-indented keys beneath it.
        let step_indent = indent_of(raw_line.strip_prefix("- ").unwrap_or(raw_line));
        let mut inputs = BTreeMap::new();
        let mut in_with = false;
        while let Some(next) = lines.peek() {
            let trimmed = next.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                lines.next();
                continue;
            }
            let next_indent = indent_of(next);
            if next_indent < step_indent || (next_indent == step_indent && in_with) {
                break;
            }
            if next_indent == step_indent {
                if trimmed == "with:" {
                    in_with = true;
                    lines.next();
                    continue;
                }
                break;
            }
            if in_with {
                if let Some((key, value)) = trimmed.split_once(':') {
                    inputs.insert(
                        key.trim().to_string(),
                        value.trim().trim_matches('"').to_string(),
                    );
                }
            }
            lines.next();
        }
        invocations.push(inputs);
    }

    invocations
}
