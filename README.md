# ccswap

Account profile switcher for Claude Code.

`ccswap` saves the currently logged-in Claude Code account under a profile name,
then switches Claude Code between saved accounts or organizations.

## Features

- Save the active Claude Code account as a named profile.
- Switch between saved profiles.
- Jump back to the previous account with `ccswap use -`.
- List saved profiles and mark the current one.
- Show whether the active account matches a saved profile.
- Remove saved profiles.

`ccswap` only touches the Claude Code account state required for a switch:

- the active credential Claude Code reads now
- the `oauthAccount` object in `~/.claude.json`

It does not modify Claude Code settings, projects, history, or other files under
`~/.claude/`.

## Install

From this repository:

```sh
cargo install --path .
```

Or build a local binary:

```sh
cargo build --release
./target/release/ccswap --help
```

## Usage

First, log in to Claude Code with the account you want to save. Then save it:

```sh
ccswap save work
```

Log in to another Claude Code account and save that too:

```sh
ccswap save personal
```

Switch accounts:

```sh
ccswap use work
ccswap use personal
```

Return to the account that was active before the last switch:

```sh
ccswap use -
```

List profiles:

```sh
ccswap list
```

Show the active account:

```sh
ccswap current
```

Delete a profile:

```sh
ccswap rm personal
```

Profile names may contain ASCII letters, numbers, `.`, `_`, and `-`.
The names `.`, `..`, `-`, and `__previous` are reserved.

## Switching Safely

Quit Claude Code before running `ccswap use <name>`.

Claude Code can rewrite its own account state while it is running. `ccswap use`
prints an advisory by default; pass `--force` to silence it:

```sh
ccswap use work --force
```

During a switch, `ccswap` snapshots the current account and credential before it
writes the target account. If a normal write failure occurs, it attempts to roll
back to the previous account.

## Storage

`ccswap` stores profile metadata separately from credential bytes.

- Profile metadata is the saved `oauthAccount` JSON object.
- Credential bytes are handled through the platform active store and profile
  vault abstractions.
- Metadata files never contain OAuth tokens.
- `~/.claude.json` is edited surgically: only `oauthAccount` is replaced.

Default paths use XDG directories on all platforms, including macOS:

```text
$XDG_DATA_HOME/ccswap/profiles/<name>.json
$XDG_DATA_HOME/ccswap/vault/<name>.secret
$XDG_STATE_HOME/ccswap/previous.json
```

With standard defaults, these resolve under:

```text
~/.local/share/ccswap/
~/.local/state/ccswap/
```

Claude Code state:

| Platform | Active credential | Account identity |
| --- | --- | --- |
| macOS | Keychain service `Claude Code-credentials`, account `$USER` | `~/.claude.json` |
| Linux | `~/.claude/.credentials.json` | `~/.claude.json` |

## Configuration

There is no config file. Optional environment variables can override discovered
paths:

| Variable | Purpose |
| --- | --- |
| `CCSWAP_KEYCHAIN_ACCOUNT` | macOS Keychain account for Claude Code credentials. Defaults to `$USER`. |
| `CCSWAP_CLAUDE_JSON` | Path to the Claude Code JSON state file. Defaults to `~/.claude.json`. |
| `CCSWAP_CREDENTIALS_PATH` | Linux active credential path. Defaults to `~/.claude/.credentials.json`. |

## Development

```sh
cargo test
cargo test -- --ignored
cargo build --release
```

Ignored tests touch the real OS keychain or secret store.

See [docs/DESIGN.md](docs/DESIGN.md) and [docs/adr](docs/adr) for the design
notes and architectural decisions.

## License

MIT. See [LICENSE](LICENSE).
