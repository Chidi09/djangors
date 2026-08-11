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
