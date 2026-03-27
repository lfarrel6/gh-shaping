# gh-shaping

A cargo plugin for pinning GitHub Actions workflow steps to exact commit SHAs.

Mutable refs like `actions/checkout@v4` can silently change what code runs in
your CI pipeline. Pinning to a SHA makes the dependency immutable — the only
way it changes is if you explicitly update it.

## How it works

Actions are resolved using your local `git` binary and its configured
credentials. There is no GitHub token requirement and no API rate limiting to
worry about — `git ls-remote` does the work.

Pinned lines preserve the original ref as a comment so the file stays readable:

```yaml
# before
- uses: actions/checkout@v4

# after
- uses: actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683 # v4
```

## Prerequisites

- Rust toolchain (edition 2024)
- `git` in your `PATH` with credentials configured for GitHub (SSH keys,
  credential helpers, `~/.netrc`, etc.)

## Installation

```sh
cargo install --path .
```

After installation the plugin is available as `cargo gh-shaping`.

## Commands

### audit

Inspect the pinning status of your workflows without modifying any files.

```sh
cargo gh-shaping audit
cargo gh-shaping audit --output report.txt
```

Prints a table of every `uses:` directive with its resolved SHA and whether it
needs pinning.

### migrate

Resolve every unpinned `uses:` ref to a SHA and rewrite the workflow files
in-place. Already-pinned lines are left untouched, so the command is safe to
re-run.

```sh
cargo gh-shaping migrate
```

Pass `--interactive` (or `-i`) to open a TUI for each action where you can
browse all available tags, see their commit SHAs, and compare against the
currently referenced version before committing to a choice.

```sh
cargo gh-shaping migrate --interactive
```

### update

Find actions that are already pinned to a SHA and check whether a newer release
exists. If one does, the SHA and comment label are updated in-place.

```sh
cargo gh-shaping update
cargo gh-shaping update --interactive
```

Tags are sorted by semver when all tags follow `vMAJOR.MINOR.PATCH`, and by
reverse-lexicographic order otherwise.

## Interactive mode

The interactive TUI is available for both `migrate` and `update`. It shows:

- All available tags for the action being processed, newest first, alongside
  their commit SHAs
- The surrounding lines of the workflow file for context, with the relevant
  `uses:` line highlighted
- A comparison link (`c`) that opens the GitHub compare view between the
  current ref and the selected tag in your browser

```
 gh-shaping — interactive migrate
  File:    .github/workflows/ci.yml
  Action:  actions/checkout
  Current: v3
┌─ Versions ──────────────────────┬─ Workflow Context ──────────────────┐
│  ► v4.2.0  11bd71901bbe         │    - name: Checkout                 │
│    v4.1.0  7a4ea8430519         │  ► uses: actions/checkout@v3        │
│    v4.0.0  85e7b9dc8a           │    with:                            │
│    v3.6.0  df84f44ade1          │      fetch-depth: 0                 │
└─────────────────────────────────┴─────────────────────────────────────┘
  [↑↓ / jk] navigate  [Enter] pin  [c] changelog  [s] skip  [q] quit
```

## Recommended workflow

```sh
# 1. See what needs pinning
cargo gh-shaping audit

# 2. Pin everything (or use -i to review each action)
cargo gh-shaping migrate

# 3. Commit the result
git add .github/workflows
git commit -m "chore: pin actions to commit SHAs"

# 4. Keep pins up to date over time
cargo gh-shaping update
```

## Skipped actions

The following `uses:` forms are ignored and left unchanged:

- Local path actions: `./.github/actions/...`
- Docker image actions: `docker://...`
- Any `uses:` line already pinned to a 40-character SHA (migrate only)

## License

MIT
