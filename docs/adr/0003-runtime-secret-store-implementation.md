# ADR-0003: Runtime secret store implementation

## Status

Accepted

## Context

ADR-0001 decides that saved profile tokens must live in an OS secret store, with
a 0600 file fallback for headless Linux, and that active credentials are accessed
through the `ActiveStore` abstraction. The MVP implementation needs concrete
Rust dependencies for those stores.

Using platform-specific APIs directly would duplicate behavior already provided
by maintained secret-store crates and would make future Linux and Windows work
more fragmented.

## Decision

Use the `keyring` crate as the normal OS secret-store interface:

- macOS active credential: `keyring` entry for service
  `Claude Code-credentials`, account `$USER`.
- macOS profile vault: `keyring` entry for service `ccswap`, account
  `<profile-name>`.
- Linux profile vault: `keyring` with Secret Service enabled, falling back to a
  0600 file under XDG data when keyring storage is unavailable.
- Linux active credential: Claude Code's `~/.claude/.credentials.json` file,
  written atomically with mode 0600.

The code keeps `ActiveStore` and `ProfileVault` traits so future platform
support can be added without changing CLI behavior.

## Consequences

- Token handling remains byte-for-byte and does not require UTF-8 conversion.
- macOS and Linux profile vaults share one high-level dependency.
- Headless Linux still works without Secret Service, but the fallback is weaker
  than an OS secret store and must remain 0600.
- Real OS secret-store tests remain `#[ignore]` integration tests because they
  touch the user's keychain or secret service.
