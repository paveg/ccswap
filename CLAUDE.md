# ccswap

Account profile switcher for Claude Code: save the logged-in account under a
name and switch between accounts/orgs. Rust CLI, distributed via npm.

Full design and decisions: `docs/DESIGN.md`, `docs/adr/`.

## What a switch touches (and nothing else)

Two pieces of Claude Code state, swapped atomically as one unit:

- **A — active credential**: macOS Keychain `Claude Code-credentials` (acct = `$USER`);
  Linux `~/.claude/.credentials.json` (0600).
- **C — account identity**: the `oauthAccount` object in `~/.claude.json`.

Never touch other `~/.claude/` state (settings, projects, history).
Account unique key = `(accountUuid, organizationUuid)`.

## Security (non-negotiable)

- Tokens must never leak. Raw token bytes live only in `Secret` (`src/secret.rs`):
  redacted `Debug`, no `Display`/`Serialize`, zeroized on drop. Do not add those impls.
- No plaintext token on disk — secrets go to the OS secret store (vault); metadata
  files hold the `oauthAccount` snapshot only, never the token.
- After install: zero network I/O. No telemetry, auto-update, or remote calls.
  Runtime deps = OS secret store + local files only. Do not add network crates.
- Never pass tokens via argv/env (they would appear in `ps`).

## Conventions

- Project-facing documentation is written in English, including `CLAUDE.md`,
  `AGENTS.md`, `docs/`, and ADRs. User-facing conversation can be Japanese.
- XDG strict via the `etcetera` Xdg strategy (macOS also uses `~/.config` /
  `~/.local/share`). Do NOT use the `directories` crate.
- `~/.claude.json` is edited surgically: replace only `oauthAccount`, atomic
  temp→rename. `serde_json` has `preserve_order` enabled — keep it, or the whole
  file gets reordered.
- Cross-platform via `ActiveStore` / `ProfileVault` traits (macOS Keychain /
  Linux Secret Service + 0600-file fallback).
- TDD: failing test first; tests live in-module under `#[cfg(test)]`. Real-keychain
  tests are `#[ignore]` integration tests. Add a dependency only when a failing
  test needs it.

## Commands

```
cargo test                 # unit tests
cargo test -- --ignored    # real OS keychain integration tests
cargo build --release
```
