use clap::{Parser, Subcommand};
use hm::{authorization, hosts::parse_lines, mutations, output::Renderer, storage};
use std::{path::Path, process::ExitCode};

#[derive(Parser)]
#[command(
    name = "hm",
    color = clap::ColorChoice::Never,
    disable_help_flag = true,
    disable_version_flag = true,
    disable_help_subcommand = true,
    arg_required_else_help = true,
    args_conflicts_with_subcommands = true
)]
struct Cli {
    #[arg(long, global = true, action = clap::ArgAction::Help)]
    help: Option<bool>,
    #[arg(long, exclusive = true)]
    version: bool,
    #[command(subcommand)]
    command: Option<Action>,
}

#[derive(Subcommand)]
enum Action {
    List {
        #[arg(long, value_name = "QUERY")]
        search: Option<String>,
        #[arg(long)]
        json: bool,
    },
    Add {
        #[arg(value_name = "IP")]
        ip: String,
        #[arg(value_name = "HOSTNAME", required = true, num_args = 1..)]
        hostnames: Vec<String>,
        #[arg(long)]
        force: bool,
    },
    Remove {
        #[arg(value_name = "HOSTNAME", required = true, num_args = 1..)]
        hostnames: Vec<String>,
    },
    Toggle {
        #[arg(value_name = "HOSTNAME", required = true, num_args = 1..)]
        hostnames: Vec<String>,
    },
    Restore,
    Setup {
        #[arg(long)]
        remove: bool,
    },
}

fn main() -> ExitCode {
    let output = Renderer::stdout();
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            use clap::error::ErrorKind;
            if error.kind() == ErrorKind::DisplayHelp {
                print!("{}", output.help());
                return ExitCode::SUCCESS;
            }
            let errors = Renderer::stderr();
            if error.kind() == ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand {
                eprint!("{}", errors.help());
            } else {
                let message = error.to_string();
                let message = message
                    .trim_start_matches("error: ")
                    .split("\n\n")
                    .next()
                    .unwrap_or_default()
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ");
                eprint!(
                    "{}",
                    errors.error(&message, Some("Run hm --help for usage examples."))
                );
            }
            return ExitCode::FAILURE;
        }
    };
    if cli.version {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    // SAFETY: geteuid only returns the calling process's effective user ID.
    let unprivileged = unsafe { libc::geteuid() } != 0;
    let writes = matches!(
        cli.command,
        Some(Action::Add { .. } | Action::Remove { .. } | Action::Toggle { .. } | Action::Restore)
    );
    if writes && !unprivileged {
        if let Err(error) =
            authorization::validate_system_hosts(matches!(cli.command, Some(Action::Restore)))
        {
            eprint!("{}", Renderer::stderr().error(&error, None));
            return ExitCode::FAILURE;
        }
    }
    let result = if unprivileged && writes {
        if let Some(Action::Add {
            ip,
            hostnames,
            force,
        }) = &cli.command
        {
            if let Err(error) = mutations::add(&[], ip, hostnames, *force) {
                eprint!("{}", Renderer::stderr().error(&error, None));
                return ExitCode::FAILURE;
            }
        }
        match authorization::execute(&std::env::args_os().skip(1).collect::<Vec<_>>()) {
            Ok(code) => return ExitCode::from(code),
            Err(error) => Err(error),
        }
    } else {
        run(cli, Path::new("/etc/hosts"), &output)
    };
    match result {
        Ok(text) => {
            print!("{text}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprint!("{}", Renderer::stderr().error(&error, None));
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli, path: &Path, output: &Renderer) -> Result<String, String> {
    match cli.command.ok_or("No command provided.")? {
        Action::Add {
            ip,
            hostnames,
            force,
        } => {
            // Validate input before reading the selected file.
            let _ = mutations::add(&[], &ip, &hostnames, force)?;
            let result =
                storage::update(path, |lines| mutations::add(lines, &ip, &hostnames, force))?;
            Ok(output.mutation(&result, path))
        }
        Action::Remove { hostnames } => {
            let result = storage::update(path, |lines| mutations::remove(lines, &hostnames))?;
            Ok(output.mutation(&result, path))
        }
        Action::Toggle { hostnames } => {
            let result = storage::update(path, |lines| mutations::toggle(lines, &hostnames))?;
            Ok(output.mutation(&result, path))
        }
        Action::Restore => {
            storage::restore(path)?;
            Ok(output.restored(path))
        }
        Action::Setup { remove } => authorization::setup(remove),
        Action::List { search, json } => {
            let mut entries = parse_lines(&storage::read(path)?.lines);
            entries.retain(|entry| {
                search.as_ref().is_none_or(|query| {
                    let query = query.to_lowercase();
                    entry.ip.to_lowercase().contains(&query)
                        || entry
                            .hostnames
                            .iter()
                            .any(|hostname| hostname.to_lowercase().contains(&query))
                })
            });
            if json {
                Ok(format!(
                    "{}\n",
                    serde_json::to_string_pretty(&entries).map_err(|error| error.to_string())?
                ))
            } else {
                Ok(output.list(&entries, path, search.as_deref()))
            }
        }
    }
}

#[cfg(test)]
#[path = "cli_tests.rs"]
mod tests;
