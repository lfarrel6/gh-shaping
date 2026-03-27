use std::collections::HashMap;
use std::fs;
use std::path::Path;

use regex::Regex;

use crate::error::Result;
use crate::resolver::{RefKey, is_sha, list_tags_newest_first, resolve};
use crate::workflow::ActionRef;

pub struct UpdateResult {
    pub action: String,
    pub old_sha: String,
    pub new_sha: String,
    pub tag: String,
    pub updated: bool,
}

/// Process all already-pinned refs, check for newer releases, and rewrite files.
pub fn run_updates(all_refs: &[ActionRef]) -> Result<Vec<UpdateResult>> {
    let pinned: Vec<&ActionRef> = all_refs.iter().filter(|r| is_sha(&r.ref_str)).collect();

    // For each unique owner/repo, find the latest tag — once per repo
    let mut latest_tags: HashMap<(String, String), Option<String>> = HashMap::new();
    for r in &pinned {
        let parts: Vec<&str> = r.action.splitn(3, '/').collect();
        if parts.len() < 2 {
            continue;
        }
        let repo_key = (parts[0].to_string(), parts[1].to_string());
        if !latest_tags.contains_key(&repo_key) {
            let tag = match list_tags_newest_first(&repo_key.0, &repo_key.1) {
                Ok(tags) => tags.into_iter().next(),
                Err(e) => {
                    eprintln!(
                        "warning: could not list tags for {}/{}: {e}",
                        repo_key.0, repo_key.1
                    );
                    None
                }
            };
            latest_tags.insert(repo_key, tag);
        }
    }

    // Resolve latest tag → SHA for each unique repo
    let mut tag_shas: HashMap<(String, String, String), Option<String>> = HashMap::new();
    for ((owner, repo), tag_opt) in &latest_tags {
        if let Some(tag) = tag_opt {
            let key = RefKey {
                owner: owner.clone(),
                repo: repo.clone(),
                ref_str: tag.clone(),
            };
            let sha = match resolve(&key) {
                Ok(r) => Some(r.sha),
                Err(e) => {
                    eprintln!("warning: could not resolve {owner}/{repo}@{tag}: {e}");
                    None
                }
            };
            tag_shas.insert((owner.clone(), repo.clone(), tag.clone()), sha);
        }
    }

    // Group pinned refs by file and apply replacements
    let mut by_file: HashMap<&Path, Vec<&ActionRef>> = HashMap::new();
    for r in &pinned {
        by_file.entry(r.file.as_path()).or_default().push(r);
    }

    let mut results = Vec::new();

    for (file, refs) in &by_file {
        let mut content = fs::read_to_string(file)?;
        let mut changed = false;

        for r in refs {
            let parts: Vec<&str> = r.action.splitn(3, '/').collect();
            if parts.len() < 2 {
                continue;
            }
            let owner = parts[0];
            let repo = parts[1];
            let repo_key = (owner.to_string(), repo.to_string());

            let latest_tag = match latest_tags.get(&repo_key).and_then(|t| t.as_ref()) {
                Some(t) => t.clone(),
                None => continue,
            };

            let new_sha = match tag_shas
                .get(&(owner.to_string(), repo.to_string(), latest_tag.clone()))
                .and_then(|s| s.as_ref())
            {
                Some(s) => s.clone(),
                None => continue,
            };

            if new_sha == r.ref_str {
                results.push(UpdateResult {
                    action: r.action.clone(),
                    old_sha: r.ref_str.clone(),
                    new_sha,
                    tag: latest_tag,
                    updated: false,
                });
                continue;
            }

            // Preserve the inline comment label if present; otherwise use the latest tag
            let comment_label = r
                .inline_comment
                .clone()
                .unwrap_or_else(|| latest_tag.clone());

            let pattern = format!(
                r"uses:\s+{action}@{sha}(\s*#[^\n]*)?",
                action = regex::escape(&r.action),
                sha = regex::escape(&r.ref_str),
            );
            let replacement = format!(
                "uses: {action}@{new_sha} # {label}",
                action = r.action,
                new_sha = new_sha,
                label = comment_label,
            );
            let re = Regex::new(&pattern).expect("invalid regex");
            let new_content = re.replace_all(&content, replacement.as_str());
            if new_content != content.as_str() {
                content = new_content.into_owned();
                changed = true;
                results.push(UpdateResult {
                    action: r.action.clone(),
                    old_sha: r.ref_str.clone(),
                    new_sha,
                    tag: latest_tag,
                    updated: true,
                });
            }
        }

        if changed {
            fs::write(file, &content)?;
        }
    }

    Ok(results)
}
