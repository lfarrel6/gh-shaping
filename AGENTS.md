# Agent Guidelines

## Commands

```bash
cargo check          # type-check the codebase
cargo fmt --check    # verify formatting
cargo fmt            # apply formatting
cargo test           # run all tests
cargo build          # debug build
cargo build --release
```

## Before accepting any change

1. Run `cargo fmt` to format all modified files.
2. Run `cargo check` to confirm the codebase compiles cleanly.

## Testing

New behaviour must be accompanied by unit tests.  Place tests in a `#[cfg(test)]` module at the bottom of the relevant source file.  Prefer testing pure logic directly; avoid spawning external processes in tests.

## Architecture

| File | Responsibility |
|------|----------------|
| `src/main.rs` | Entry point, CLI dispatch, `collect_refs` helper |
| `src/cli.rs` | `clap` argument definitions |
| `src/workflow.rs` | YAML parsing, action-ref extraction, context extraction |
| `src/resolver.rs` | SHA resolution via `git ls-remote`, tag listing, semver sorting |
| `src/pinner.rs` | In-place regex rewriting of workflow files |
| `src/updater.rs` | Update-mode batch logic |
| `src/auditor.rs` | Audit report building and formatting |
| `src/interactive.rs` | `ratatui` TUI for interactive migrate/update |
| `src/orchestrator.rs` | `Strategy` enum and `Orchestrate` trait for parallel/sequential dispatch |
| `src/error.rs` | Crate-wide `Error` enum and `Result<T>` alias |

## Key conventions

- **No GitHub API.** All remote queries go through `git ls-remote`.  `git` must be available on `PATH`; no authentication tokens are stored.
- **SHA resolution**: `git ls-remote` is called with both `refs/tags/<ref>` and `refs/tags/<ref>^{}` in one invocation; the peeled `^{}` entry takes priority for annotated tags, then falls back to `refs/heads/<ref>`.
- **Cargo plugin convention**: `cargo gh-shaping <cmd>` — cargo injects `"gh-shaping"` at `argv[1]`; `main.rs` strips it before passing args to clap so the binary works whether invoked directly or via cargo.
- **Parallelism**: file parsing and SHA resolution both use `Strategy`, defaulting to `Parallel` (`std::thread::scope`).  Pass `--single-threaded` to force `Strategy::Sequential`.
- **In-place rewrites** preserve existing formatting and inline comments (e.g. `# v4`).
- **Skipped action types**: `docker://` references and local path actions (starting with `.`) are not processed.

## Versioning

Crate version in `Cargo.toml` must match the git tag (`vX.Y.Z`) before publishing.  The release workflow enforces this check.
