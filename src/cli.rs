use std::path::PathBuf;

use clap::{Parser, Subcommand};

const LONG_ABOUT: &str = "\
Pin GitHub Actions workflow steps to exact commit SHAs so that your CI supply \
chain is reproducible and immune to tag mutation or branch force-pushes.

Actions are resolved using your local `git` binary and its configured \
credentials (SSH keys, credential helpers, .netrc, etc.).  No GitHub token or \
API access is required.

WORKFLOW

  1. audit    — inspect what is and isn't pinned, without touching files
  2. migrate  — pin every unpinned action to the SHA its tag/branch resolves to
  3. update   — refresh already-pinned SHAs to the latest release of each action

Run `cargo gh-shaping <COMMAND> --help` for per-command options and examples.";

#[derive(Parser)]
#[command(
    name       = "gh-shaping",
    about      = "Pin GitHub Actions to exact commit SHAs",
    long_about = LONG_ABOUT,
    version,
    after_help = "Git credentials are read from your environment (SSH keys, \
credential helpers, ~/.netrc).  No GITHUB_TOKEN required.",
)]
pub struct Cli {
    /// Process workflow files one at a time instead of in parallel.
    ///
    /// By default each file is parsed on its own thread. Pass this flag to
    /// disable that behaviour, which can be useful when debugging or when
    /// running inside an environment that does not support threads.
    #[arg(long, global = true)]
    pub single_threaded: bool,

    /// Fetch branches in addition to tags.
    ///
    /// When set, `git ls-remote --heads` is called for each action and the
    /// resulting branches are made available alongside tags in the interactive
    /// TUI (press Tab to switch between the two views).  Has no effect on
    /// non-interactive runs.
    #[arg(long, global = true)]
    pub include_branches: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Inspect pinning status without modifying any files
    ///
    /// Reads every `uses:` directive across all workflow files, resolves each
    /// action ref to a commit SHA via `git ls-remote`, and prints a table
    /// showing whether each action is already pinned or still uses a mutable
    /// tag or branch ref.
    ///
    /// EXAMPLES
    ///
    /// Print the report to the terminal:
    ///
    ///   cargo gh-shaping audit
    ///
    /// Save the report to a file:
    ///
    ///   cargo gh-shaping audit --output pinning-report.txt
    ///
    /// Scan a non-default workflows directory:
    ///
    ///   cargo gh-shaping audit --workflows-dir path/to/.github/workflows
    Audit {
        /// Directory to scan for *.yml / *.yaml workflow files
        #[arg(long, default_value = ".github/workflows", value_name = "DIR")]
        workflows_dir: PathBuf,

        /// Write the report to FILE instead of stdout
        #[arg(long, value_name = "FILE")]
        output: Option<PathBuf>,

        /// Restrict to a single action (exact match, e.g. `actions/checkout`)
        #[arg(long, value_name = "ACTION")]
        action: Option<String>,
    },

    /// Pin every unpinned action to the SHA its tag or branch currently resolves to
    ///
    /// For each `uses: owner/repo@ref` directive that is not already a 40-character
    /// SHA, gh-shaping calls `git ls-remote` to resolve the ref and rewrites the
    /// line in-place, preserving the original ref as a trailing comment:
    ///
    ///   uses: actions/checkout@v4
    ///   → uses: actions/checkout@abcdef1234...5678 # v4
    ///
    /// Already-pinned lines are left untouched, making this command idempotent.
    /// Docker and local path actions (docker://, ./.github/...) are skipped.
    ///
    /// EXAMPLES
    ///
    /// Pin everything automatically:
    ///
    ///   cargo gh-shaping migrate
    ///
    /// Choose a specific version for each action via the interactive TUI:
    ///
    ///   cargo gh-shaping migrate --interactive
    ///
    /// Operate on a non-default workflows directory:
    ///
    ///   cargo gh-shaping migrate --workflows-dir path/to/.github/workflows
    Migrate {
        /// Directory to scan for *.yml / *.yaml workflow files
        #[arg(long, default_value = ".github/workflows", value_name = "DIR")]
        workflows_dir: PathBuf,

        /// Open an interactive TUI to choose which version to pin for each action.
        ///
        /// Displays all available tags alongside their commit SHAs, shows the
        /// surrounding workflow YAML for context, and lets you open a GitHub
        /// comparison between the current ref and the selected tag in your
        /// browser by pressing `c`.
        ///
        /// Navigation: [↑↓ / jk] move  [Enter] pin  [c] changelog  [s] skip  [q] quit
        #[arg(short, long)]
        interactive: bool,

        /// Restrict to a single action (exact match, e.g. `actions/checkout`)
        #[arg(long, value_name = "ACTION")]
        action: Option<String>,
    },

    /// Update already-pinned actions to the SHA of their latest release
    ///
    /// Scans for `uses:` lines pinned to a commit SHA, looks up the most recent
    /// release tag for each action via `git ls-remote --tags`, resolves that tag
    /// to a SHA, and rewrites the line if a newer release exists:
    ///
    ///   uses: actions/checkout@oldsha... # v4.1.0
    ///   → uses: actions/checkout@newsha... # v4.2.0
    ///
    /// Tag selection follows semver ordering when all tags parse as vMAJOR.MINOR.PATCH,
    /// and falls back to reverse-lexicographic order otherwise.  Actions with no
    /// published releases are skipped with a warning.
    ///
    /// EXAMPLES
    ///
    /// Update all pinned actions automatically:
    ///
    ///   cargo gh-shaping update
    ///
    /// Review each update interactively before applying:
    ///
    ///   cargo gh-shaping update --interactive
    ///
    /// Operate on a non-default workflows directory:
    ///
    ///   cargo gh-shaping update --workflows-dir path/to/.github/workflows
    Update {
        /// Directory to scan for *.yml / *.yaml workflow files
        #[arg(long, default_value = ".github/workflows", value_name = "DIR")]
        workflows_dir: PathBuf,

        /// Open an interactive TUI to review available versions before applying updates.
        ///
        /// Shows all tags with their commit SHAs, highlights the currently pinned
        /// SHA, displays the surrounding YAML for context, and lets you open a
        /// GitHub comparison between the current SHA and the selected tag in your
        /// browser by pressing `c`.
        ///
        /// Navigation: [↑↓ / jk] move  [Enter] pin  [c] changelog  [s] skip  [q] quit
        #[arg(short, long)]
        interactive: bool,

        /// Restrict to a single action (exact match, e.g. `actions/checkout`)
        #[arg(long, value_name = "ACTION")]
        action: Option<String>,
    },
}
