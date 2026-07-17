use tracing_subscriber::EnvFilter;

/// Initializes a development-friendly tracing subscriber: compact,
/// colored console output, sensible default log level, and support for
/// the standard `RUST_LOG` environment variable to override verbosity.
///
/// Roughly the Djangors equivalent of Django's default `runserver`
/// request logging (e.g. `[16/Jul/2026 10:23:11] "GET /hello HTTP/1.1" 200 15`),
/// but built on the `tracing` ecosystem so any `tracing`-instrumented code
/// (this framework's middleware included) shows up automatically.
///
/// Call this once, early in your application's `main()`, before starting
/// the server. Calling it more than once is a no-op after the first
/// successful call (tracing only allows a single global subscriber).
///
/// Under the hood, this uses the standard Rust ecosystem `RUST_LOG` environment
/// variable (e.g. `RUST_LOG=debug` or `RUST_LOG=djangors_core=trace`) to override
/// the default log level, which defaults to `"info,djangors_core=debug"`.
pub fn init_dev_logging() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,djangors_core=debug"));

    let _ = tracing_subscriber::fmt()
        .compact()
        .with_env_filter(filter)
        .try_init();
}

/// Initializes a production-friendly tracing subscriber: structured JSON output,
/// sensible default log level, and support for the standard `RUST_LOG` environment
/// variable to override verbosity.
///
/// This formatter does not include colors and outputs logs in a structured JSON
/// format, which is ideal for piping into log aggregators (e.g., Elasticsearch,
/// AWS CloudWatch, Datadog).
///
/// Call this once, early in your application's `main()`, before starting
/// the server. Calling it more than once is a no-op after the first
/// successful call.
///
/// Under the hood, this uses the standard Rust ecosystem `RUST_LOG` environment
/// variable to override the default log level, which defaults to `"info,djangors_core=info"`.
pub fn init_production_logging() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,djangors_core=info"));

    let _ = tracing_subscriber::fmt()
        .json()
        .with_env_filter(filter)
        .try_init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_init_logging_does_not_panic() {
        // First initialization call should succeed.
        init_dev_logging();

        // Subsequent initialization calls should be harmless no-ops.
        init_dev_logging();
        init_production_logging();
    }
}
