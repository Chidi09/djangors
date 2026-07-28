//! A [`DeployProvider`] for a raw VPS reachable over SSH, shelling out to the
//! system's own `ssh` binary (via `tokio::process::Command`) rather than
//! embedding an SSH protocol implementation - this avoids adding a native SSH
//! library dependency (`ssh2`/libssh2 need a C library and `pkg-config`, the
//! exact class of build pain already hit once this session deploying to
//! Render) and matches how real deployment tools (Ansible, Capistrano) work in
//! practice: `ssh`/`scp` are already installed and battle-tested wherever a
//! developer or CI runner already deploys from.
//!
//! Builds and runs directly on the remote host from a `git clone`/`git pull`
//! of `spec.repo_url`, the same Dockerfile-based flow the Render provider
//! uses, rather than building a local image and transferring a tarball -
//! simpler, and avoids shipping potentially large image layers over the
//! network on every deploy.

use super::provider::{DeployError, DeployProvider, DeploySpec, DeployStatus, DeploymentInfo};
use std::path::PathBuf;

/// An SSH-reachable VPS target implementing [`DeployProvider`].
pub struct SshProvider {
    host: String,
    port: u16,
    user: String,
    key_path: PathBuf,
}

/// Where deployed projects live on the remote host.
const REMOTE_BASE_DIR: &str = "/opt/djangors-deploys";

impl SshProvider {
    /// Creates a provider targeting `user@host:port`, authenticating with the
    /// private key at `key_path`.
    pub fn new(
        host: impl Into<String>,
        port: u16,
        user: impl Into<String>,
        key_path: impl Into<PathBuf>,
    ) -> Self {
        Self {
            host: host.into(),
            port,
            user: user.into(),
            key_path: key_path.into(),
        }
    }

    fn ssh_command(&self) -> tokio::process::Command {
        let mut cmd = tokio::process::Command::new("ssh");
        cmd.args([
            "-i",
            &self.key_path.to_string_lossy(),
            "-o",
            "BatchMode=yes",
            "-o",
            "StrictHostKeyChecking=accept-new",
            "-p",
            &self.port.to_string(),
            &format!("{}@{}", self.user, self.host),
        ]);
        cmd
    }

    /// Runs `remote_cmd` (already a complete, properly-quoted shell command
    /// string - see [`shell_quote`]) on the remote host and returns
    /// `(stdout, stderr, exit_code)`.
    async fn exec(&self, remote_cmd: &str) -> Result<(String, String, i32), DeployError> {
        let output = self
            .ssh_command()
            .arg(remote_cmd)
            .output()
            .await
            .map_err(|e| DeployError::UnexpectedResponse(format!("failed to run ssh: {e}")))?;
        Ok((
            String::from_utf8_lossy(&output.stdout).to_string(),
            String::from_utf8_lossy(&output.stderr).to_string(),
            output.status.code().unwrap_or(-1),
        ))
    }

    async fn exec_checked(&self, remote_cmd: &str) -> Result<String, DeployError> {
        let (stdout, stderr, code) = self.exec(remote_cmd).await?;
        if code != 0 {
            return Err(DeployError::Api {
                status: 0,
                message: format!("remote command exited {code}: {stderr}"),
            });
        }
        Ok(stdout)
    }

    /// The raw (unquoted) remote directory path for `project_name`. Callers
    /// must quote the *complete* string with [`shell_quote`] at the point
    /// it's embedded as a shell argument - quoting `project_name` alone here
    /// and concatenating it into a larger path would leave a stray quote
    /// character in the middle of the path, which is exactly the kind of bug
    /// this module's own tests exist to catch.
    fn remote_dir(project_name: &str) -> Result<String, DeployError> {
        validate_slug(project_name)?;
        Ok(format!("{REMOTE_BASE_DIR}/{project_name}"))
    }
}

/// Rejects anything that isn't a safe, simple path/container-name segment:
/// used for `project_name`, since it becomes a directory name, a Docker
/// container name, and part of an image tag - all places where `/`, `..`, or
/// shell metacharacters would be a real problem even though every value is
/// also passed through [`shell_quote`] as defense in depth.
fn validate_slug(s: &str) -> Result<(), DeployError> {
    let valid = !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if valid {
        Ok(())
    } else {
        Err(DeployError::UnexpectedResponse(format!(
            "invalid project name `{s}` - must be non-empty alphanumeric/-/_ only"
        )))
    }
}

/// POSIX single-quote shell escaping: wraps `s` in single quotes, escaping any
/// embedded single quote as `'\''`. Every value interpolated into a remote
/// command string in this module goes through this first - `spec.env_vars`
/// values and the project name are not trusted to be shell-metacharacter-free
/// (an env var value could legitimately contain `$`, `;`, backticks, etc.).
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[async_trait::async_trait]
impl DeployProvider for SshProvider {
    async fn provision(&self, spec: &DeploySpec) -> Result<DeploymentInfo, DeployError> {
        // Nothing to provision on an existing VPS beyond confirming it's
        // actually reachable and has Docker - unlike a PaaS provider, there's
        // no "create the VM" step here, the VM already exists.
        let docker_version = self.exec_checked("docker --version").await?;
        if !docker_version.to_lowercase().contains("docker") {
            return Err(DeployError::UnexpectedResponse(
                "docker does not appear to be installed on the remote host".to_string(),
            ));
        }
        Ok(DeploymentInfo {
            provider: "ssh".to_string(),
            service_id: spec.project_name.clone(),
            database_id: None,
            url: None,
        })
    }

    async fn deploy(&self, info: &DeploymentInfo, spec: &DeploySpec) -> Result<(), DeployError> {
        let repo = spec.repo_url.as_ref().ok_or_else(|| {
            DeployError::UnexpectedResponse("SshProvider requires spec.repo_url".to_string())
        })?;
        let remote_dir = Self::remote_dir(&info.service_id)?;
        let remote_dir_q = shell_quote(&remote_dir);
        let branch = shell_quote(&spec.branch);
        let repo_q = shell_quote(repo);

        // Clone on first deploy, hard-reset to the target branch on every
        // subsequent one - matches a GitOps provider's own "always deploy
        // exactly what's on the branch" semantics, not an accumulating local
        // checkout that could drift.
        let sync_cmd = format!(
            "mkdir -p {} && \
             (git -C {remote_dir_q} rev-parse --git-dir > /dev/null 2>&1 && \
              git -C {remote_dir_q} fetch origin {branch} && \
              git -C {remote_dir_q} reset --hard origin/{branch} || \
              git clone --branch {branch} {repo_q} {remote_dir_q})",
            shell_quote(REMOTE_BASE_DIR)
        );
        self.exec_checked(&sync_cmd).await?;

        let image_tag = format!("djangors-{}", info.service_id);
        // Full (unquoted) paths are built by simple concatenation first, then
        // the *complete* path is quoted once as a single shell argument -
        // quoting `remote_dir` on its own and concatenating further onto it
        // would leave a stray quote character in the middle of the path.
        let dockerfile = format!("{remote_dir}/{}", spec.dockerfile_path);
        let context = format!("{remote_dir}/{}", spec.docker_context);
        let build_cmd = format!(
            "docker build -f {} -t {} {}",
            shell_quote(&dockerfile),
            shell_quote(&image_tag),
            shell_quote(&context)
        );
        self.exec_checked(&build_cmd).await?;

        let env_flags: String = spec
            .env_vars
            .iter()
            .map(|(k, v)| format!("-e {}={}", shell_quote(k), shell_quote(v)))
            .collect::<Vec<_>>()
            .join(" ");
        let container_name = shell_quote(&info.service_id);
        let run_cmd = format!(
            "docker stop {container_name} > /dev/null 2>&1; \
             docker rm {container_name} > /dev/null 2>&1; \
             docker run -d --name {container_name} --restart unless-stopped \
             --add-host=host.docker.internal:host-gateway \
             -p 8000:8000 {env_flags} {}",
            shell_quote(&image_tag)
        );
        self.exec_checked(&run_cmd).await?;
        Ok(())
    }

    async fn status(&self, info: &DeploymentInfo) -> Result<DeployStatus, DeployError> {
        let container_name = shell_quote(&info.service_id);
        let (stdout, _, code) = self
            .exec(&format!(
                "docker inspect -f '{{{{.State.Status}}}}' {container_name}"
            ))
            .await?;
        if code != 0 {
            return Ok(DeployStatus::NotFound);
        }
        Ok(match stdout.trim() {
            "running" => DeployStatus::Live,
            "restarting" | "created" => DeployStatus::InProgress,
            other => DeployStatus::Failed(other.to_string()),
        })
    }

    async fn logs(&self, info: &DeploymentInfo, lines: u32) -> Result<String, DeployError> {
        let container_name = shell_quote(&info.service_id);
        self.exec_checked(&format!("docker logs --tail {lines} {container_name} 2>&1"))
            .await
    }

    async fn destroy(&self, info: &DeploymentInfo) -> Result<(), DeployError> {
        let container_name = shell_quote(&info.service_id);
        // Best-effort: a container that's already gone shouldn't make destroy
        // itself an error.
        let _ = self
            .exec(&format!(
                "docker stop {container_name}; docker rm {container_name}"
            ))
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_quote_neutralizes_every_dangerous_character() {
        assert_eq!(shell_quote("hello"), "'hello'");
        assert_eq!(shell_quote("it's"), r#"'it'\''s'"#);
        assert_eq!(shell_quote("$(rm -rf /)"), "'$(rm -rf /)'");
        assert_eq!(shell_quote("a; rm -rf /"), "'a; rm -rf /'");
        assert_eq!(shell_quote("`whoami`"), "'`whoami`'");
    }
}
