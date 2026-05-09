# Contributing to gDriver

Thanks for your interest in contributing.

## Getting Started

### Setup

```bash
git clone https://github.com/gdriver/gdriver.git
cd gdriver
pnpm install
cp .env.example .env   # Fill in your Google OAuth credentials
```

See the [README](README.md) for platform-specific prerequisites.

### Running in development

```bash
pnpm dev               # Start Tauri dev mode (hot-reload frontend)
cargo build --workspace  # Build all Rust crates
cargo test --workspace   # Run all tests
```

## Project Conventions

### Code Style

- **Rust**: `rustfmt` and `clippy` (see `rustfmt.toml` and `clippy.toml`). Run `cargo fmt --all` and `cargo clippy --workspace --all-targets` before committing.
- **TypeScript**: `prettier` for formatting, `tsc -b --noEmit` for type checking. Run `pnpm format` and `pnpm lint`.

### Commit Messages

Follow [conventional commits](https://www.conventionalcommits.org/en/v1.0.0/):

```
feat(sync): add incremental sync support
fix(vfs): handle FUSE unmount during active transfer
docs: update build instructions for Fedora 41
```

Prefixes: `feat`, `fix`, `docs`, `chore`, `refactor`, `test`, `ci`, `perf`

### Pull Requests

- Create a feature branch from `main`
- Keep changes focused — one concern per PR
- Ensure CI passes (format, clippy, type check, tests)
- Update relevant docs if changing user-facing behavior
- PRs need at least one review before merge

## Architecture

For an overview of the system design, see [docs/design.md](docs/design.md).

Key principles:
- The **daemon** (`gdriver-daemon`) is the single source of truth — it owns the database, sync engine, and Google API client
- The **Tauri app** is a thin UI layer that communicates with the daemon via IPC
- Platform **extensions** (shell/Finder/Nautilus) talk to the daemon through the same IPC protocol
- All Google API calls go through `gdriver-api` — never call the REST API directly from other crates

## Testing

```bash
cargo test --workspace              # All Rust tests
cargo test -p gdriver-daemon -- --test-threads=1  # Daemon tests (serial)
pnpm test                           # Frontend tests (Vitest)
```

Integration tests that hit Google APIs require valid OAuth credentials in `.env`.

## Need Help?

Open a [discussion](https://github.com/gdriver/gdriver/discussions) or ask in an issue.
