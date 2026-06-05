# ADR-0001: Account profile switching model

## Status

Accepted

## Context

Claude Code supports one logged-in account at a time. Users who work across
multiple accounts/orgs (e.g. personal vs work) must log out and back in to
switch. `ccswap` automates this.

To switch accounts safely we must know exactly what state encodes "the logged-in
account". Investigation of a live macOS install found:

- The OAuth token lives in the **macOS Keychain** (service
  `Claude Code-credentials`, account = `$USER`). There is **no**
  `~/.claude/.credentials.json` on macOS. On Linux, Claude Code uses that 0600
  file instead.
- The account **identity** (email, org, UUIDs, billing) lives in the
  `oauthAccount` object inside `~/.claude.json`.
- Everything else under `~/.claude/` (settings, projects, history) is
  account-independent.

The token is a 471-byte **UTF-8 JSON** string (`{ "claudeAiOauth": ... }`),
which means it survives a byte-for-byte copy through any secret store.

Forces:

- **Security**: OAuth tokens are secrets and should not sit in plaintext on disk.
- **Portability**: target macOS and Linux; the active-credential location differs
  between them.
- **XDG**: the user wants strict XDG paths, including on macOS.
- **Distribution**: ship via npm without requiring a Rust toolchain on install.

## Decision

1. **A switch swaps two things atomically**: the active credential (**A**) and
   the `oauthAccount` object (**C**). Nothing else under `~/.claude/` is touched.

2. **Secrets live in the OS secret store, metadata in XDG.** Saved profile tokens
   go into an OS-keychain-backed vault (**B**); the non-secret `oauthAccount`
   snapshot goes into `$XDG_DATA_HOME/ccswap/profiles/`. Tokens are handled as raw
   bytes. Chosen over plaintext-in-XDG (tokens on disk) and over
   encrypted-file-in-XDG (key-management UX cost).

3. **Cross-platform via two traits.** `ActiveStore` abstracts Claude Code's live
   slot (macOS Keychain vs Linux file); `ProfileVault` abstracts ccswap's store
   (Keychain / Secret Service, with a 0600-file fallback for headless Linux).

4. **Strict XDG via `etcetera` Xdg strategy** — not the `directories` crate, which
   maps to `~/Library/...` on macOS.

5. **`~/.claude.json` is edited surgically**: read → replace only the
   `oauthAccount` key → atomic temp+rename, preserving all other keys and
   avoiding clobbering concurrent writes.

6. **npm distribution via per-platform `optionalDependencies`** (esbuild-style):
   prebuilt binaries in `@ccswap/<os>-<arch>` packages, a thin JS launcher,
   no postinstall network access.

7. **`use` is rollback-capable**: the previous account is snapshotted to
   `$XDG_STATE_HOME/ccswap/previous.json` before any mutation; partial failures
   auto-roll-back.

## Consequences

**Easier**

- Minimal blast radius: only A + C change, so a switch cannot corrupt unrelated
  Claude Code state.
- No plaintext tokens on disk; saved tokens are at least as protected as the
  source (on Linux, strictly more — Secret Service vs the plaintext source file).
- Adding Windows later is a third `ActiveStore`/`ProfileVault` impl, no redesign.

**Harder**

- Two secret backends per platform to test (Keychain entries can't be unit
  tested; real-store coverage needs `#[ignore]` integration tests).
- Headless Linux without Secret Service degrades to a 0600 file — documented, but
  a weaker guarantee than the keychain path.
- `oauthAccount` schema is undocumented; a future Claude Code change to its shape
  could require updating the snapshot/matching logic (mitigated by snapshotting
  the whole object rather than cherry-picking fields).

## Verified assumptions

- R1 (token round-trips losslessly): **confirmed** — UTF-8 JSON text.
- R3 (keychain acct = `$USER`): confirmed locally; overridable via `config.toml`.

## Open assumptions

- R2 (Linux active-credential path/shape) — validate on a real Linux host.
- R4 (Secret Service availability) — fallback path required.

## Amendments

- **R3 override mechanism** (point 2/configuration): implemented as **environment
  variables** (`CCSWAP_KEYCHAIN_ACCOUNT`, `CCSWAP_CLAUDE_JSON`,
  `CCSWAP_CREDENTIALS_PATH`), not a `config.toml`. Chosen to avoid adding a TOML
  dependency to a deliberately minimal tool; the only override that fixes R3 is a
  single string.
- **`use -` (point 7)**: the previous **token** is now persisted in the vault
  under the reserved `__previous` key (alongside the previous account in
  `previous.json`), so returning to the previous account restores both A and C.
  This also makes a hard-kill mid-switch recoverable.
- **R5 running-check (point 7)**: descoped to a `--force` flag plus a one-line
  advisory. The CLI leaves no reliable lock/pid file, and a process scan would
  false-positive when ccswap runs as a subprocess of Claude Code; reliable
  auto-detection is intentionally out of scope.
