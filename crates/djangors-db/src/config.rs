/// Configuration settings for the database connection pool.
///
/// Unlike standard configuration structs that implement `Default`, this struct does not
/// implement `Default` because a database connection URL is always required to be provided.
/// Instead, use [`DatabaseConfig::new`] to instantiate the configuration with defaults for
/// all other settings, and customize it using the builder-style methods.
#[derive(Debug, Clone)]
pub struct DatabaseConfig {
    /// The database connection URL (e.g. "postgres://user:pass@localhost/dbname").
    pub url: String,
    /// The maximum number of connections to maintain in the pool. Default is 10.
    pub max_connections: u32,
    /// The minimum number of connections to maintain in the pool. Default is 1.
    pub min_connections: u32,
    /// The timeout in seconds when trying to connect to the database. Default is 10.
    pub connect_timeout_secs: u64,
    /// The idle timeout in seconds before closing idle connections. Default is None.
    pub idle_timeout_secs: Option<u64>,
}

impl DatabaseConfig {
    /// Create a new configuration with the required database URL and default options:
    /// - `max_connections`: 10
    /// - `min_connections`: 1
    /// - `connect_timeout_secs`: 10
    /// - `idle_timeout_secs`: None
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            max_connections: 10,
            min_connections: 1,
            connect_timeout_secs: 10,
            idle_timeout_secs: None,
        }
    }

    /// Set the maximum number of connections in the pool.
    pub fn max_connections(mut self, max: u32) -> Self {
        self.max_connections = max;
        self
    }

    /// Set the minimum number of connections in the pool.
    pub fn min_connections(mut self, min: u32) -> Self {
        self.min_connections = min;
        self
    }

    /// Set the connection timeout in seconds.
    pub fn connect_timeout_secs(mut self, secs: u64) -> Self {
        self.connect_timeout_secs = secs;
        self
    }

    /// Set the idle timeout in seconds (None means no idle timeout).
    pub fn idle_timeout_secs(mut self, secs: Option<u64>) -> Self {
        self.idle_timeout_secs = secs;
        self
    }
}
