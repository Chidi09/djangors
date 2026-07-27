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

/// Initializes Sentry error tracking together with a production-style structured
/// JSON `tracing` subscriber, in one call.
///
/// `tracing` only allows a single global subscriber to be installed process-wide,
/// so Sentry can't be bolted onto an already-initialized subscriber from
/// [`init_production_logging`] after the fact — this function builds the combined
/// layered subscriber (JSON formatting + Sentry event/breadcrumb capture) itself.
///
/// Returns a guard that **must be held for the lifetime of the process** —
/// typically `let _sentry_guard = djangors_core::logging::init_production_logging_with_sentry(&dsn);`
/// at the top of `main()`. Dropping it flushes any queued events before the
/// client shuts down. An empty or invalid `dsn` produces a disabled client
/// (matching the Sentry SDK's own convention across languages) rather than
/// erroring, so this is always safe to call unconditionally with a settings
/// value that may be empty in development.
///
/// `ERROR`-level `tracing` events are captured as Sentry events automatically;
/// `WARN`/`INFO`/`DEBUG`/`TRACE` events become breadcrumbs attached to the next
/// captured event. Panics are captured automatically via Sentry's built-in panic
/// integration (enabled by this crate's `sentry` feature).
///
/// Requires the `sentry` Cargo feature.
#[cfg(feature = "sentry")]
pub fn init_production_logging_with_sentry(dsn: &str) -> sentry::ClientInitGuard {
    let guard = sentry::init((
        dsn,
        sentry::ClientOptions {
            release: sentry::release_name!(),
            ..Default::default()
        },
    ));

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,djangors_core=info"));

    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().json())
        .with(sentry_tracing::layer())
        .try_init();

    guard
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

    #[cfg(feature = "sentry")]
    #[test]
    fn test_init_sentry_logging_with_empty_dsn_does_not_panic_or_reach_the_network() {
        // An empty DSN produces a disabled Sentry client (the SDK's own convention
        // across languages, not a Djangors-specific fallback) - safe to call in a
        // unit test with no real Sentry project configured, and confirms the
        // combined layered subscriber (JSON fmt + sentry-tracing) actually builds.
        let guard = init_production_logging_with_sentry("");
        assert!(!guard.is_enabled());
    }
}
