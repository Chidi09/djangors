//! A [`DeployProvider`] for [Render](https://render.com), driven directly via
//! Render's REST API (`https://api.render.com/v1`) - no external CLI needed.
//! Every request shape here was validated against the real API while deploying
//! `djangors-polls` live during this framework's own development (see
//! `render.yaml` and PLAN.md's Phase 11 deployment notes): create a managed
//! Postgres database, create a Docker-runtime web service pointed at a GitHub
//! repo, trigger and poll a deploy, and fetch logs.

use super::provider::{DeployError, DeployProvider, DeploySpec, DeployStatus, DeploymentInfo};
use serde_json::{json, Value};
use std::time::Duration;

const API_BASE: &str = "https://api.render.com/v1";

/// A Render API client implementing [`DeployProvider`].
pub struct RenderProvider {
    api_key: String,
    owner_id: String,
    client: reqwest::Client,
}

impl RenderProvider {
    /// Creates a client for the given Render API key and workspace (owner) ID.
    /// Find your owner ID via [`RenderProvider::discover_owner_id`] if you
    /// don't already have it.
    pub fn new(api_key: impl Into<String>, owner_id: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            owner_id: owner_id.into(),
            client: reqwest::Client::new(),
        }
    }

    /// Looks up the first workspace (owner) ID associated with `api_key` -
    /// convenient for a single-workspace account, which covers the common
    /// case; an account with multiple workspaces should pass the right one to
    /// [`RenderProvider::new`] explicitly instead.
    pub async fn discover_owner_id(api_key: &str) -> Result<String, DeployError> {
        let client = reqwest::Client::new();
        let resp = client
            .get(format!("{API_BASE}/owners"))
            .bearer_auth(api_key)
            .send()
            .await?;
        let status = resp.status();
        let body: Value = resp.json().await?;
        if !status.is_success() {
            return Err(DeployError::Api {
                status: status.as_u16(),
                message: body.to_string(),
            });
        }
        body.as_array()
            .and_then(|owners| owners.first())
            .and_then(|o| o["owner"]["id"].as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| {
                DeployError::UnexpectedResponse("no owner found for this API key".to_string())
            })
    }

    async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, DeployError> {
        let mut req = self
            .client
            .request(method, format!("{API_BASE}{path}"))
            .bearer_auth(&self.api_key);
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send().await?;
        let status = resp.status();
        let text = resp.text().await?;
        let value: Value =
            serde_json::from_str(&text).unwrap_or_else(|_| Value::String(text.clone()));
        if !status.is_success() {
            return Err(DeployError::Api {
                status: status.as_u16(),
                message: value.to_string(),
            });
        }
        Ok(value)
    }

    fn db_name(spec: &DeploySpec) -> String {
        format!("{}-db", spec.project_name)
    }
}

#[async_trait::async_trait]
impl DeployProvider for RenderProvider {
    async fn provision(&self, spec: &DeploySpec) -> Result<DeploymentInfo, DeployError> {
        let database_id = if spec.needs_database {
            let db_body = json!({
                "name": Self::db_name(spec),
                "ownerId": self.owner_id,
                "plan": spec.plan.clone().unwrap_or_else(|| "free".to_string()),
                "version": "16",
                "databaseName": spec.project_name.replace('-', "_"),
                "region": spec.region.clone().unwrap_or_else(|| "oregon".to_string()),
            });
            let db = self
                .request(reqwest::Method::POST, "/postgres", Some(db_body))
                .await?;
            let id = db["id"]
                .as_str()
                .ok_or_else(|| {
                    DeployError::UnexpectedResponse("postgres create response missing id".into())
                })?
                .to_string();
            Some(id)
        } else {
            None
        };

        let mut env_vars: Vec<Value> = spec
            .env_vars
            .iter()
            .map(|(k, v)| json!({"key": k, "value": v}))
            .collect();
        if let Some(db_id) = &database_id {
            let conn = self
                .request(
                    reqwest::Method::GET,
                    &format!("/postgres/{db_id}/connection-info"),
                    None,
                )
                .await?;
            let internal_url = conn["internalConnectionString"].as_str().ok_or_else(|| {
                DeployError::UnexpectedResponse(
                    "postgres connection-info missing internalConnectionString".into(),
                )
            })?;
            env_vars.push(json!({"key": "DATABASE_URL", "value": internal_url}));
        }

        let service_body = json!({
            "type": "web_service",
            "name": spec.project_name,
            "ownerId": self.owner_id,
            "repo": spec.repo_url,
            "branch": spec.branch,
            "autoDeploy": "yes",
            "envVars": env_vars,
            "serviceDetails": {
                "env": "docker",
                "runtime": "docker",
                "envSpecificDetails": {
                    "dockerCommand": "",
                    "dockerContext": spec.docker_context,
                    "dockerfilePath": spec.dockerfile_path,
                },
                "healthCheckPath": spec.health_check_path,
                "plan": spec.plan.clone().unwrap_or_else(|| "free".to_string()),
                "region": spec.region.clone().unwrap_or_else(|| "oregon".to_string()),
            },
        });
        let created = self
            .request(reqwest::Method::POST, "/services", Some(service_body))
            .await?;
        let service_id = created["service"]["id"]
            .as_str()
            .ok_or_else(|| {
                DeployError::UnexpectedResponse("service create response missing service.id".into())
            })?
            .to_string();
        let url = created["service"]["serviceDetails"]["url"]
            .as_str()
            .map(|s| s.to_string());

        Ok(DeploymentInfo {
            provider: "render".to_string(),
            service_id,
            database_id,
            url,
        })
    }

    async fn deploy(&self, info: &DeploymentInfo, _spec: &DeploySpec) -> Result<(), DeployError> {
        let deploy = self
            .request(
                reqwest::Method::POST,
                &format!("/services/{}/deploys", info.service_id),
                Some(json!({})),
            )
            .await?;
        let deploy_id = deploy["id"].as_str().ok_or_else(|| {
            DeployError::UnexpectedResponse("deploy response missing id".to_string())
        })?;

        // Poll until a terminal state, mirroring the real polling loop used to
        // deploy djangors-polls live: build/update in progress can legitimately
        // take several minutes for a Rust release build.
        let max_polls = 60;
        let poll_interval = Duration::from_secs(15);
        for _ in 0..max_polls {
            let current = self
                .request(
                    reqwest::Method::GET,
                    &format!("/services/{}/deploys/{deploy_id}", info.service_id),
                    None,
                )
                .await?;
            let status = current["status"].as_str().unwrap_or("");
            match status {
                "live" => return Ok(()),
                "build_failed" | "update_failed" | "canceled" | "deactivated"
                | "pre_deploy_failed" => {
                    return Err(DeployError::Api {
                        status: 0,
                        message: format!("deploy ended in status `{status}`"),
                    })
                }
                _ => tokio::time::sleep(poll_interval).await,
            }
        }
        Err(DeployError::Timeout(format!(
            "deploy of service {} to reach a terminal state",
            info.service_id
        )))
    }

    async fn status(&self, info: &DeploymentInfo) -> Result<DeployStatus, DeployError> {
        let deploys = self
            .request(
                reqwest::Method::GET,
                &format!("/services/{}/deploys?limit=1", info.service_id),
                None,
            )
            .await;
        let deploys = match deploys {
            Ok(v) => v,
            Err(DeployError::Api { status: 404, .. }) => return Ok(DeployStatus::NotFound),
            Err(e) => return Err(e),
        };
        let latest = deploys
            .as_array()
            .and_then(|a| a.first())
            .and_then(|d| d["deploy"]["status"].as_str())
            .unwrap_or("");
        Ok(match latest {
            "live" => DeployStatus::Live,
            "build_failed" | "update_failed" | "canceled" | "deactivated" | "pre_deploy_failed" => {
                DeployStatus::Failed(latest.to_string())
            }
            "" => DeployStatus::NotFound,
            _ => DeployStatus::InProgress,
        })
    }

    async fn logs(&self, info: &DeploymentInfo, lines: u32) -> Result<String, DeployError> {
        let resp = self
            .request(
                reqwest::Method::GET,
                &format!(
                    "/logs?ownerId={}&resource={}&limit={lines}&direction=backward",
                    self.owner_id, info.service_id
                ),
                None,
            )
            .await?;
        let entries = resp["logs"].as_array().cloned().unwrap_or_default();
        let mut lines_out: Vec<String> = entries
            .iter()
            .map(|l| {
                let ts = l["timestamp"].as_str().unwrap_or("");
                let msg = l["message"].as_str().unwrap_or("");
                format!("{ts} {msg}")
            })
            .collect();
        lines_out.reverse();
        Ok(lines_out.join("\n"))
    }

    async fn destroy(&self, info: &DeploymentInfo) -> Result<(), DeployError> {
        self.request(
            reqwest::Method::DELETE,
            &format!("/services/{}", info.service_id),
            None,
        )
        .await?;
        if let Some(db_id) = &info.database_id {
            self.request(reqwest::Method::DELETE, &format!("/postgres/{db_id}"), None)
                .await?;
        }
        Ok(())
    }
}
