use std::collections::HashMap;
use std::process::Command;

use crate::error::{Error, Result};
use crate::orchestrator::Strategy;

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct RefKey {
    pub owner: String,
    pub repo: String,
    pub ref_str: String,
}

impl RefKey {
    /// Parse "owner/repo" or "owner/repo/subpath" into a RefKey.
    pub fn from_action(action: &str, ref_str: &str) -> Option<Self> {
        let parts: Vec<&str> = action.splitn(3, '/').collect();
        if parts.len() < 2 {
            return None;
        }
        Some(RefKey {
            owner: parts[0].to_string(),
            repo: parts[1].to_string(),
            ref_str: ref_str.to_string(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedSha {
    pub sha: String,
    /// True if ref_str was already a full 40-char SHA
    pub was_pinned: bool,
}

pub fn is_sha(s: &str) -> bool {
    s.len() == 40 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Resolve a ref to a commit SHA using `git ls-remote`.
pub fn resolve(key: &RefKey) -> Result<ResolvedSha> {
    if is_sha(&key.ref_str) {
        return Ok(ResolvedSha {
            sha: key.ref_str.clone(),
            was_pinned: true,
        });
    }

    let url = format!("https://github.com/{}/{}", key.owner, key.repo);

    // Query both the tag ref and its peeled form (^{}) in one call.
    // The peeled form resolves annotated tags to the underlying commit.
    let tag_ref = format!("refs/tags/{}", key.ref_str);
    let tag_ref_peeled = format!("refs/tags/{}^{{}}", key.ref_str);

    let output = Command::new("git")
        .args(["ls-remote", &url, &tag_ref, &tag_ref_peeled])
        .output()
        .map_err(|e| Error::Git {
            owner: key.owner.clone(),
            repo: key.repo.clone(),
            message: format!("failed to run git ls-remote: {e}"),
        })?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut tag_sha: Option<String> = None;
        let mut peeled_sha: Option<String> = None;

        for line in stdout.lines() {
            if let Some((sha, refname)) = line.split_once('\t') {
                if refname.ends_with("^{}") {
                    peeled_sha = Some(sha.to_string());
                } else {
                    tag_sha = Some(sha.to_string());
                }
            }
        }

        // Prefer the peeled SHA (commit behind an annotated tag) over the tag object SHA
        if let Some(sha) = peeled_sha.or(tag_sha) {
            return Ok(ResolvedSha {
                sha,
                was_pinned: false,
            });
        }
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.trim().is_empty() {
            return Err(Error::Git {
                owner: key.owner.clone(),
                repo: key.repo.clone(),
                message: stderr.trim().to_string(),
            });
        }
    }

    // Fall back to branch lookup
    let head_ref = format!("refs/heads/{}", key.ref_str);
    let output = Command::new("git")
        .args(["ls-remote", &url, &head_ref])
        .output()
        .map_err(|e| Error::Git {
            owner: key.owner.clone(),
            repo: key.repo.clone(),
            message: format!("failed to run git ls-remote: {e}"),
        })?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if let Some((sha, _)) = line.split_once('\t') {
                return Ok(ResolvedSha {
                    sha: sha.to_string(),
                    was_pinned: false,
                });
            }
        }
    }

    Err(Error::RefNotFound {
        owner: key.owner.clone(),
        repo: key.repo.clone(),
        ref_str: key.ref_str.clone(),
    })
}

/// Resolve all unique RefKeys using the provided strategy.
///
/// Each key is an independent unit of work (one `git ls-remote` call), so
/// they map naturally onto the strategy's worker slots.  Errors are collected
/// as strings rather than propagated so that one failed lookup does not abort
/// the rest.
pub fn resolve_all(
    keys: impl IntoIterator<Item = RefKey>,
    strategy: &Strategy,
) -> HashMap<RefKey, std::result::Result<ResolvedSha, String>> {
    strategy
        .run(keys.into_iter().collect(), &|key| {
            let result = resolve(&key).map_err(|e| e.to_string());
            (key, result)
        })
        .into_iter()
        .collect()
}

/// Fetch all tags for a GitHub repo with their resolved commit SHAs, sorted newest-first.
/// A single `git ls-remote --tags` call captures both lightweight and annotated tag SHAs.
pub fn list_tags_with_shas(owner: &str, repo: &str) -> Result<Vec<(String, String)>> {
    let url = format!("https://github.com/{owner}/{repo}");
    // Without --refs we get both refs/tags/vX and refs/tags/vX^{} (peeled) entries.
    // The peeled entry gives the commit SHA for annotated tags.
    let output = Command::new("git")
        .args(["ls-remote", "--tags", &url])
        .output()
        .map_err(|e| Error::Git {
            owner: owner.to_string(),
            repo: repo.to_string(),
            message: format!("failed to run git ls-remote: {e}"),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::Git {
            owner: owner.to_string(),
            repo: repo.to_string(),
            message: stderr.trim().to_string(),
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut tag_shas: HashMap<String, String> = HashMap::new();
    let mut peeled_shas: HashMap<String, String> = HashMap::new();
    let mut order: Vec<String> = Vec::new();

    for line in stdout.lines() {
        if let Some((sha, refname)) = line.split_once('\t') {
            if let Some(rest) = refname.strip_prefix("refs/tags/") {
                if let Some(base) = rest.strip_suffix("^{}") {
                    peeled_shas.insert(base.to_string(), sha.to_string());
                } else {
                    if !tag_shas.contains_key(rest) {
                        order.push(rest.to_string());
                    }
                    tag_shas.insert(rest.to_string(), sha.to_string());
                }
            }
        }
    }

    let mut result: Vec<(String, String)> = order
        .into_iter()
        .map(|tag| {
            let sha = peeled_shas
                .get(&tag)
                .cloned()
                .unwrap_or_else(|| tag_shas[&tag].clone());
            (tag, sha)
        })
        .collect();

    sort_tag_sha_pairs(&mut result);
    Ok(result)
}

/// Fetch all branches for a GitHub repo with their current commit SHAs,
/// sorted alphabetically by branch name.
pub fn list_branches_with_shas(owner: &str, repo: &str) -> Result<Vec<(String, String)>> {
    let url = format!("https://github.com/{owner}/{repo}");
    let output = Command::new("git")
        .args(["ls-remote", "--heads", &url])
        .output()
        .map_err(|e| Error::Git {
            owner: owner.to_string(),
            repo: repo.to_string(),
            message: format!("failed to run git ls-remote: {e}"),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::Git {
            owner: owner.to_string(),
            repo: repo.to_string(),
            message: stderr.trim().to_string(),
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut branches: Vec<(String, String)> =
        stdout.lines().filter_map(parse_branch_line).collect();
    branches.sort_by(|(a, _), (b, _)| a.cmp(b));
    Ok(branches)
}

fn parse_branch_line(line: &str) -> Option<(String, String)> {
    let (sha, refname) = line.split_once('\t')?;
    let branch = refname.strip_prefix("refs/heads/")?;
    Some((branch.to_string(), sha.to_string()))
}

fn sort_tag_sha_pairs(tags: &mut Vec<(String, String)>) {
    let all_semver = tags.iter().all(|(t, _)| parse_semver(t).is_some());
    if all_semver && !tags.is_empty() {
        tags.sort_by(|(a, _), (b, _)| {
            let av = parse_semver(a).unwrap();
            let bv = parse_semver(b).unwrap();
            bv.0.cmp(&av.0).then(bv.1.cmp(&av.1)).then(bv.2.cmp(&av.2))
        });
    } else {
        tags.sort_by(|(a, _), (b, _)| b.cmp(a));
    }
}

/// List all tags for a GitHub repo via `git ls-remote --tags --refs`, sorted newest-first.
/// Tries semver ordering (vX.Y.Z or X.Y.Z) and falls back to lexicographic.
pub fn list_tags_newest_first(owner: &str, repo: &str) -> Result<Vec<String>> {
    let url = format!("https://github.com/{owner}/{repo}");
    let output = Command::new("git")
        .args(["ls-remote", "--tags", "--refs", &url])
        .output()
        .map_err(|e| Error::Git {
            owner: owner.to_string(),
            repo: repo.to_string(),
            message: format!("failed to run git ls-remote: {e}"),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::Git {
            owner: owner.to_string(),
            repo: repo.to_string(),
            message: stderr.trim().to_string(),
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut tags: Vec<String> = stdout
        .lines()
        .filter_map(|line| {
            let (_, refname) = line.split_once('\t')?;
            refname.strip_prefix("refs/tags/").map(|t| t.to_string())
        })
        .collect();

    // Try semver sort: parse vMAJOR.MINOR.PATCH or MAJOR.MINOR.PATCH
    let semver_tags: Vec<(u64, u64, u64, &str)> = tags
        .iter()
        .filter_map(|t| parse_semver(t).map(|v| (v.0, v.1, v.2, t.as_str())))
        .collect();

    if !semver_tags.is_empty() && semver_tags.len() == tags.len() {
        // All tags are semver — sort them
        let mut sorted = semver_tags;
        sorted.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)).then(b.2.cmp(&a.2)));
        return Ok(sorted
            .into_iter()
            .map(|(_, _, _, t)| t.to_string())
            .collect());
    }

    // Fall back to reverse lexicographic
    tags.sort_by(|a, b| b.cmp(a));
    Ok(tags)
}

fn parse_semver(s: &str) -> Option<(u64, u64, u64)> {
    let s = s.strip_prefix('v').unwrap_or(s);
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() < 3 {
        return None;
    }
    let major = parts[0].parse().ok()?;
    let minor = parts[1].parse().ok()?;
    // Allow trailing metadata like "1.2.3-rc1" — just parse the numeric prefix
    let patch_str = parts[2].split('-').next().unwrap_or(parts[2]);
    let patch = patch_str.parse().ok()?;
    Some((major, minor, patch))
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_branch_line_valid() {
        let sha = "abc123def456abc123def456abc123def456abc1";
        let line = format!("{sha}\trefs/heads/main");
        assert_eq!(
            parse_branch_line(&line),
            Some(("main".to_string(), sha.to_string()))
        );
    }

    #[test]
    fn parse_branch_line_nested_name() {
        let sha = "abc123def456abc123def456abc123def456abc1";
        let line = format!("{sha}\trefs/heads/feat/my-feature");
        assert_eq!(
            parse_branch_line(&line),
            Some(("feat/my-feature".to_string(), sha.to_string()))
        );
    }

    #[test]
    fn parse_branch_line_tag_not_matched() {
        let sha = "abc123def456abc123def456abc123def456abc1";
        let line = format!("{sha}\trefs/tags/v1.0.0");
        assert_eq!(parse_branch_line(&line), None);
    }

    #[test]
    fn parse_branch_line_empty() {
        assert_eq!(parse_branch_line(""), None);
    }
}
