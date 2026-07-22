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
        Commands::Check => match commands::check() {
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
        Commands::Migrate => {
            commands::migrate().await;
        }
        Commands::Makemigrations { check } => {
            commands::makemigrations(check);
        }
        Commands::Createsuperuser { username, email } => {
            commands::createsuperuser(username, email).await;
        }
        Commands::Createpermissions => {
            commands::createpermissions().await;
        }
        Commands::Shell => {
            commands::shell();
        }
        Commands::Test => {
            commands::test();
        }
        Commands::Collectstatic { source, output } => {
            commands::collectstatic(source, output);
        }
    }
}
