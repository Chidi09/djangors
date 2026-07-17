use clap::{Parser, Subcommand};

/// The command-line interface for the Djangors web framework.
#[derive(Parser, Debug)]
#[command(
    name = "dj",
    version,
    about = "Djangors CLI",
    long_about = "The dj command-line tool for scaffolding, running, and managing Djangors projects."
)]
pub struct Cli {
    /// The subcommand to execute.
    #[command(subcommand)]
    pub command: Commands,
}

/// Available subcommands for the dj binary.
#[derive(Subcommand, Debug, Clone)]
pub enum Commands {
    /// Create a new Djangors project.
    #[command(name = "new")]
    New {
        /// The name of the new project.
        name: String,
    },
    /// Create a new app within a project.
    #[command(name = "new-app")]
    NewApp {
        /// The name of the new app.
        name: String,
    },
    /// Start the dev server.
    #[command(name = "run")]
    Run {
        /// The port to listen on.
        #[arg(long, default_value_t = 8000)]
        port: u16,
    },
    /// Apply database migrations.
    #[command(name = "migrate")]
    Migrate,
    /// Generate new migrations.
    #[command(name = "makemigrations")]
    Makemigrations {
        /// Check for changes without writing them.
        #[arg(long)]
        check: bool,
    },
    /// Create an admin user.
    #[command(name = "createsuperuser")]
    Createsuperuser,
    /// Open a REPL.
    #[command(name = "shell")]
    Shell,
    /// Run the test suite.
    #[command(name = "test")]
    Test,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_new() {
        let args = vec!["dj", "new", "myproject"];
        let cli = Cli::try_parse_from(args).unwrap();
        match cli.command {
            Commands::New { name } => {
                assert_eq!(name, "myproject");
            }
            _ => panic!("Expected Commands::New"),
        }
    }

    #[test]
    fn test_parse_run_with_port() {
        let args = vec!["dj", "run", "--port", "9000"];
        let cli = Cli::try_parse_from(args).unwrap();
        match cli.command {
            Commands::Run { port } => {
                assert_eq!(port, 9000);
            }
            _ => panic!("Expected Commands::Run"),
        }
    }

    #[test]
    fn test_parse_run_default_port() {
        let args = vec!["dj", "run"];
        let cli = Cli::try_parse_from(args).unwrap();
        match cli.command {
            Commands::Run { port } => {
                assert_eq!(port, 8000);
            }
            _ => panic!("Expected Commands::Run"),
        }
    }

    #[test]
    fn test_parse_makemigrations_check() {
        let args = vec!["dj", "makemigrations", "--check"];
        let cli = Cli::try_parse_from(args).unwrap();
        match cli.command {
            Commands::Makemigrations { check } => {
                assert!(check);
            }
            _ => panic!("Expected Commands::Makemigrations"),
        }
    }
}
