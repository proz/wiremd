use std::process::Command;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::Config;

/// Remote path structure:
///   {docs_path}/{relative_path}          — the markdown file
///   {docs_path}/.wiremd/{relative_path}.yrs  — yrs doc state
///   {docs_path}/.wiremd/{relative_path}.updates/{user}_{timestamp} — pending deltas

pub struct SyncClient {
    host: String,
    ssh_user: String,
    port: u16,
    docs_path: String,
    user_name: String,
}

impl SyncClient {
    pub fn new(config: &Config) -> Self {
        Self {
            host: config.server.host.clone(),
            ssh_user: config.server.ssh_user.clone(),
            port: config.server.port,
            docs_path: config.server.docs_path.clone(),
            user_name: config.user.name.clone(),
        }
    }

    fn ssh_dest(&self) -> String {
        format!("{}@{}", self.ssh_user, self.host)
    }

    fn ssh_cmd(&self) -> Command {
        let mut cmd = Command::new("ssh");
        cmd.arg("-p").arg(self.port.to_string());
        cmd.arg("-o").arg("BatchMode=yes");
        cmd.arg("-o").arg("ConnectTimeout=5");
        cmd.arg(self.ssh_dest());
        cmd
    }

    fn scp_cmd(&self) -> Command {
        let mut cmd = Command::new("scp");
        cmd.arg("-P").arg(self.port.to_string());
        cmd.arg("-o").arg("BatchMode=yes");
        cmd.arg("-o").arg("ConnectTimeout=5");
        cmd
    }

    /// Remote path for the yrs state file
    fn yrs_state_path(&self, relative_path: &str) -> String {
        format!("{}/.wiremd/{}.yrs", self.docs_path, relative_path)
    }

    /// Remote dir for update deltas
    fn updates_dir(&self, relative_path: &str) -> String {
        format!("{}/.wiremd/{}.updates", self.docs_path, relative_path)
    }

    /// Test SSH connectivity
    pub fn test_connection(&self) -> Result<(), String> {
        let output = self.ssh_cmd()
            .arg("echo ok")
            .output()
            .map_err(|e| format!("SSH failed: {}", e))?;

        if output.status.success() {
            Ok(())
        } else {
            Err(format!(
                "SSH connection failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ))
        }
    }

    /// Ensure remote directories exist for a given file
    pub fn ensure_remote_dirs(&self, relative_path: &str) -> Result<(), String> {
        let updates_dir = self.updates_dir(relative_path);
        let output = self.ssh_cmd()
            .arg(format!("mkdir -p {}", updates_dir))
            .output()
            .map_err(|e| format!("SSH mkdir failed: {}", e))?;

        if output.status.success() {
            Ok(())
        } else {
            Err(format!(
                "Failed to create remote dirs: {}",
                String::from_utf8_lossy(&output.stderr)
            ))
        }
    }

    /// Push a yrs update delta to the server
    pub fn push_update(&self, relative_path: &str, update: &[u8]) -> Result<(), String> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let filename = format!("{}_{}", self.user_name, timestamp);
        let remote_path = format!(
            "{}:{}",
            self.ssh_dest(),
            format!("{}/{}", self.updates_dir(relative_path), filename)
        );

        // Write update to a temp file, then scp it
        let tmp = std::env::temp_dir().join(format!("wiremd_update_{}", timestamp));
        std::fs::write(&tmp, update)
            .map_err(|e| format!("Failed to write temp file: {}", e))?;

        let output = self.scp_cmd()
            .arg(tmp.to_str().unwrap())
            .arg(&remote_path)
            .output()
            .map_err(|e| format!("SCP push failed: {}", e))?;

        let _ = std::fs::remove_file(&tmp);

        if output.status.success() {
            Ok(())
        } else {
            Err(format!(
                "SCP push failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ))
        }
    }

    /// Push multiple updates as a single merged blob
    pub fn push_updates(&self, relative_path: &str, updates: &[Vec<u8>]) -> Result<(), String> {
        if updates.is_empty() {
            return Ok(());
        }
        // Merge all updates into one
        let merged = yrs_merge_updates(updates)?;
        self.push_update(relative_path, &merged)
    }

    /// Pull all pending update deltas from the server
    pub fn pull_updates(&self, relative_path: &str) -> Result<Vec<Vec<u8>>, String> {
        let updates_dir = self.updates_dir(relative_path);

        // List files in updates dir
        let output = self.ssh_cmd()
            .arg(format!("ls -1 {} 2>/dev/null || true", updates_dir))
            .output()
            .map_err(|e| format!("SSH ls failed: {}", e))?;

        let file_list = String::from_utf8_lossy(&output.stdout);
        let files: Vec<&str> = file_list.lines().filter(|l| !l.is_empty()).collect();

        if files.is_empty() {
            return Ok(Vec::new());
        }

        let mut updates = Vec::new();
        let tmp_dir = std::env::temp_dir().join("wiremd_pull");
        let _ = std::fs::create_dir_all(&tmp_dir);

        for file in &files {
            let remote = format!(
                "{}:{}/{}",
                self.ssh_dest(),
                updates_dir,
                file
            );
            let local = tmp_dir.join(file);

            let output = self.scp_cmd()
                .arg(&remote)
                .arg(local.to_str().unwrap())
                .output()
                .map_err(|e| format!("SCP pull failed: {}", e))?;

            if output.status.success() {
                if let Ok(data) = std::fs::read(&local) {
                    updates.push(data);
                }
                let _ = std::fs::remove_file(&local);
            }
        }

        let _ = std::fs::remove_dir(&tmp_dir);
        Ok(updates)
    }

    /// Clear pulled updates from the server (after successfully applying them)
    pub fn clear_updates(&self, relative_path: &str) -> Result<(), String> {
        let updates_dir = self.updates_dir(relative_path);
        let output = self.ssh_cmd()
            .arg(format!("rm -f {}/* 2>/dev/null || true", updates_dir))
            .output()
            .map_err(|e| format!("SSH rm failed: {}", e))?;

        if output.status.success() {
            Ok(())
        } else {
            Err(format!(
                "Failed to clear remote updates: {}",
                String::from_utf8_lossy(&output.stderr)
            ))
        }
    }

    /// Push the full yrs state snapshot to the server
    pub fn push_state(&self, relative_path: &str, state: &[u8]) -> Result<(), String> {
        let remote_path = format!(
            "{}:{}",
            self.ssh_dest(),
            self.yrs_state_path(relative_path)
        );

        let tmp = std::env::temp_dir().join("wiremd_state");
        std::fs::write(&tmp, state)
            .map_err(|e| format!("Failed to write temp file: {}", e))?;

        let output = self.scp_cmd()
            .arg(tmp.to_str().unwrap())
            .arg(&remote_path)
            .output()
            .map_err(|e| format!("SCP push state failed: {}", e))?;

        let _ = std::fs::remove_file(&tmp);

        if output.status.success() {
            Ok(())
        } else {
            Err(format!(
                "SCP push state failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ))
        }
    }

    /// Pull the full yrs state snapshot from the server
    pub fn pull_state(&self, relative_path: &str) -> Result<Option<Vec<u8>>, String> {
        let remote_path = format!(
            "{}:{}",
            self.ssh_dest(),
            self.yrs_state_path(relative_path)
        );

        let tmp = std::env::temp_dir().join("wiremd_state_pull");

        let output = self.scp_cmd()
            .arg(&remote_path)
            .arg(tmp.to_str().unwrap())
            .output()
            .map_err(|e| format!("SCP pull state failed: {}", e))?;

        if output.status.success() {
            let data = std::fs::read(&tmp)
                .map_err(|e| format!("Failed to read pulled state: {}", e))?;
            let _ = std::fs::remove_file(&tmp);
            Ok(Some(data))
        } else {
            // File doesn't exist on server — that's OK
            Ok(None)
        }
    }

    /// Push the markdown file itself to the server
    pub fn push_file(&self, relative_path: &str, content: &str) -> Result<(), String> {
        let remote_path = format!(
            "{}:{}/{}",
            self.ssh_dest(),
            self.docs_path,
            relative_path
        );

        let tmp = std::env::temp_dir().join("wiremd_file");
        std::fs::write(&tmp, content)
            .map_err(|e| format!("Failed to write temp file: {}", e))?;

        // Ensure parent dir exists
        let parent = Path::new(relative_path).parent().map(|p| p.to_str().unwrap_or(""));
        if let Some(parent) = parent {
            if !parent.is_empty() {
                let _ = self.ssh_cmd()
                    .arg(format!("mkdir -p {}/{}", self.docs_path, parent))
                    .output();
            }
        }

        let output = self.scp_cmd()
            .arg(tmp.to_str().unwrap())
            .arg(&remote_path)
            .output()
            .map_err(|e| format!("SCP push file failed: {}", e))?;

        let _ = std::fs::remove_file(&tmp);

        if output.status.success() {
            Ok(())
        } else {
            Err(format!(
                "SCP push file failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ))
        }
    }
    /// List markdown files on the remote server. Returns relative paths sorted.
    pub fn list_remote_files(&self) -> Result<Vec<String>, String> {
        let output = self.ssh_cmd()
            .arg(format!(
                "find {} -name '*.md' -type f -printf '%P\\n' 2>/dev/null | sort",
                self.docs_path
            ))
            .output()
            .map_err(|e| format!("SSH find failed: {}", e))?;

        if !output.status.success() {
            return Err(format!(
                "Failed to list remote files: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        let files = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| l.to_string())
            .collect();
        Ok(files)
    }

    /// Read a file from the remote server.
    pub fn read_remote_file(&self, relative_path: &str) -> Result<String, String> {
        let output = self.ssh_cmd()
            .arg(format!("cat {}/{}", self.docs_path, relative_path))
            .output()
            .map_err(|e| format!("SSH cat failed: {}", e))?;

        if output.status.success() {
            String::from_utf8(output.stdout)
                .map_err(|e| format!("Invalid UTF-8 in remote file: {}", e))
        } else {
            Err(format!(
                "Failed to read remote file: {}",
                String::from_utf8_lossy(&output.stderr)
            ))
        }
    }

    /// Get the remote docs_path (for display).
    pub fn docs_path(&self) -> &str {
        &self.docs_path
    }

    /// Get the host (for display).
    pub fn host(&self) -> &str {
        &self.host
    }
}

/// Merge multiple yrs updates into one
fn yrs_merge_updates(updates: &[Vec<u8>]) -> Result<Vec<u8>, String> {
    use yrs::updates::decoder::Decode;
    use yrs::updates::encoder::Encode;
    use yrs::Update;

    let parsed: Result<Vec<Update>, _> = updates.iter().map(|u| Update::decode_v1(u)).collect();
    let parsed = parsed.map_err(|e| format!("Failed to decode updates: {}", e))?;

    let merged = Update::merge_updates(parsed);

    Ok(merged.encode_v1())
}
