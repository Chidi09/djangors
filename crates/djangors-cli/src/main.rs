mod cli;
mod commands;

use clap::Parser;
use cli::{Cli, Commands};

fn builtin_command_names() -> std::collections::HashSet<&'static str> {
    [
        "new",
        "new-app",
        "run",
        "check",
        "dbshell",
        "runworker",
        "migrate",
        "makemigrations",
        "createsuperuser",
        "createpermissions",
        "shell",
        "test",
        "collectstatic",
    ]
    .into_iter()
    .collect()
}

#[tokio::main]
async fn main() {
    // Intercept unknown subcommands and route them as custom management commands
    // via cargo run --quiet, mirroring the introspect_models pattern.
    let args: Vec<String> = std::env::args().collect();
    if let Some(cmd) = args.get(1) {
        let builtins = builtin_command_names();
        if !cmd.starts_with('-') && !builtins.contains(cmd.as_str()) {
            let extra: Vec<&str> = args.iter().skip(2).map(|s| s.as_str()).collect();
            let mut cmd_process = std::process::Command::new("cargo");
            cmd_process
                .args(["run", "--quiet"])
                .env("DJANGORS_RUN_COMMAND", cmd)
                .current_dir(".");
            if !extra.is_empty() {
                cmd_process.arg("--");
                cmd_process.args(&extra);
            }
            let status = cmd_process.status().unwrap_or_else(|e| {
                eprintln!("[dj] error: failed to execute cargo: {e}");
                std::process::exit(1);
            });
            if !status.success() {
                let mut list: Vec<&str> = builtins.into_iter().collect();
                list.sort();
                eprintln!("[dj] error: unknown command '{cmd}'");
                eprintln!("[dj] available commands: {}", list.join(", "));
                std::process::exit(1);
            }
            return;
        }
    }

    let cli = Cli::parse();

    match cli.command {
        Commands::New { name } => {
            if let Err(e) = commands::new(&name) {
                eprintln!("[dj new] error: {e}");
                std::process::exit(1);
            }
        }
        Commands::NewApp { name } => {
            if let Err(e) = commands::new_app(&name) {
                eprintln!("[dj new-app] error: {e}");
                std::process::exit(1);
            }
        }
        Commands::Run { port } => {
            if let Err(e) = commands::run(port) {
                eprintln!("[dj run] error: {e}");
                std::process::exit(1);
            }
        }
        Commands::Check { deploy } => match commands::check(deploy) {
            Ok(issues) if issues.is_empty() => println!("[dj check] no issues found"),
            Ok(issues) => {
                println!("[dj check] {} issue(s) found:", issues.len());
                for issue in issues {
                    println!("- {issue}");
                }
                std::process::exit(1);
            }
            Err(e) => {
                eprintln!("[dj check] error: {e}");
                std::process::exit(1);
            }
        },
        Commands::Dbshell => {
            if let Err(e) = commands::dbshell() {
                eprintln!("[dj dbshell] error: {e}");
                std::process::exit(1);
            }
        }
        Commands::RunWorker { poll_interval_secs } => {
            commands::runworker(poll_interval_secs).await;
        }
        Commands::Migrate { rollback } => {
            commands::migrate(rollback).await;
        }
        Commands::Makemigrations { check } => {
            if let Err(e) = commands::makemigrations(check) {
                eprintln!("[dj makemigrations] error: {e}");
                std::process::exit(1);
            }
        }
        Commands::Createsuperuser { username, email } => {
            commands::createsuperuser(username, email).await;
        }
        Commands::Createpermissions => {
            commands::createpermissions().await;
        }
        Commands::Shell => {
            if let Err(e) = commands::shell().await {
                eprintln!("[dj shell] error: {e}");
                std::process::exit(1);
            }
        }
        Commands::Test => {
            if let Err(e) = commands::test() {
                eprintln!("[dj test] error: {e}");
                std::process::exit(1);
            }
        }
        Commands::Collectstatic { source, output } => {
            commands::collectstatic(source, output);
        }
    }
}
