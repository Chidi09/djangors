use clap::{Parser, Subcommand};
use std::path::PathBuf;

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
    /// Check project settings and structure.
    #[command(name = "check")]
    Check {
        /// Run production deployment pre-flight checks.
        #[arg(long)]
        deploy: bool,
    },
    /// Open an interactive database shell using psql.
    #[command(name = "dbshell")]
    Dbshell,
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
    Createsuperuser {
        /// The username for the superuser.
        #[arg(long)]
        username: String,

        /// The email address for the superuser.
        #[arg(long, default_value = "")]
        email: String,
    },
    /// Create/update the standard view/add/change/delete permissions for every registered model.
    #[command(name = "createpermissions")]
    Createpermissions,
    /// Open a REPL.
    #[command(name = "shell")]
    Shell,
    /// Run the test suite.
    #[command(name = "test")]
    Test,
    /// Collect static files from source directories into one output directory.
    #[command(name = "collectstatic")]
    Collectstatic {
        /// Source directory to collect from. May be repeated; earlier
        /// directories win on a filename collision. Defaults to "static".
        #[arg(long = "source")]
        source: Vec<PathBuf>,
        /// Output directory for the collected, hashed files and manifest.json.
        #[arg(long, default_value = "staticfiles")]
        output: PathBuf,
    },
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

    #[test]
    fn test_parse_createsuperuser() {
        let args = vec![
            "dj",
            "createsuperuser",
            "--username",
            "admin",
            "--email",
            "admin@example.com",
        ];
        let cli = Cli::try_parse_from(args).unwrap();
        match cli.command {
            Commands::Createsuperuser { username, email } => {
                assert_eq!(username, "admin");
                assert_eq!(email, "admin@example.com");
            }
            _ => panic!("Expected Commands::Createsuperuser"),
        }
    }

    #[test]
    fn test_parse_check_deploy() {
        let args = vec!["dj", "check", "--deploy"];
        let cli = Cli::try_parse_from(args).unwrap();
        match cli.command {
            Commands::Check { deploy } => {
                assert!(deploy);
            }
            _ => panic!("Expected Commands::Check"),
        }
    }

    #[test]
    fn test_parse_dbshell() {
        let args = vec!["dj", "dbshell"];
        let cli = Cli::try_parse_from(args).unwrap();
        match cli.command {
            Commands::Dbshell => {}
            _ => panic!("Expected Commands::Dbshell"),
        }
    }
}
