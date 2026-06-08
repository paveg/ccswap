use anyhow::{Context, Result};
use ccswap::active::SystemActiveStore;
use ccswap::app::{format_identity, Ccswap};
use ccswap::hooks::{ConfiguredHooks, HookConfigStore, HookPhase, HookSpec};
use ccswap::paths::Paths;
use ccswap::vault::SystemProfileVault;
use clap::{Parser, Subcommand, ValueEnum};
use std::process::Command as ProcessCommand;

#[derive(Debug, Parser)]
#[command(name = "ccswap", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Save the active Claude Code account under a profile name.
    Save { name: String },
    /// Switch Claude Code to a saved profile (or `-` for the previous one).
    Use {
        name: String,
        /// Skip the "quit Claude Code first" advisory.
        #[arg(long)]
        force: bool,
    },
    /// List saved profiles.
    List,
    /// Show the active Claude Code account.
    Current,
    /// Delete a saved profile.
    Rm { name: String },
    /// Manage hooks run by `ccswap use`.
    Hook {
        #[command(subcommand)]
        command: HookCommand,
    },
}

#[derive(Debug, Subcommand)]
enum HookCommand {
    /// Add a hook command for a profile and phase.
    Add {
        profile: String,
        #[arg(value_enum)]
        phase: HookPhaseArg,
        /// Command and arguments to run. Use `--` before commands with flags.
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
    /// List configured hooks.
    List { profile: Option<String> },
    /// Remove a hook by profile, phase, and 1-based index.
    Rm {
        profile: String,
        #[arg(value_enum)]
        phase: HookPhaseArg,
        index: usize,
    },
    /// Open the hook config in $VISUAL or $EDITOR.
    Edit,
    /// Print the hook config path.
    Path,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum HookPhaseArg {
    #[value(alias = "pre")]
    PreUse,
    #[value(alias = "post")]
    PostUse,
}

impl From<HookPhaseArg> for HookPhase {
    fn from(value: HookPhaseArg) -> Self {
        match value {
            HookPhaseArg::PreUse => HookPhase::PreUse,
            HookPhaseArg::PostUse => HookPhase::PostUse,
        }
    }
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Save { name } => {
            let app = discover_app()?;
            let saved = app.save_profile(&name)?;
            println!(
                "saved {}\t{}",
                saved.name,
                format_identity(&saved.oauth_account)
            );
        }
        Command::Use { name, force } => {
            let app = discover_app()?;
            if !force {
                eprintln!(
                    "note: if Claude Code is running, quit it before switching or it may \
                     overwrite the change; pass --force to silence."
                );
            }
            let switched = if name == "-" {
                app.use_previous()?
            } else {
                app.use_profile(&name)?
            };
            println!(
                "current {}\t{}",
                switched.name,
                format_identity(&switched.oauth_account)
            );
        }
        Command::List => {
            let app = discover_app()?;
            for entry in app.list_profiles()? {
                let marker = if entry.current { "*" } else { " " };
                println!(
                    "{marker} {}\t{}",
                    entry.name,
                    format_identity(&entry.oauth_account)
                );
            }
        }
        Command::Current => {
            let app = discover_app()?;
            let current = app.current_profile()?;
            match current.name {
                Some(name) => println!("{name}\t{}", format_identity(&current.oauth_account)),
                None => println!(
                    "unsaved account\t{}",
                    format_identity(&current.oauth_account)
                ),
            }
        }
        Command::Rm { name } => {
            let app = discover_app()?;
            app.remove_profile(&name)?;
            println!("removed {name}");
        }
        Command::Hook { command } => run_hook_command(command)?,
    }

    Ok(())
}

fn discover_app() -> Result<Ccswap<SystemActiveStore, SystemProfileVault, ConfiguredHooks>> {
    Ccswap::<SystemActiveStore, SystemProfileVault, ConfiguredHooks>::discover()
}

fn run_hook_command(command: HookCommand) -> Result<()> {
    let paths = Paths::discover()?;
    let store = HookConfigStore::new(paths.hooks_path);

    match command {
        HookCommand::Add {
            profile,
            phase,
            command,
        } => {
            let phase = HookPhase::from(phase);
            let spec = HookSpec::from_argv(command)?;
            let index = store.add(&profile, phase, spec)?;
            println!("added {} hook #{index} for {profile}", phase.label());
        }
        HookCommand::List { profile } => {
            let config = store.load()?;
            let entries = config.entries(profile.as_deref())?;
            if entries.is_empty() {
                println!("no hooks configured");
            } else {
                for entry in entries {
                    println!(
                        "{}\t{}\t#{}\t{}",
                        entry.profile,
                        entry.phase.label(),
                        entry.index,
                        entry.spec.display_command()
                    );
                }
            }
        }
        HookCommand::Rm {
            profile,
            phase,
            index,
        } => {
            let phase = HookPhase::from(phase);
            let removed = store.remove(&profile, phase, index)?;
            println!(
                "removed {} hook #{index} for {profile}: {}",
                phase.label(),
                removed.display_command()
            );
        }
        HookCommand::Edit => {
            store.ensure_exists()?;
            open_editor(store.path())?;
            store.load().context("validate edited hook config")?;
        }
        HookCommand::Path => {
            println!("{}", store.path().display());
        }
    }

    Ok(())
}

fn open_editor(path: &std::path::Path) -> Result<()> {
    let editor = std::env::var("VISUAL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            std::env::var("EDITOR")
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
        .ok_or_else(|| {
            anyhow::anyhow!(
                "set VISUAL or EDITOR to edit hook config at {}",
                path.display()
            )
        })?;

    let mut parts = editor.split_whitespace();
    let program = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("editor command is empty"))?;
    let status = ProcessCommand::new(program)
        .args(parts)
        .arg(path)
        .status()
        .with_context(|| format!("run editor '{program}'"))?;
    if !status.success() {
        anyhow::bail!("editor exited with {status}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn use_accepts_force_flag() {
        let cli = Cli::try_parse_from(["ccswap", "use", "work", "--force"]).unwrap();
        match cli.command {
            Command::Use { name, force } => {
                assert_eq!(name, "work");
                assert!(force);
            }
            other => panic!("expected use, got {other:?}"),
        }
    }

    #[test]
    fn use_defaults_force_to_false() {
        let cli = Cli::try_parse_from(["ccswap", "use", "work"]).unwrap();
        assert!(matches!(cli.command, Command::Use { force: false, .. }));
    }

    #[test]
    fn use_accepts_dash_as_name() {
        let cli = Cli::try_parse_from(["ccswap", "use", "-"]).unwrap();
        match cli.command {
            Command::Use { name, .. } => assert_eq!(name, "-"),
            other => panic!("expected use, got {other:?}"),
        }
    }

    #[test]
    fn hook_add_captures_command_and_args() {
        let cli = Cli::try_parse_from([
            "ccswap",
            "hook",
            "add",
            "max",
            "post",
            "--",
            "switch-mcp",
            "--level",
            "max",
        ])
        .unwrap();
        match cli.command {
            Command::Hook {
                command:
                    HookCommand::Add {
                        profile,
                        phase,
                        command,
                    },
            } => {
                assert_eq!(profile, "max");
                assert!(matches!(phase, HookPhaseArg::PostUse));
                assert_eq!(command, vec!["switch-mcp", "--level", "max"]);
            }
            other => panic!("expected hook add, got {other:?}"),
        }
    }

    #[test]
    fn hook_phase_accepts_short_alias() {
        let cli = Cli::try_parse_from(["ccswap", "hook", "add", "max", "pre", "true"]).unwrap();
        match cli.command {
            Command::Hook {
                command: HookCommand::Add { phase, .. },
            } => assert!(matches!(phase, HookPhaseArg::PreUse)),
            other => panic!("expected hook add, got {other:?}"),
        }
    }
}
