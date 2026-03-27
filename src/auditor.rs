use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::resolver::{RefKey, ResolvedSha};
use crate::workflow::ActionRef;

pub enum AuditStatus {
    AlreadyPinned,
    NeedsPinning,
    Error(String),
}

pub struct AuditRow {
    pub file: PathBuf,
    pub raw: String,
    pub sha: String,
    pub status: AuditStatus,
}

pub fn build_report(
    all_refs: &[ActionRef],
    resolution_map: &HashMap<RefKey, std::result::Result<ResolvedSha, String>>,
) -> Vec<AuditRow> {
    let mut rows = Vec::new();
    for r in all_refs {
        let key = match RefKey::from_action(&r.action, &r.ref_str) {
            Some(k) => k,
            None => {
                rows.push(AuditRow {
                    file: r.file.clone(),
                    raw: r.raw.clone(),
                    sha: String::new(),
                    status: AuditStatus::Error(format!("could not parse action: {}", r.raw)),
                });
                continue;
            }
        };

        match resolution_map.get(&key) {
            Some(Ok(resolved)) => {
                let status = if resolved.was_pinned {
                    AuditStatus::AlreadyPinned
                } else {
                    AuditStatus::NeedsPinning
                };
                rows.push(AuditRow {
                    file: r.file.clone(),
                    raw: r.raw.clone(),
                    sha: resolved.sha.clone(),
                    status,
                });
            }
            Some(Err(e)) => {
                rows.push(AuditRow {
                    file: r.file.clone(),
                    raw: r.raw.clone(),
                    sha: String::new(),
                    status: AuditStatus::Error(e.clone()),
                });
            }
            None => {
                rows.push(AuditRow {
                    file: r.file.clone(),
                    raw: r.raw.clone(),
                    sha: String::new(),
                    status: AuditStatus::Error("not resolved".to_string()),
                });
            }
        }
    }
    rows
}

pub fn write_report(rows: &[AuditRow], output: Option<&Path>) -> crate::error::Result<()> {
    let mut buf = String::new();

    // Column widths
    let file_w = rows
        .iter()
        .map(|r| r.file.display().to_string().len())
        .max()
        .unwrap_or(4)
        .max(4);
    let action_w = rows
        .iter()
        .map(|r| {
            r.raw
                .split_once('@')
                .map(|(a, _)| a.len())
                .unwrap_or(r.raw.len())
        })
        .max()
        .unwrap_or(6)
        .max(6);
    let ref_w = rows
        .iter()
        .map(|r| {
            r.raw
                .split_once('@')
                .map(|(_, v)| v.len())
                .unwrap_or(3)
        })
        .max()
        .unwrap_or(3)
        .max(3);

    // Header
    buf.push_str(&format!(
        "{:<file_w$}  {:<action_w$}  {:<ref_w$}  {:<42}  {}\n",
        "FILE", "ACTION", "REF", "SHA", "STATUS",
        file_w = file_w,
        action_w = action_w,
        ref_w = ref_w,
    ));
    buf.push_str(&format!(
        "{:-<file_w$}  {:-<action_w$}  {:-<ref_w$}  {:-<42}  {:-<14}\n",
        "", "", "", "", "",
        file_w = file_w,
        action_w = action_w,
        ref_w = ref_w,
    ));

    for row in rows {
        let (action, ref_str) = row
            .raw
            .split_once('@')
            .map(|(a, r)| (a.to_string(), r.to_string()))
            .unwrap_or_else(|| (row.raw.clone(), String::new()));

        let (sha_display, status_str) = match &row.status {
            AuditStatus::AlreadyPinned => (truncate(&row.sha, 40), "already-pinned".to_string()),
            AuditStatus::NeedsPinning => (truncate(&row.sha, 40), "needs-pinning".to_string()),
            AuditStatus::Error(e) => (String::new(), format!("error: {e}")),
        };

        buf.push_str(&format!(
            "{:<file_w$}  {:<action_w$}  {:<ref_w$}  {:<42}  {}\n",
            row.file.display(),
            action,
            ref_str,
            sha_display,
            status_str,
            file_w = file_w,
            action_w = action_w,
            ref_w = ref_w,
        ));
    }

    match output {
        Some(path) => fs::write(path, &buf)?,
        None => io::stdout().write_all(buf.as_bytes())?,
    }
    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max - 3])
    }
}
