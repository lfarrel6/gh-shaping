use std::collections::HashMap;
use std::fs;
use std::path::Path;

use regex::Regex;

use crate::error::Result;
use crate::resolver::{RefKey, ResolvedSha, is_sha};
use crate::workflow::ActionRef;

/// Rewrite `uses: action@old_ref` → `uses: action@sha # label` in a file in-place.
/// Returns true if the file was modified.
pub fn rewrite_uses(file: &Path, action: &str, old_ref: &str, sha: &str, label: &str) -> Result<bool> {
    let content = fs::read_to_string(file)?;
    let pattern = format!(
        r"(uses:\s+)({action}@{old_ref})(\s*#[^\n]*)?",
        action = regex::escape(action),
        old_ref = regex::escape(old_ref),
    );
    let re = Regex::new(&pattern).expect("invalid regex");
    let replacement = format!("${{1}}{action}@{sha} # {label}", action = action, sha = sha, label = label);
    let new_content = re.replace_all(&content, replacement.as_str());
    if new_content.as_ref() != content.as_str() {
        fs::write(file, new_content.as_ref())?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Rewrite a workflow file, replacing unpinned action refs with their resolved SHAs.
/// Returns the number of replacements made.
pub fn pin_workflow_file(
    file: &Path,
    resolution_map: &HashMap<RefKey, std::result::Result<ResolvedSha, String>>,
    refs: &[ActionRef],
) -> Result<usize> {
    let mut content = fs::read_to_string(file)?;
    let mut replacements = 0;

    for r in refs {
        // Skip already-pinned refs
        if is_sha(&r.ref_str) {
            continue;
        }

        let key = match RefKey::from_action(&r.action, &r.ref_str) {
            Some(k) => k,
            None => continue,
        };

        let sha = match resolution_map.get(&key) {
            Some(Ok(resolved)) if !resolved.was_pinned => resolved.sha.clone(),
            _ => continue,
        };

        let pattern = format!(
            r"(uses:\s+)({action}@{ref_str})(\s*#[^\n]*)?",
            action = regex::escape(&r.action),
            ref_str = regex::escape(&r.ref_str),
        );
        let re = Regex::new(&pattern).expect("invalid regex");
        let replacement = format!("${{1}}{action}@{sha} # {ref_str}",
            action = r.action,
            sha = sha,
            ref_str = r.ref_str,
        );

        let new_content = re.replace_all(&content, replacement.as_str());
        if new_content != content.as_str() {
            replacements += 1;
            content = new_content.into_owned();
        }
    }

    if replacements > 0 {
        fs::write(file, &content)?;
    }
    Ok(replacements)
}
