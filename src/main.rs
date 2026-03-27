mod auditor;
mod cli;
mod error;
mod interactive;
mod orchestrator;
mod pinner;
mod resolver;
mod updater;
mod workflow;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use clap::Parser;

use cli::{Cli, Command};
use error::Result;
use orchestrator::Strategy;
use resolver::{RefKey, is_sha, resolve_all};
use workflow::ActionRef;

fn main() -> Result<()> {
    // When invoked as a cargo subcommand (`cargo gh-shaping <cmd>`), cargo
    // inserts "gh-shaping" as the first argument.  Strip it so that clap sees
    // the same shape whether the binary is called directly or via cargo.
    let args: Vec<_> = std::env::args_os()
        .enumerate()
        .filter(|(i, arg)| !(*i == 1 && arg == "gh-shaping"))
        .map(|(_, arg)| arg)
        .collect();

    let cli = Cli::parse_from(args);

    let strategy = if cli.single_threaded {
        Strategy::Sequential
    } else {
        Strategy::Parallel
    };

    match cli.command {
        Command::Migrate {
            workflows_dir,
            interactive,
            action,
        } => {
            if interactive {
                run_migrate_interactive(&workflows_dir, action.as_deref(), &strategy)
            } else {
                run_migrate(&workflows_dir, action.as_deref(), &strategy)
            }
        }
        Command::Audit {
            workflows_dir,
            output,
            action,
        } => run_audit(
            &workflows_dir,
            output.as_deref(),
            action.as_deref(),
            &strategy,
        ),
        Command::Update {
            workflows_dir,
            interactive,
            action,
        } => {
            if interactive {
                run_update_interactive(&workflows_dir, action.as_deref(), &strategy)
            } else {
                run_update(&workflows_dir, action.as_deref(), &strategy)
            }
        }
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Retain only refs whose action field exactly matches `filter`, or all refs
/// when `filter` is `None`.
fn filter_by_action<'a>(refs: &'a [ActionRef], filter: Option<&str>) -> Vec<&'a ActionRef> {
    refs.iter()
        .filter(|r| filter.is_none_or(|f| r.action == f))
        .collect()
}

/// Parse every workflow file using the provided orchestrator and return all
/// collected action refs.  Each file is treated as an independent unit of work;
/// errors are printed as warnings and result in an empty contribution from that
/// file rather than aborting the run.
fn collect_refs(files: &[PathBuf], strategy: &Strategy) -> Vec<ActionRef> {
    strategy
        .run(
            files.to_vec(),
            &|file| match workflow::extract_action_refs(&file) {
                Ok(refs) => refs,
                Err(e) => {
                    eprintln!("warning: skipping {}: {e}", file.display());
                    Vec::new()
                }
            },
        )
        .into_iter()
        .flatten()
        .collect()
}

// ── migrate ───────────────────────────────────────────────────────────────────

fn run_migrate(workflows_dir: &Path, action: Option<&str>, strategy: &Strategy) -> Result<()> {
    let files = workflow::find_workflow_files(workflows_dir)?;
    if files.is_empty() {
        eprintln!("no workflow files found in {}", workflows_dir.display());
        return Ok(());
    }

    let all_refs = collect_refs(&files, strategy);
    let all_refs: Vec<ActionRef> = filter_by_action(&all_refs, action)
        .into_iter()
        .cloned()
        .collect();

    let unique_keys: HashSet<RefKey> = all_refs
        .iter()
        .filter(|r| !is_sha(&r.ref_str))
        .filter_map(|r| RefKey::from_action(&r.action, &r.ref_str))
        .collect();

    println!("resolving {} unique action ref(s)...", unique_keys.len());
    let resolution_map = resolve_all(unique_keys, strategy);

    let mut total = 0;
    for file in &files {
        let file_refs: Vec<_> = all_refs
            .iter()
            .filter(|r| r.file == *file)
            .cloned()
            .collect();
        match pinner::pin_workflow_file(file, &resolution_map, &file_refs) {
            Ok(n) => {
                if n > 0 {
                    println!("  {} — pinned {n} action(s)", file.display());
                    total += n;
                }
            }
            Err(e) => eprintln!("error writing {}: {e}", file.display()),
        }
    }

    if total == 0 {
        println!("all actions already pinned — nothing to do");
    } else {
        println!("done — pinned {total} action(s)");
    }
    Ok(())
}

fn run_migrate_interactive(
    workflows_dir: &Path,
    action: Option<&str>,
    strategy: &Strategy,
) -> Result<()> {
    let files = workflow::find_workflow_files(workflows_dir)?;
    if files.is_empty() {
        eprintln!("no workflow files found in {}", workflows_dir.display());
        return Ok(());
    }

    let all_refs = collect_refs(&files, strategy);
    let all_refs: Vec<ActionRef> = filter_by_action(&all_refs, action)
        .into_iter()
        .cloned()
        .collect();

    let mut seen: HashSet<(String, String)> = HashSet::new();
    let unique_unpinned: Vec<&ActionRef> = all_refs
        .iter()
        .filter(|r| !is_sha(&r.ref_str))
        .filter(|r| seen.insert((r.action.clone(), r.ref_str.clone())))
        .collect();

    if unique_unpinned.is_empty() {
        println!("all actions already pinned — nothing to do");
        return Ok(());
    }

    for r in unique_unpinned {
        let key = match RefKey::from_action(&r.action, &r.ref_str) {
            Some(k) => k,
            None => continue,
        };

        eprint!("fetching tags for {}... ", r.action);
        let tags = match resolver::list_tags_with_shas(&key.owner, &key.repo) {
            Ok(t) => {
                eprintln!("{} tag(s)", t.len());
                t
            }
            Err(e) => {
                eprintln!("error: {e}");
                continue;
            }
        };

        let (ctx_lines, ctx_highlight) = workflow::extract_context(&r.file, &r.raw, 3);

        let choice = interactive::pick_version(
            "migrate",
            &r.file.display().to_string(),
            &r.action,
            &r.ref_str,
            &interactive::TagEntry::from_pairs(tags),
            ctx_lines,
            ctx_highlight,
            &key.owner,
            &key.repo,
        )
        .map_err(error::Error::Io)?;

        match choice {
            interactive::Choice::Pin { sha, tag } => {
                let mut pinned = 0;
                for file in &files {
                    let has_ref = all_refs.iter().any(|ar| {
                        ar.file == *file && ar.action == r.action && ar.ref_str == r.ref_str
                    });
                    if has_ref {
                        match pinner::rewrite_uses(file, &r.action, &r.ref_str, &sha, &tag) {
                            Ok(true) => {
                                pinned += 1;
                            }
                            Ok(false) => {}
                            Err(e) => eprintln!("error writing {}: {e}", file.display()),
                        }
                    }
                }
                println!(
                    "pinned {}@{} → {} ({}) in {pinned} file(s)",
                    r.action,
                    r.ref_str,
                    &sha[..8],
                    tag
                );
            }
            interactive::Choice::Skip => println!("skipped {}", r.action),
            interactive::Choice::Quit => {
                println!("quit");
                return Ok(());
            }
        }
    }
    Ok(())
}

// ── audit ─────────────────────────────────────────────────────────────────────

fn run_audit(
    workflows_dir: &Path,
    output: Option<&Path>,
    action: Option<&str>,
    strategy: &Strategy,
) -> Result<()> {
    let files = workflow::find_workflow_files(workflows_dir)?;
    if files.is_empty() {
        eprintln!("no workflow files found in {}", workflows_dir.display());
        return Ok(());
    }

    let all_refs = collect_refs(&files, strategy);
    let all_refs: Vec<ActionRef> = filter_by_action(&all_refs, action)
        .into_iter()
        .cloned()
        .collect();

    let unique_keys: HashSet<RefKey> = all_refs
        .iter()
        .filter_map(|r| RefKey::from_action(&r.action, &r.ref_str))
        .collect();

    eprintln!("resolving {} unique action ref(s)...", unique_keys.len());
    let resolution_map = resolve_all(unique_keys, strategy);

    let rows = auditor::build_report(&all_refs, &resolution_map);
    auditor::write_report(&rows, output)?;
    Ok(())
}

// ── update ────────────────────────────────────────────────────────────────────

fn run_update(workflows_dir: &Path, action: Option<&str>, strategy: &Strategy) -> Result<()> {
    let files = workflow::find_workflow_files(workflows_dir)?;
    if files.is_empty() {
        eprintln!("no workflow files found in {}", workflows_dir.display());
        return Ok(());
    }

    let all_refs = collect_refs(&files, strategy);
    let all_refs: Vec<ActionRef> = filter_by_action(&all_refs, action)
        .into_iter()
        .cloned()
        .collect();

    let pinned_count = all_refs.iter().filter(|r| is_sha(&r.ref_str)).count();
    if pinned_count == 0 {
        println!("no pinned actions found — run `migrate` first");
        return Ok(());
    }

    println!("checking {pinned_count} pinned action(s) for updates...");
    let results = updater::run_updates(&all_refs)?;

    let updated: Vec<_> = results.iter().filter(|r| r.updated).collect();
    let current: Vec<_> = results.iter().filter(|r| !r.updated).collect();

    for r in &updated {
        println!(
            "  updated  {}  {} → {} ({})",
            r.action,
            &r.old_sha[..8],
            &r.new_sha[..8],
            r.tag,
        );
    }
    for r in &current {
        println!("  current  {}  {} ({})", r.action, &r.old_sha[..8], r.tag);
    }

    if updated.is_empty() {
        println!("all pinned actions are up to date");
    } else {
        println!("updated {} action(s)", updated.len());
    }
    Ok(())
}

fn run_update_interactive(
    workflows_dir: &Path,
    action: Option<&str>,
    strategy: &Strategy,
) -> Result<()> {
    let files = workflow::find_workflow_files(workflows_dir)?;
    if files.is_empty() {
        eprintln!("no workflow files found in {}", workflows_dir.display());
        return Ok(());
    }

    let all_refs = collect_refs(&files, strategy);
    let all_refs: Vec<ActionRef> = filter_by_action(&all_refs, action)
        .into_iter()
        .cloned()
        .collect();

    let mut seen: HashSet<(String, String)> = HashSet::new();
    let unique_pinned: Vec<&ActionRef> = all_refs
        .iter()
        .filter(|r| is_sha(&r.ref_str))
        .filter(|r| seen.insert((r.action.clone(), r.ref_str.clone())))
        .collect();

    if unique_pinned.is_empty() {
        println!("no pinned actions found — run `migrate` first");
        return Ok(());
    }

    for r in unique_pinned {
        let key = match RefKey::from_action(&r.action, &r.ref_str) {
            Some(k) => k,
            None => continue,
        };

        eprint!("fetching tags for {}... ", r.action);
        let tags = match resolver::list_tags_with_shas(&key.owner, &key.repo) {
            Ok(t) => {
                eprintln!("{} tag(s)", t.len());
                t
            }
            Err(e) => {
                eprintln!("error: {e}");
                continue;
            }
        };

        let current_display = match &r.inline_comment {
            Some(c) => format!("{} ({})", &r.ref_str[..8], c),
            None => r.ref_str[..8].to_string(),
        };

        let (ctx_lines, ctx_highlight) = workflow::extract_context(&r.file, &r.raw, 3);

        let choice = interactive::pick_version(
            "update",
            &r.file.display().to_string(),
            &r.action,
            &current_display,
            &interactive::TagEntry::from_pairs(tags),
            ctx_lines,
            ctx_highlight,
            &key.owner,
            &key.repo,
        )
        .map_err(error::Error::Io)?;

        match choice {
            interactive::Choice::Pin { sha, tag } => {
                if sha == r.ref_str {
                    println!("already up to date: {} ({})", r.action, tag);
                    continue;
                }
                let label = r.inline_comment.clone().unwrap_or_else(|| tag.clone());
                let mut updated = 0;
                for file in &files {
                    let has_ref = all_refs.iter().any(|ar| {
                        ar.file == *file && ar.action == r.action && ar.ref_str == r.ref_str
                    });
                    if has_ref {
                        match pinner::rewrite_uses(file, &r.action, &r.ref_str, &sha, &label) {
                            Ok(true) => {
                                updated += 1;
                            }
                            Ok(false) => {}
                            Err(e) => eprintln!("error writing {}: {e}", file.display()),
                        }
                    }
                }
                println!(
                    "updated {}  {} → {} ({}) in {updated} file(s)",
                    r.action,
                    &r.ref_str[..8],
                    &sha[..8],
                    tag,
                );
            }
            interactive::Choice::Skip => println!("skipped {}", r.action),
            interactive::Choice::Quit => {
                println!("quit");
                return Ok(());
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use workflow::ActionRef;

    use super::*;

    fn make_ref(action: &str) -> ActionRef {
        ActionRef {
            file: PathBuf::from("workflow.yml"),
            action: action.to_string(),
            ref_str: "v1".to_string(),
            raw: format!("{action}@v1"),
            inline_comment: None,
        }
    }

    #[test]
    fn filter_by_action_none_returns_all() {
        let refs = vec![make_ref("actions/checkout"), make_ref("actions/setup-node")];
        assert_eq!(filter_by_action(&refs, None).len(), 2);
    }

    #[test]
    fn filter_by_action_exact_match() {
        let refs = vec![make_ref("actions/checkout"), make_ref("actions/setup-node")];
        let filtered = filter_by_action(&refs, Some("actions/checkout"));
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].action, "actions/checkout");
    }

    #[test]
    fn filter_by_action_no_match_returns_empty() {
        let refs = vec![make_ref("actions/checkout"), make_ref("actions/setup-node")];
        assert_eq!(filter_by_action(&refs, Some("actions/cache")).len(), 0);
    }

    #[test]
    fn filter_by_action_no_partial_match() {
        let refs = vec![make_ref("actions/checkout")];
        // "actions" alone must not match "actions/checkout"
        assert_eq!(filter_by_action(&refs, Some("actions")).len(), 0);
    }
}
