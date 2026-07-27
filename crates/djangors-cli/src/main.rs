mod cli;
mod commands;

use clap::Parser;
use cli::{Cli, Commands};

#[tokio::main]
async fn main() {
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
        Commands::Migrate => {
            commands::migrate().await;
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
