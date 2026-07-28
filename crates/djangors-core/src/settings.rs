//! Settings and configuration system for Djangors.
//! Mirrors Django's settings approach with environment variable overlays
//! and file-based TOML configuration.

use crate::error::DjangorsError;
use serde::Deserialize;
use std::fmt;

/// Error type produced by `#[derive(Settings)]`-generated `load()` methods.
///
/// Distinguishes a required field with no matching environment variable from a
/// field whose environment variable was present but couldn't be parsed into the
/// field's type, so callers (and error messages) can tell the two apart.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SettingsError {
    /// A field had no `#[djangors(default = ...)]` and wasn't `Option<T>`, but its
    /// environment variable was not set.
    #[error(
        "missing required setting: environment variable `{env_var}` is not set (field `{field}`)"
    )]
    MissingRequired {
        /// The struct field name.
        field: &'static str,
        /// The environment variable name that was checked.
        env_var: String,
    },
    /// A field's environment variable was set but could not be parsed into the
    /// field's declared type.
    #[error("invalid value for setting `{field}` (env var `{env_var}`): {message}")]
    InvalidValue {
        /// The struct field name.
        field: &'static str,
        /// The environment variable name that was checked.
        env_var: String,
        /// A human-readable description of why parsing failed.
        message: String,
    },
}

/// Implemented for every type a `#[derive(Settings)]` field can hold, so the derive
/// macro can generate a single generic `std::env::var(...)` + parse call per field
/// regardless of its concrete type.
///
/// Djangors implements this for `String`, `bool`, every built-in integer and float
/// type, and `Vec<String>` (parsed as a comma-separated list, matching
/// `DjangorsSettings`'s own `DJANGORS_ALLOWED_HOSTS` convention). `Option<T>` fields
/// are handled directly by the derive macro (absent env var -> `None`) rather than
/// through this trait.
pub trait FromSettingsValue: Sized {
    /// Parses a raw environment variable string into `Self`.
    fn parse_settings_value(raw: &str) -> Result<Self, String>;
}

impl FromSettingsValue for String {
    fn parse_settings_value(raw: &str) -> Result<Self, String> {
        Ok(raw.to_string())
    }
}

impl FromSettingsValue for bool {
    fn parse_settings_value(raw: &str) -> Result<Self, String> {
        match raw.to_lowercase().as_str() {
            "true" | "1" => Ok(true),
            "false" | "0" => Ok(false),
            other => Err(format!(
                "expected a boolean (`true`/`false`/`1`/`0`), got `{other}`"
            )),
        }
    }
}

impl FromSettingsValue for Vec<String> {
    fn parse_settings_value(raw: &str) -> Result<Self, String> {
        Ok(raw
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect())
    }
}

macro_rules! impl_from_settings_value_via_parse {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl FromSettingsValue for $ty {
                fn parse_settings_value(raw: &str) -> Result<Self, String> {
                    raw.parse::<$ty>().map_err(|e| e.to_string())
                }
            }
        )+
    };
}

impl_from_settings_value_via_parse!(
    i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, f32, f64
);

/// Settings structure holding configuration parameters for the Djangors framework.
/// Mirrors Django's `settings.py` structure.
#[derive(Clone)]
pub struct DjangorsSettings {
    /// Mirrors Django's `DEBUG` setting. If true, detailed error reports are shown.
    /// Never set to true in production.
    pub debug: bool,

    /// Mirrors Django's `ALLOWED_HOSTS` setting. A list of host/domain names that
    /// this site can serve.
    pub allowed_hosts: Vec<String>,

    /// Mirrors Django's `SECRET_KEY` setting. A secret key used for cryptographic signing.
    /// Must be kept confidential and never be empty in production.
    pub secret_key: String,

    /// Bind address host name or IP address (e.g., "127.0.0.1").
    pub host: String,

    /// Port number to listen on (e.g., 8000).
    pub port: u16,
}

impl fmt::Debug for DjangorsSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DjangorsSettings")
            .field("debug", &self.debug)
            .field("allowed_hosts", &self.allowed_hosts)
            .field("secret_key", &"[redacted]")
            .field("host", &self.host)
            .field("port", &self.port)
            .finish()
    }
}

impl Default for DjangorsSettings {
    fn default() -> Self {
        Self {
            debug: true,
            allowed_hosts: vec!["127.0.0.1".into(), "localhost".into()],
            secret_key: String::new(),
            host: "127.0.0.1".into(),
            port: 8000,
        }
    }
}

#[derive(Deserialize)]
struct PartialSettings {
    debug: Option<bool>,
    allowed_hosts: Option<Vec<String>>,
    secret_key: Option<String>,
    host: Option<String>,
    port: Option<u16>,
}

impl DjangorsSettings {
    /// Overlays environment variables onto the settings struct.
    /// Returns a list of warning messages for any malformed environment variables.
    fn merge_env(&mut self) -> Vec<String> {
        let mut warnings = Vec::new();

        if let Ok(val) = std::env::var("DJANGORS_DEBUG") {
            match val.to_lowercase().as_str() {
                "true" | "1" => self.debug = true,
                "false" | "0" => self.debug = false,
                _ => warnings.push(format!("Ignored invalid env var DJANGORS_DEBUG: '{}'", val)),
            }
        }

        if let Ok(val) = std::env::var("DJANGORS_ALLOWED_HOSTS") {
            self.allowed_hosts = val
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }

        if let Ok(val) = std::env::var("DJANGORS_SECRET_KEY") {
            self.secret_key = val;
        }

        if let Ok(val) = std::env::var("DJANGORS_HOST") {
            self.host = val;
        }

        if let Ok(val) = std::env::var("DJANGORS_PORT") {
            match val.parse::<u16>() {
                Ok(port) => self.port = port,
                Err(_) => {
                    warnings.push(format!("Ignored invalid env var DJANGORS_PORT: '{}'", val))
                }
            }
        }

        warnings
    }

    /// Loads settings entirely from the environment variables, starting from defaults.
    /// Returns the constructed settings along with warning messages for malformed environment variables.
    pub fn from_env() -> (Self, Vec<String>) {
        let mut settings = Self::default();
        let warnings = settings.merge_env();
        (settings, warnings)
    }

    /// Parses a TOML configuration string, overlaying specified fields onto defaults.
    pub fn from_toml_str(s: &str) -> Result<Self, DjangorsError> {
        let partial: PartialSettings = toml::from_str(s).map_err(|e| {
            DjangorsError::Internal(format!("Failed to parse TOML settings: {}", e))
        })?;

        let mut settings = Self::default();
        if let Some(debug) = partial.debug {
            settings.debug = debug;
        }
        if let Some(allowed_hosts) = partial.allowed_hosts {
            settings.allowed_hosts = allowed_hosts;
        }
        if let Some(secret_key) = partial.secret_key {
            settings.secret_key = secret_key;
        }
        if let Some(host) = partial.host {
            settings.host = host;
        }
        if let Some(port) = partial.port {
            settings.port = port;
        }

        Ok(settings)
    }

    /// The standard entry point for loading framework settings.
    /// Starts from defaults, overlays values from `djangors.toml` if it exists in the current working directory,
    /// and finally overlays any environment variables.
    /// Returns the loaded settings and warning messages.
    pub fn load() -> Result<(Self, Vec<String>), DjangorsError> {
        let mut settings = if std::path::Path::new("djangors.toml").exists() {
            let content = std::fs::read_to_string("djangors.toml").map_err(|e| {
                DjangorsError::Internal(format!("Failed to read djangors.toml: {}", e))
            })?;
            Self::from_toml_str(&content)?
        } else {
            Self::default()
        };

        let warnings = settings.merge_env();
        Ok((settings, warnings))
    }

    /// Validates the loaded settings structure.
    /// Returns an error if the configuration is invalid or unsafe for startup.
    pub fn validate(&self) -> Result<(), DjangorsError> {
        if self.secret_key.is_empty() && !self.debug {
            return Err(DjangorsError::Internal(
                "SECRET_KEY cannot be empty when DEBUG is false".into(),
            ));
        }
        if self.port == 0 {
            return Err(DjangorsError::Internal("Port cannot be 0".into()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Mutex to ensure environment variable tests do not run concurrently.
    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    struct EnvGuard<'a> {
        _lock: std::sync::MutexGuard<'a, ()>,
        vars: Vec<&'static str>,
    }

    impl<'a> EnvGuard<'a> {
        fn new(vars: Vec<&'static str>) -> Self {
            let lock = ENV_MUTEX.lock().unwrap();
            for var in &vars {
                std::env::remove_var(var);
            }
            Self { _lock: lock, vars }
        }
    }

    impl<'a> Drop for EnvGuard<'a> {
        fn drop(&mut self) {
            for var in &self.vars {
                std::env::remove_var(var);
            }
        }
    }

    #[test]
    fn test_defaults() {
        let settings = DjangorsSettings::default();
        assert!(settings.debug);
        assert_eq!(
            settings.allowed_hosts,
            vec!["127.0.0.1".to_string(), "localhost".to_string()]
        );
        assert_eq!(settings.secret_key, "");
        assert_eq!(settings.host, "127.0.0.1");
        assert_eq!(settings.port, 8000);
    }

    #[test]
    fn test_from_toml_str_partial() {
        let toml_content = r#"
            debug = false
            port = 9000
        "#;
        let settings = DjangorsSettings::from_toml_str(toml_content).unwrap();
        assert!(!settings.debug);
        assert_eq!(settings.port, 9000);
        // Untouched fields should be default
        assert_eq!(
            settings.allowed_hosts,
            vec!["127.0.0.1".to_string(), "localhost".to_string()]
        );
        assert_eq!(settings.secret_key, "");
        assert_eq!(settings.host, "127.0.0.1");
    }

    #[test]
    fn test_from_env_clean() {
        let _guard = EnvGuard::new(vec![
            "DJANGORS_DEBUG",
            "DJANGORS_ALLOWED_HOSTS",
            "DJANGORS_SECRET_KEY",
            "DJANGORS_HOST",
            "DJANGORS_PORT",
        ]);

        std::env::set_var("DJANGORS_DEBUG", "false");
        std::env::set_var("DJANGORS_ALLOWED_HOSTS", "example.com,test.org");
        std::env::set_var("DJANGORS_SECRET_KEY", "super-secret-key-123");
        std::env::set_var("DJANGORS_HOST", "0.0.0.0");
        std::env::set_var("DJANGORS_PORT", "8888");

        let (settings, warnings) = DjangorsSettings::from_env();
        assert!(warnings.is_empty());
        assert!(!settings.debug);
        assert_eq!(
            settings.allowed_hosts,
            vec!["example.com".to_string(), "test.org".to_string()]
        );
        assert_eq!(settings.secret_key, "super-secret-key-123");
        assert_eq!(settings.host, "0.0.0.0");
        assert_eq!(settings.port, 8888);
    }

    #[test]
    fn test_from_env_malformed() {
        let _guard = EnvGuard::new(vec!["DJANGORS_DEBUG", "DJANGORS_PORT"]);

        std::env::set_var("DJANGORS_DEBUG", "not-a-bool");
        std::env::set_var("DJANGORS_PORT", "not-a-port");

        let (settings, warnings) = DjangorsSettings::from_env();
        assert_eq!(warnings.len(), 2);
        assert!(warnings[0].contains("DJANGORS_DEBUG"));
        assert!(warnings[1].contains("DJANGORS_PORT"));

        // Defaults should be preserved
        assert!(settings.debug);
        assert_eq!(settings.port, 8000);
    }

    #[test]
    fn test_validate() {
        let mut settings = DjangorsSettings::default();

        // Dev/debug mode with empty secret key should be valid (this is
        // exactly Default::default()'s state — debug: true, secret_key: "").
        assert!(settings.validate().is_ok());

        // Production mode with empty secret key should fail validation
        settings.debug = false;
        settings.secret_key = "".to_string();
        assert!(settings.validate().is_err());

        // Production mode with non-empty secret key should be valid
        settings.secret_key = "some-key".to_string();
        assert!(settings.validate().is_ok());

        // Port 0 should fail validation
        settings.port = 0;
        assert!(settings.validate().is_err());
    }

    #[derive(djangors_macros::Settings, Debug, PartialEq)]
    #[djangors(prefix = "TESTAPP")]
    struct DeriveSettingsFixture {
        api_key: String,
        #[djangors(default = "https://api.example.com".to_string())]
        base_url: String,
        #[djangors(default = 30)]
        timeout_secs: u64,
        feature_flag: Option<bool>,
        allowed_origins: Option<Vec<String>>,
    }

    #[test]
    fn derive_settings_required_field_missing_errors() {
        let _guard = EnvGuard::new(vec![
            "TESTAPP_API_KEY",
            "TESTAPP_BASE_URL",
            "TESTAPP_TIMEOUT_SECS",
            "TESTAPP_FEATURE_FLAG",
            "TESTAPP_ALLOWED_ORIGINS",
        ]);

        let err = DeriveSettingsFixture::load().unwrap_err();
        assert_eq!(
            err,
            SettingsError::MissingRequired {
                field: "api_key",
                env_var: "TESTAPP_API_KEY".to_string(),
            }
        );
    }

    #[test]
    fn derive_settings_applies_defaults_and_parses_option_fields() {
        let _guard = EnvGuard::new(vec![
            "TESTAPP_API_KEY",
            "TESTAPP_BASE_URL",
            "TESTAPP_TIMEOUT_SECS",
            "TESTAPP_FEATURE_FLAG",
            "TESTAPP_ALLOWED_ORIGINS",
        ]);
        std::env::set_var("TESTAPP_API_KEY", "sk_live_123");

        let settings = DeriveSettingsFixture::load().unwrap();
        assert_eq!(settings.api_key, "sk_live_123");
        assert_eq!(settings.base_url, "https://api.example.com");
        assert_eq!(settings.timeout_secs, 30);
        assert_eq!(settings.feature_flag, None);
        assert_eq!(settings.allowed_origins, None);
    }

    #[test]
    fn derive_settings_env_vars_override_defaults_and_parse_every_type() {
        let _guard = EnvGuard::new(vec![
            "TESTAPP_API_KEY",
            "TESTAPP_BASE_URL",
            "TESTAPP_TIMEOUT_SECS",
            "TESTAPP_FEATURE_FLAG",
            "TESTAPP_ALLOWED_ORIGINS",
        ]);
        std::env::set_var("TESTAPP_API_KEY", "sk_live_456");
        std::env::set_var("TESTAPP_BASE_URL", "https://staging.example.com");
        std::env::set_var("TESTAPP_TIMEOUT_SECS", "90");
        std::env::set_var("TESTAPP_FEATURE_FLAG", "true");
        std::env::set_var("TESTAPP_ALLOWED_ORIGINS", "a.com, b.com,c.com");

        let settings = DeriveSettingsFixture::load().unwrap();
        assert_eq!(settings.api_key, "sk_live_456");
        assert_eq!(settings.base_url, "https://staging.example.com");
        assert_eq!(settings.timeout_secs, 90);
        assert_eq!(settings.feature_flag, Some(true));
        assert_eq!(
            settings.allowed_origins,
            Some(vec![
                "a.com".to_string(),
                "b.com".to_string(),
                "c.com".to_string()
            ])
        );
    }

    #[test]
    fn derive_settings_invalid_value_is_a_distinct_error_from_missing() {
        let _guard = EnvGuard::new(vec![
            "TESTAPP_API_KEY",
            "TESTAPP_BASE_URL",
            "TESTAPP_TIMEOUT_SECS",
            "TESTAPP_FEATURE_FLAG",
            "TESTAPP_ALLOWED_ORIGINS",
        ]);
        std::env::set_var("TESTAPP_API_KEY", "sk_live_789");
        std::env::set_var("TESTAPP_TIMEOUT_SECS", "not-a-number");

        let err = DeriveSettingsFixture::load().unwrap_err();
        match err {
            SettingsError::InvalidValue { field, env_var, .. } => {
                assert_eq!(field, "timeout_secs");
                assert_eq!(env_var, "TESTAPP_TIMEOUT_SECS");
            }
            other => panic!("expected InvalidValue, got {other:?}"),
        }
    }

    #[test]
    fn test_custom_debug_redacts_secret_key() {
        let settings = DjangorsSettings {
            debug: true,
            allowed_hosts: vec![],
            secret_key: "my-super-secret-unredacted-key".to_string(),
            host: "127.0.0.1".to_string(),
            port: 8000,
        };

        let debug_str = format!("{:?}", settings);
        assert!(!debug_str.contains("my-super-secret-unredacted-key"));
        assert!(debug_str.contains("[redacted]"));
    }
}
