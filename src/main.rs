use anyhow::Result;
use ccswap::active::SystemActiveStore;
use ccswap::app::{Ccswap, format_identity};
use ccswap::vault::SystemProfileVault;
use clap::{Parser, Subcommand};

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
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let app = Ccswap::<SystemActiveStore, SystemProfileVault>::discover()?;

    match cli.command {
        Command::Save { name } => {
            let saved = app.save_profile(&name)?;
            println!(
                "saved {}\t{}",
                saved.name,
                format_identity(&saved.oauth_account)
            );
        }
        Command::Use { name, force } => {
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
            app.remove_profile(&name)?;
            println!("removed {name}");
        }
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
}
