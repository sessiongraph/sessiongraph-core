//! License key validation for the SessionGraph desktop app.
//!
//! The server issues RS256-signed JWTs containing { sub, tier, seats, iat, exp }.
//! The desktop app validates the JWT locally using an embedded public key (no
//! network required per validation). Once per day the app phones home to
//! `/api/license/validate` to check for revocation and refresh the cached tier.
//!
//! License file: `~/.sessiongraph/license.json`
//! ```json
//! { "key": "<jwt>", "tier": "pro", "seats": 1, "expires_at": "2027-05-16T..." }
//! ```

use anyhow::Context;
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// The RS256 public key is embedded at compile time via an environment variable
// set during the release build. `option_env!` returns None in dev builds where
// the env var is absent — verify_jwt() will fail gracefully and get_license_status()
// falls through to free tier.
const EMBEDDED_PUBLIC_KEY: Option<&str> = option_env!("SG_LICENSE_PUBLIC_KEY");

/// Claims encoded in the license JWT.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LicenseClaims {
    pub sub: String, // userId
    pub tier: String,
    pub seats: u32,
    pub iat: u64,
    pub exp: u64,
}

/// Persisted license file on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LicenseFile {
    pub key: String,
    pub tier: String,
    pub seats: u32,
    pub expires_at: Option<String>,
    /// ISO-8601 of last successful phone-home (used for 7-day offline grace).
    pub last_validated: Option<String>,
}

/// The resolved license status exposed to the frontend via Tauri IPC.
#[derive(Debug, Clone, Serialize)]
pub struct LicenseStatus {
    pub tier: String,
    pub seats: u32,
    pub valid: bool,
    pub expires_at: Option<String>,
    pub source: &'static str, // "local_jwt" | "cached" | "free_fallback"
}

fn license_path() -> anyhow::Result<PathBuf> {
    let home = if cfg!(windows) {
        std::env::var("USERPROFILE").context("USERPROFILE not set")?
    } else {
        std::env::var("HOME").context("HOME not set")?
    };
    Ok(PathBuf::from(home)
        .join(".sessiongraph")
        .join("license.json"))
}

/// Read the license file from disk. Returns `None` if absent or malformed.
pub fn read_license_file() -> Option<LicenseFile> {
    let path = license_path().ok()?;
    let contents = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&contents).ok()
}

/// Write a license file to disk.
pub fn write_license_file(lf: &LicenseFile) -> anyhow::Result<()> {
    let path = license_path()?;
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    let json = serde_json::to_string_pretty(lf)?;
    std::fs::write(&path, json).context("Failed to write license.json")?;
    Ok(())
}

/// Validate the JWT signature and expiry using the embedded public key.
/// Returns the decoded claims on success.
pub fn verify_jwt(jwt: &str) -> anyhow::Result<LicenseClaims> {
    let pem = EMBEDDED_PUBLIC_KEY
        .ok_or_else(|| anyhow::anyhow!("No license public key embedded (dev build)"))?;
    let key =
        DecodingKey::from_rsa_pem(pem.as_bytes()).context("Failed to parse embedded public key")?;

    let mut validation = Validation::new(Algorithm::RS256);
    validation.leeway = 60; // 60s clock skew tolerance

    let token_data =
        decode::<LicenseClaims>(jwt, &key, &validation).context("JWT verification failed")?;

    Ok(token_data.claims)
}

/// Determine the current license status.
///
/// Priority:
/// 1. Read `~/.sessiongraph/license.json`
/// 2. Verify JWT signature and expiry
/// 3. If valid: return licensed tier
/// 4. If missing/invalid: return free tier
pub fn get_license_status() -> LicenseStatus {
    let Some(lf) = read_license_file() else {
        return free_fallback();
    };

    match verify_jwt(&lf.key) {
        Ok(claims) => LicenseStatus {
            tier: claims.tier,
            seats: claims.seats,
            valid: true,
            expires_at: lf.expires_at,
            source: "local_jwt",
        },
        Err(e) => {
            tracing::warn!("License JWT invalid: {} — falling back to free tier", e);
            // If JWT is expired but we've phone-homed within the 7-day grace
            // period, honour the cached tier.
            if within_grace_period(&lf) {
                tracing::info!("Within 7-day offline grace period — using cached tier");
                LicenseStatus {
                    tier: lf.tier,
                    seats: lf.seats,
                    valid: true,
                    expires_at: lf.expires_at,
                    source: "cached",
                }
            } else {
                free_fallback()
            }
        }
    }
}

fn free_fallback() -> LicenseStatus {
    LicenseStatus {
        tier: "free".into(),
        seats: 1,
        valid: false,
        expires_at: None,
        source: "free_fallback",
    }
}

fn within_grace_period(lf: &LicenseFile) -> bool {
    let Some(last) = &lf.last_validated else {
        return false;
    };
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(last) {
        let age = chrono::Utc::now().signed_duration_since(dt.with_timezone(&chrono::Utc));
        return age.num_days() <= 7;
    }
    false
}

/// Phone-home to `SG_SERVER_URL/api/license/validate`. Updates `last_validated`
/// in the license file and refreshes the cached tier.
/// Non-fatal — any network error is logged and silently ignored.
pub async fn phone_home(client: &reqwest::Client) {
    let Some(lf) = read_license_file() else {
        tracing::debug!("No license file — skipping phone-home");
        return;
    };

    let server_url =
        std::env::var("SG_SERVER_URL").unwrap_or_else(|_| "https://sessiongraph.dev".into());

    let url = format!("{}/api/license/validate", server_url);

    let body = serde_json::json!({ "key": lf.key });

    let result = client
        .post(&url)
        .json(&body)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await;

    match result {
        Ok(resp) if resp.status().is_success() => {
            if let Ok(json) = resp.json::<serde_json::Value>().await {
                if json.get("valid").and_then(|v| v.as_bool()) == Some(true) {
                    // Update cached tier and last_validated
                    let tier = json
                        .get("tier")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&lf.tier)
                        .to_string();
                    let seats = json
                        .get("seats")
                        .and_then(|v| v.as_u64())
                        .map(|s| s as u32)
                        .unwrap_or(lf.seats);
                    let expires_at = json
                        .get("expiresAt")
                        .and_then(|v| v.as_str())
                        .map(String::from)
                        .or(lf.expires_at.clone());

                    let updated = LicenseFile {
                        key: lf.key,
                        tier,
                        seats,
                        expires_at,
                        last_validated: Some(chrono::Utc::now().to_rfc3339()),
                    };
                    if let Err(e) = write_license_file(&updated) {
                        tracing::warn!("Failed to update license file after phone-home: {}", e);
                    } else {
                        tracing::info!("License phone-home succeeded — tier: {}", updated.tier);
                    }
                } else {
                    tracing::warn!("License phone-home: server returned valid=false");
                }
            }
        }
        Ok(resp) => {
            tracing::warn!("License phone-home: server returned HTTP {}", resp.status());
        }
        Err(e) => {
            tracing::debug!("License phone-home failed (offline?): {}", e);
        }
    }
}
