#![deny(missing_docs)]
//! TOTP-based two-factor authentication primitives for Djangors.
//!
//! This v1 intentionally ships TOTP only; WebAuthn is a separate future scope.  `OtpDevice`
//! stores the base32 secret in plaintext because this codebase has no existing reversible
//! encryption-at-rest convention for secrets (password hashing is one-way and cannot be reused).
//! Secret encryption is a known v1 gap and should be addressed by a dedicated future project.
//!
//! `djangors-admin` has no username/password login handler: applications own their login views.
//! After password authentication, an application should find the user's confirmed device and
//! call [`verify_code`] before establishing the final authenticated session. Enrollment is the
//! usual create-device → show [`provisioning_uri`] as a QR payload → verify first code → set
//! `confirmed = true` flow.

use djangors_macros::Model;
use djangors_orm::ForeignKey;
use totp_rs::{Algorithm, Secret, TOTP};

/// Represents an enrolled or unconfirmed TOTP device for a user.
#[derive(Model, Debug, Clone)]
#[djangors(app = "djangors_contrib_otp", table_name = "djangors_otp_device")]
pub struct OtpDevice {
    /// Auto-incrementing primary key.
    #[djangors(primary_key, auto)]
    pub id: i64,
    /// Foreign key reference to the owning user.
    pub user: ForeignKey<djangors_auth::User>,
    /// Base32 encoded TOTP secret key.
    #[djangors(max_length = 255)]
    pub secret: String,
    /// Whether the device setup has been confirmed by verifying a code.
    pub confirmed: bool,
}

/// Generates a new random base32 encoded TOTP secret key.
pub fn generate_secret() -> String {
    Secret::generate_secret().to_encoded().to_string()
}

/// Generates an `otpauth://` provisioning URI string for QR code creation.
pub fn provisioning_uri(secret: &str, account_name: &str, issuer: &str) -> String {
    let totp = TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        Secret::Encoded(secret.to_string())
            .to_bytes()
            .expect("generated TOTP secret must be valid base32"),
        Some(issuer.to_string()),
        account_name.to_string(),
    )
    .expect("standard generated TOTP parameters must be valid");
    totp.get_url()
}

/// Verifies a 6-digit TOTP code against a base32 encoded secret key.
pub fn verify_code(secret: &str, code: &str) -> bool {
    let Ok(bytes) = Secret::Encoded(secret.to_string()).to_bytes() else {
        return false;
    };
    let Ok(totp) = TOTP::new(Algorithm::SHA1, 6, 1, 30, bytes, None, String::new()) else {
        return false;
    };
    let Ok(now) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) else {
        return false;
    };
    totp.check(code, now.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use djangors_orm::Model;

    #[test]
    fn generated_code_verifies_and_invalid_code_does_not() {
        let secret = generate_secret();
        let bytes = Secret::Encoded(secret.clone()).to_bytes().unwrap();
        let totp = TOTP::new(Algorithm::SHA1, 6, 1, 30, bytes, None, String::new()).unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let code = totp.generate(now);
        assert!(verify_code(&secret, &code));
        assert!(!verify_code(&secret, "000000"));
    }

    #[test]
    fn provisioning_uri_has_expected_fields() {
        let uri = provisioning_uri(&generate_secret(), "alice@example.com", "Djangors");
        assert!(uri.starts_with("otpauth://totp/"));
        assert!(uri.contains("alice%40example.com"));
        assert!(uri.contains("issuer=Djangors"));
    }

    #[tokio::test]
    async fn enrollment_round_trip_against_database() {
        let Ok(db) = djangors_test::TestDatabase::connect().await else {
            return;
        };
        db.create_table("CREATE TABLE IF NOT EXISTS djangors_otp_device (id BIGSERIAL PRIMARY KEY, \"user\" BIGINT NOT NULL, secret VARCHAR(255) NOT NULL, confirmed BOOLEAN NOT NULL)").await.unwrap();
        let user_id = 42;
        let device = OtpDevice {
            id: 0,
            user: ForeignKey::new(user_id),
            secret: generate_secret(),
            confirmed: false,
        }
        .save(db.database())
        .await
        .unwrap();
        assert!(!device.confirmed);
        sqlx::query("UPDATE djangors_otp_device SET confirmed = TRUE WHERE id = $1")
            .bind(device.id)
            .execute(db.database().pool())
            .await
            .unwrap();
        let loaded = OtpDevice::objects()
            .filter(djangors_orm::q!(id = device.id))
            .unwrap()
            .first(db.database())
            .await
            .unwrap()
            .unwrap();
        assert!(loaded.confirmed);
        db.drop_table("djangors_otp_device").await.unwrap();
    }
}
