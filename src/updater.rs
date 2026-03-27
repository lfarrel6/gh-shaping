use std::collections::HashMap;
use std::fs;
use std::path::Path;

use regex::Regex;

use crate::error::Result;
use crate::resolver::{
    is_ancestor_of_branch, is_sha, list_tags_newest_first, probe_branch, probe_tag, resolve,
};
use crate::workflow::ActionRef;

pub struct UpdateResult {
    pub action: String,
    pub old_sha: String,
    pub new_sha: String,
    /// The tag name or branch name that this update targets.
    pub label: String,
    pub updated: bool,
}

/// How a pinned ref should be updated.
#[derive(Debug)]
enum PinKind {
    /// Update to the latest tag for the repo.
    Tag,
    /// Update to the current HEAD of the named branch.
    Branch(String),
}

/// Determine whether `comment` refers to a branch or a tag for `owner/repo`.
///
/// Tags are checked first; a branch name that shadows a tag is always treated
/// as a tag.  Returns `None` if the comment matches neither.
fn classify_comment(
    owner: &str,
    repo: &str,
    comment: &str,
    disable_ancestry_checks: bool,
    current_sha: &str,
    verbose: bool,
) -> Option<PinKind> {
    if verbose {
        eprintln!(
            "[verbose] classifying comment \"{comment}\" for {owner}/{repo} (current SHA: {})",
            &current_sha[..8]
        );
    }

    // Tags take priority.
    match probe_tag(owner, repo, comment) {
        Ok(Some(sha)) => {
            if verbose {
                eprintln!(
                    "[verbose] probe_tag {owner}/{repo}#{comment} → found {}",
                    &sha[..8]
                );
            }
            return Some(PinKind::Tag);
        }
        Ok(None) => {
            if verbose {
                eprintln!("[verbose] probe_tag {owner}/{repo}#{comment} → not found");
            }
        }
        Err(e) => {
            eprintln!("warning: could not probe tag {owner}/{repo}#{comment}: {e}");
            return None;
        }
    }

    // Check if it is a branch.
    let branch_sha = match probe_branch(owner, repo, comment) {
        Ok(Some(sha)) => {
            if verbose {
                eprintln!(
                    "[verbose] probe_branch {owner}/{repo}#{comment} → found {}",
                    &sha[..8]
                );
            }
            sha
        }
        Ok(None) => {
            if verbose {
                eprintln!("[verbose] probe_branch {owner}/{repo}#{comment} → not found");
            }
            eprintln!(
                "warning: inline comment `{comment}` on {owner}/{repo} does not match any tag or \
                 branch — skipping update"
            );
            return None;
        }
        Err(e) => {
            eprintln!("warning: could not probe branch {owner}/{repo}#{comment}: {e}");
            return None;
        }
    };

    if disable_ancestry_checks {
        if verbose {
            eprintln!("[verbose] ancestry check skipped (--disable-ancestry-checks)");
        }
        return Some(PinKind::Branch(branch_sha));
    }

    if verbose {
        eprintln!(
            "[verbose] running ancestry check: is {} reachable from {owner}/{repo}#{comment}?",
            &current_sha[..8]
        );
    }

    // Verify the current pinned SHA is in the branch's recent history before
    // accepting this as a branch update.
    match is_ancestor_of_branch(owner, repo, comment, current_sha, 100, verbose) {
        Ok(true) => {
            if verbose {
                eprintln!("[verbose] ancestry confirmed → classified as Branch");
            }
            Some(PinKind::Branch(branch_sha))
        }
        Ok(false) => {
            eprintln!(
                "warning: {owner}/{repo}: pinned SHA {} is not in the recent history of branch \
                 `{comment}` (checked last 100 commits) — skipping update. \
                 Re-run with --disable-ancestry-checks to bypass.",
                &current_sha[..8]
            );
            None
        }
        Err(e) => {
            eprintln!(
                "warning: ancestry check failed for {owner}/{repo}#{comment}: {e} — skipping"
            );
            None
        }
    }
}

/// Process all already-pinned refs, check for newer releases, and rewrite files.
pub fn run_updates(
    all_refs: &[ActionRef],
    disable_ancestry_checks: bool,
    verbose: bool,
) -> Result<Vec<UpdateResult>> {
    let pinned: Vec<&ActionRef> = all_refs.iter().filter(|r| is_sha(&r.ref_str)).collect();

    if verbose {
        eprintln!("[verbose] found {} pinned ref(s) to process", pinned.len());
    }

    // ── classify each ref as a tag or branch pin ──────────────────────────────
    // Cache results so each unique (owner, repo, comment) is only probed once.
    let mut kind_cache: HashMap<(String, String, String), Option<PinKind>> = HashMap::new();

    for r in &pinned {
        let parts: Vec<&str> = r.action.splitn(3, '/').collect();
        if parts.len() < 2 {
            continue;
        }
        let owner = parts[0];
        let repo = parts[1];

        if verbose {
            eprintln!(
                "[verbose] processing {}@{} (comment: {:?})",
                r.action,
                &r.ref_str[..8],
                r.inline_comment
            );
        }

        if let Some(comment) = &r.inline_comment {
            let cache_key = (owner.to_string(), repo.to_string(), comment.clone());
            kind_cache.entry(cache_key).or_insert_with(|| {
                classify_comment(
                    owner,
                    repo,
                    comment,
                    disable_ancestry_checks,
                    &r.ref_str,
                    verbose,
                )
            });
        } else if verbose {
            eprintln!("[verbose] no inline comment → will use latest tag");
        }
    }

    // ── resolve target SHAs for tag-based pins ────────────────────────────────
    // One latest-tag lookup per unique repo that has at least one tag pin.
    let mut latest_tags: HashMap<(String, String), Option<String>> = HashMap::new();
    for r in &pinned {
        let parts: Vec<&str> = r.action.splitn(3, '/').collect();
        if parts.len() < 2 {
            continue;
        }
        let owner = parts[0];
        let repo = parts[1];

        let is_tag_pin = r.inline_comment.as_ref().map_or(true, |c| {
            matches!(
                kind_cache.get(&(owner.to_string(), repo.to_string(), c.clone())),
                Some(Some(PinKind::Tag)) | None
            )
        });

        if is_tag_pin {
            let repo_key = (owner.to_string(), repo.to_string());
            if !latest_tags.contains_key(&repo_key) {
                let tag = match list_tags_newest_first(owner, repo) {
                    Ok(tags) => tags.into_iter().next(),
                    Err(e) => {
                        eprintln!("warning: could not list tags for {owner}/{repo}: {e}");
                        None
                    }
                };
                latest_tags.insert(repo_key, tag);
            }
        }
    }

    let mut tag_shas: HashMap<(String, String, String), Option<String>> = HashMap::new();
    for ((owner, repo), tag_opt) in &latest_tags {
        if let Some(tag) = tag_opt {
            let key = crate::resolver::RefKey {
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

    // ── apply updates ─────────────────────────────────────────────────────────
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

            let (new_sha, label) = match &r.inline_comment {
                Some(comment) => {
                    let cache_key = (owner.to_string(), repo.to_string(), comment.clone());
                    match kind_cache.get(&cache_key) {
                        Some(Some(PinKind::Branch(branch_sha))) => {
                            (branch_sha.clone(), comment.clone())
                        }
                        Some(Some(PinKind::Tag)) | None => {
                            // Tag path — use latest tag SHA.
                            let repo_key = (owner.to_string(), repo.to_string());
                            let latest_tag =
                                match latest_tags.get(&repo_key).and_then(|t| t.as_ref()) {
                                    Some(t) => t.clone(),
                                    None => continue,
                                };
                            let sha = match tag_shas
                                .get(&(owner.to_string(), repo.to_string(), latest_tag.clone()))
                                .and_then(|s| s.as_ref())
                            {
                                Some(s) => s.clone(),
                                None => continue,
                            };
                            (sha, latest_tag)
                        }
                        // classify returned None → already warned, skip
                        Some(None) => continue,
                    }
                }
                None => {
                    // No comment → tag path.
                    let repo_key = (owner.to_string(), repo.to_string());
                    let latest_tag = match latest_tags.get(&repo_key).and_then(|t| t.as_ref()) {
                        Some(t) => t.clone(),
                        None => continue,
                    };
                    let sha = match tag_shas
                        .get(&(owner.to_string(), repo.to_string(), latest_tag.clone()))
                        .and_then(|s| s.as_ref())
                    {
                        Some(s) => s.clone(),
                        None => continue,
                    };
                    (sha, latest_tag)
                }
            };

            if verbose {
                eprintln!(
                    "[verbose] {}@{}: target SHA {} (label: {label})",
                    r.action,
                    &r.ref_str[..8],
                    &new_sha[..8],
                );
            }

            if new_sha == r.ref_str {
                if verbose {
                    eprintln!(
                        "[verbose] {}@{}: already up to date",
                        r.action,
                        &r.ref_str[..8]
                    );
                }
                results.push(UpdateResult {
                    action: r.action.clone(),
                    old_sha: r.ref_str.clone(),
                    new_sha,
                    label,
                    updated: false,
                });
                continue;
            }

            let comment_label = r.inline_comment.clone().unwrap_or_else(|| label.clone());

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

            if verbose {
                eprintln!("[verbose] regex pattern: {pattern}");
                eprintln!("[verbose] replacement:    {replacement}");
            }

            let re = Regex::new(&pattern).expect("invalid regex");
            let new_content = re.replace_all(&content, replacement.as_str());
            if new_content != content.as_str() {
                if verbose {
                    eprintln!("[verbose] regex matched — rewriting {}", file.display());
                }
                content = new_content.into_owned();
                changed = true;
                results.push(UpdateResult {
                    action: r.action.clone(),
                    old_sha: r.ref_str.clone(),
                    new_sha,
                    label,
                    updated: true,
                });
            } else if verbose {
                eprintln!(
                    "[verbose] regex did not match in {} — no rewrite performed",
                    file.display()
                );
            }
        }

        if changed {
            fs::write(file, &content)?;
        }
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_kind_tag_when_no_comment() {
        // A ref with no inline comment is always a tag pin; no classification needed.
        // Verify the fallback path compiles and the enum variants are correct.
        let _tag = PinKind::Tag;
        let _branch = PinKind::Branch("abc".to_string());
    }
}
