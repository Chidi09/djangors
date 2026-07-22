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
            commands::new_app(&name);
        }
        Commands::Run { port } => {
            commands::run(port);
        }
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
