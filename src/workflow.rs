use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::{Error, Result};

#[derive(Debug, Clone)]
pub struct ActionRef {
    pub file: PathBuf,
    /// e.g. "actions/checkout"
    pub action: String,
    /// e.g. "v4" or a 40-char SHA
    pub ref_str: String,
    /// The full original token as it appears in the YAML, e.g. "actions/checkout@v4"
    pub raw: String,
    /// Inline comment on the same line as the uses: directive, e.g. "v4" from "# v4"
    pub inline_comment: Option<String>,
}

#[derive(Deserialize)]
struct Workflow {
    jobs: Option<HashMap<String, Job>>,
}

#[derive(Deserialize)]
struct Job {
    steps: Option<Vec<Step>>,
}

#[derive(Deserialize)]
struct Step {
    uses: Option<String>,
}

pub fn find_workflow_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let entries = fs::read_dir(dir)?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() {
            if let Some(ext) = path.extension() {
                if ext == "yml" || ext == "yaml" {
                    files.push(path);
                }
            }
        }
    }
    files.sort();
    Ok(files)
}

pub fn extract_action_refs(file: &Path) -> Result<Vec<ActionRef>> {
    let content = fs::read_to_string(file)?;
    let workflow: Workflow = serde_yaml::from_str(&content).map_err(|e| Error::Yaml {
        file: file.to_path_buf(),
        source: e,
    })?;

    let mut refs = Vec::new();
    let jobs = match workflow.jobs {
        Some(j) => j,
        None => return Ok(refs),
    };

    for job in jobs.values() {
        let steps = match &job.steps {
            Some(s) => s,
            None => continue,
        };
        for step in steps {
            let uses = match &step.uses {
                Some(u) => u.trim().to_string(),
                None => continue,
            };

            // Skip docker:// and local path actions
            if uses.starts_with("docker://") || uses.starts_with('.') {
                continue;
            }

            // Split on @ to get action and ref
            let (action, ref_str) = match uses.split_once('@') {
                Some((a, r)) => (a.to_string(), r.to_string()),
                None => {
                    eprintln!("warning: skipping uses without @: {uses}");
                    continue;
                }
            };

            // Look for an inline comment in the raw file content for this uses line
            // e.g. "uses: actions/checkout@abc123 # v4"
            let inline_comment = find_inline_comment(&content, &uses);

            refs.push(ActionRef {
                file: file.to_path_buf(),
                action,
                ref_str,
                raw: uses,
                inline_comment,
            });
        }
    }

    Ok(refs)
}

/// Extract lines surrounding the `uses: {uses_raw}` directive for display in the TUI.
/// Returns (lines, highlight_index) where highlight_index is the index of the uses: line.
pub fn extract_context(file: &Path, uses_raw: &str, context: usize) -> (Vec<String>, usize) {
    let content = match fs::read_to_string(file) {
        Ok(c) => c,
        Err(_) => return (Vec::new(), 0),
    };
    let lines: Vec<&str> = content.lines().collect();
    let pos = lines.iter().position(|l| {
        // A step line is typically "        - uses: owner/repo@ref", so after trimming
        // whitespace we may still have a leading "- " YAML list marker to strip.
        let trimmed = l.trim();
        let after_dash = trimmed
            .strip_prefix('-')
            .map(|s| s.trim())
            .unwrap_or(trimmed);
        if let Some(after_uses) = after_dash.strip_prefix("uses:") {
            let val = after_uses.trim();
            val == uses_raw || val.starts_with(&format!("{uses_raw} #"))
        } else {
            false
        }
    });
    match pos {
        Some(pos) => {
            let start = pos.saturating_sub(context);
            let end = (pos + context + 1).min(lines.len());
            let highlight = pos - start;
            (
                lines[start..end].iter().map(|l| l.to_string()).collect(),
                highlight,
            )
        }
        None => (Vec::new(), 0),
    }
}

/// Search the raw file content for a `uses:` line containing `uses_value` and extract
/// any trailing comment (the text after `#`).
fn find_inline_comment(content: &str, uses_value: &str) -> Option<String> {
    for line in content.lines() {
        let trimmed = line.trim();
        // Step lines are YAML list items: "        - uses: owner/repo@sha # label"
        // Strip the optional leading "- " before checking for "uses:".
        let after_dash = trimmed
            .strip_prefix('-')
            .map(|s| s.trim())
            .unwrap_or(trimmed);
        if !after_dash.starts_with("uses:") {
            continue;
        }
        let after_uses = after_dash["uses:".len()..].trim();
        if let Some((value_part, comment_part)) = after_uses.split_once('#') {
            if value_part.trim() == uses_value {
                let comment = comment_part.trim().to_string();
                if !comment.is_empty() {
                    return Some(comment);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_inline_comment_list_item_with_dash() {
        // The common case: step as a YAML list item with leading indentation and dash
        let content = "      - uses: dtolnay/rust-toolchain@29eef336d9b2848a0b548edc03f92a220660cdb8 # stable\n";
        assert_eq!(
            find_inline_comment(
                content,
                "dtolnay/rust-toolchain@29eef336d9b2848a0b548edc03f92a220660cdb8"
            ),
            Some("stable".to_string())
        );
    }

    #[test]
    fn find_inline_comment_bare_uses_line() {
        // Less common but valid: no leading dash
        let content = "uses: actions/checkout@abc123def456abc123def456abc123def456abc123 # v4\n";
        assert_eq!(
            find_inline_comment(
                content,
                "actions/checkout@abc123def456abc123def456abc123def456abc123"
            ),
            Some("v4".to_string())
        );
    }

    #[test]
    fn find_inline_comment_no_comment_returns_none() {
        let content =
            "      - uses: dtolnay/rust-toolchain@29eef336d9b2848a0b548edc03f92a220660cdb8\n";
        assert_eq!(
            find_inline_comment(
                content,
                "dtolnay/rust-toolchain@29eef336d9b2848a0b548edc03f92a220660cdb8"
            ),
            None
        );
    }

    #[test]
    fn find_inline_comment_wrong_value_returns_none() {
        let content = "      - uses: dtolnay/rust-toolchain@29eef336d9b2848a0b548edc03f92a220660cdb8 # stable\n";
        assert_eq!(find_inline_comment(content, "actions/checkout@v4"), None);
    }
}
