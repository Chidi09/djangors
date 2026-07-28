use serde::{Deserialize, Serialize};

/// Everything a [`DeployProvider`] needs to provision and deploy a project.
///
/// Deliberately covers both deployment models this framework's own example
/// apps actually use: GitOps PaaS providers (Render, Railway - build from a
/// repo + Dockerfile on every push) via `repo_url`/`branch`/`dockerfile_path`,
/// and raw-VPS providers (SSH) via `docker_context` and a locally-built image.
/// A provider implementation only reads the fields it actually needs.
#[derive(Debug, Clone)]
pub struct DeploySpec {
    /// A short, URL-safe name identifying this deployment (used to derive
    /// service/database names with most providers).
    pub project_name: String,
    /// The Git repository URL to build from (GitOps providers).
    pub repo_url: Option<String>,
    /// The branch to build and deploy from.
    pub branch: String,
    /// Path to the Dockerfile, relative to `docker_context`.
    pub dockerfile_path: String,
    /// The Docker build context directory (relative to the repo root).
    pub docker_context: String,
    /// HTTP path the provider should poll to determine the deployed service is
    /// actually healthy and ready to receive traffic (e.g. `/healthz`).
    pub health_check_path: String,
    /// Environment variables to set on the deployed service.
    pub env_vars: Vec<(String, String)>,
    /// Provider-specific region identifier (e.g. Render's `oregon`); `None`
    /// lets the provider pick its own default.
    pub region: Option<String>,
    /// Provider-specific plan/tier identifier (e.g. Render's `free`/`starter`);
    /// `None` lets the provider pick its own default (usually the free tier,
    /// when one exists).
    pub plan: Option<String>,
    /// Whether this project also needs a managed Postgres database
    /// provisioned and wired into `DATABASE_URL` automatically.
    pub needs_database: bool,
}

/// A provider-returned identifier for an already-provisioned deployment,
/// opaque to callers beyond what's needed to check on or redeploy it later.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploymentInfo {
    /// Which provider this deployment lives on (`"render"`, `"railway"`, `"ssh"`, ...).
    pub provider: String,
    /// The provider's own identifier for the deployed service.
    pub service_id: String,
    /// The provider's own identifier for the backing database, if one was provisioned.
    pub database_id: Option<String>,
    /// The public URL the deployed service is reachable at, once live.
    pub url: Option<String>,
}

/// The current state of a deployment, as reported by [`DeployProvider::status`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeployStatus {
    /// A build or deploy is currently in progress.
    InProgress,
    /// The deployment is live and (where the provider reports this) has passed
    /// its health check.
    Live,
    /// The build or deploy failed. The string is the provider's own
    /// human-readable failure reason where available.
    Failed(String),
    /// The provider reports no deployment matching this `DeploymentInfo`
    /// exists (e.g. it was destroyed through the provider's own dashboard).
    NotFound,
}

/// Errors from any [`DeployProvider`] operation.
#[derive(thiserror::Error, Debug)]
pub enum DeployError {
    /// The underlying HTTP request to the provider's API failed.
    #[error("request to provider API failed: {0}")]
    Request(#[from] reqwest::Error),
    /// The provider's API responded with an error status and message.
    #[error("provider API error ({status}): {message}")]
    Api {
        /// The HTTP status code returned.
        status: u16,
        /// The provider's own error message, where one was included.
        message: String,
    },
    /// The provider's response didn't contain a field this client expected.
    #[error("unexpected response from provider: {0}")]
    UnexpectedResponse(String),
    /// A deploy or provisioning operation exceeded its polling timeout.
    #[error("timed out waiting for {0}")]
    Timeout(String),
}

/// A deployment target: Render, Railway, a raw VPS over SSH, GCP, AWS, and so
/// on. Each implementation only needs to make sense of the [`DeploySpec`]
/// fields relevant to how that provider actually works.
#[async_trait::async_trait]
pub trait DeployProvider: Send + Sync {
    /// Provisions whatever backing infrastructure this deployment needs (a
    /// managed database if `spec.needs_database`, the service/VM record
    /// itself) and returns a [`DeploymentInfo`] identifying it for future
    /// calls. Idempotent where the provider's own API allows it; otherwise
    /// callers should persist the returned `DeploymentInfo` and reuse it
    /// rather than calling `provision` again.
    async fn provision(&self, spec: &DeploySpec) -> Result<DeploymentInfo, DeployError>;

    /// Triggers a deploy of the current `spec` (a fresh build from the
    /// configured repo/branch for a GitOps provider, or pushing a freshly
    /// built image for a raw-VPS provider) against an already-provisioned
    /// deployment.
    async fn deploy(&self, info: &DeploymentInfo, spec: &DeploySpec) -> Result<(), DeployError>;

    /// Reports the current state of a deployment.
    async fn status(&self, info: &DeploymentInfo) -> Result<DeployStatus, DeployError>;

    /// Fetches the most recent `lines` of the deployed service's logs.
    async fn logs(&self, info: &DeploymentInfo, lines: u32) -> Result<String, DeployError>;

    /// Tears down the deployment (and its database, if one was provisioned).
    /// A real, destructive, billable-resource-removing action - callers
    /// should always confirm with a human before invoking this.
    async fn destroy(&self, info: &DeploymentInfo) -> Result<(), DeployError>;
}
