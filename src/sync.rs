use std::process::{Child, Command, Stdio};
use std::path::Path;
use std::sync::mpsc;
use std::io::BufRead;

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
}

impl SyncClient {
    pub fn new(config: &Config) -> Self {
        Self {
            host: config.server.host.clone(),
            ssh_user: config.server.ssh_user.clone(),
            port: config.server.port,
            docs_path: config.server.docs_path.clone(),
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

    /// Start watching a remote .yrs file for changes via inotifywait.
    /// Returns a channel receiver that gets a message whenever the file changes,
    /// and the child process handle (kill it when done).
    pub fn watch_remote(
        &self,
        relative_path: &str,
    ) -> Result<(mpsc::Receiver<()>, Child), String> {
        let yrs_path = self.yrs_state_path(relative_path);

        let mut child = Command::new("ssh")
            .arg("-p").arg(self.port.to_string())
            .arg("-o").arg("BatchMode=yes")
            .arg("-o").arg("ServerAliveInterval=30")
            .arg(self.ssh_dest())
            .arg(format!(
                "inotifywait -m -e modify,create {} 2>/dev/null || \
                 while true; do inotifywait -e modify,create {} 2>/dev/null; done",
                yrs_path, yrs_path
            ))
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Failed to start inotifywait watcher: {}", e))?;

        let (tx, rx) = mpsc::channel();

        let stdout = child.stdout.take()
            .ok_or("Failed to capture watcher stdout")?;

        std::thread::spawn(move || {
            let reader = std::io::BufReader::new(stdout);
            for line in reader.lines() {
                if line.is_ok() {
                    // File was modified — notify the main thread
                    if tx.send(()).is_err() {
                        break; // receiver dropped, stop watching
                    }
                }
            }
        });

        Ok((rx, child))
    }

    /// Write a presence file on the server.
    pub fn set_presence(&self, relative_path: &str, user: &str) -> Result<(), String> {
        let presence_dir = format!("{}/.wiremd/{}.presence", self.docs_path, relative_path);
        let output = self.ssh_cmd()
            .arg(format!(
                "mkdir -p {} && echo '{}' > {}/{}",
                presence_dir,
                chrono_now(),
                presence_dir,
                user
            ))
            .output()
            .map_err(|e| format!("SSH presence failed: {}", e))?;

        if output.status.success() { Ok(()) } else { Err("Failed to set presence".into()) }
    }

    /// Remove presence file on the server.
    pub fn clear_presence(&self, relative_path: &str, user: &str) -> Result<(), String> {
        let presence_file = format!(
            "{}/.wiremd/{}.presence/{}",
            self.docs_path, relative_path, user
        );
        let _ = self.ssh_cmd()
            .arg(format!("rm -f {}", presence_file))
            .output();
        Ok(())
    }

    /// List currently present users for a file.
    pub fn list_presence(&self, relative_path: &str) -> Result<Vec<String>, String> {
        let presence_dir = format!("{}/.wiremd/{}.presence", self.docs_path, relative_path);
        let output = self.ssh_cmd()
            .arg(format!("ls -1 {} 2>/dev/null || true", presence_dir))
            .output()
            .map_err(|e| format!("SSH ls presence failed: {}", e))?;

        let users = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| l.to_string())
            .collect();
        Ok(users)
    }
}

fn chrono_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_default()
}
