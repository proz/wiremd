use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub user: UserConfig,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ServerConfig {
    /// SSH host (e.g., "git.proz.ovh")
    pub host: String,
    /// SSH user (e.g., "ubuntu")
    pub ssh_user: String,
    /// SSH port (default 22)
    #[serde(default = "default_port")]
    pub port: u16,
    /// Path on the server where docs are stored (e.g., "/srv/docs")
    pub docs_path: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UserConfig {
    /// Display name for this user
    pub name: String,
}

fn default_port() -> u16 {
    22
}

impl Config {
    /// Load config from ~/.config/wiremd/config.toml
    pub fn load() -> Result<Self, String> {
        let path = config_path();
        if !path.exists() {
            return Err(format!(
                "Config not found at {}. Run `wiremd --init` to create one.",
                path.display()
            ));
        }
        let content = fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
        toml::from_str(&content)
            .map_err(|e| format!("Failed to parse {}: {}", path.display(), e))
    }

    /// Create a default config file
    pub fn init() -> Result<PathBuf, String> {
        let path = config_path();
        if path.exists() {
            return Err(format!("Config already exists at {}", path.display()));
        }

        let dir = path.parent().unwrap();
        fs::create_dir_all(dir)
            .map_err(|e| format!("Failed to create {}: {}", dir.display(), e))?;

        let default = Config {
            server: ServerConfig {
                host: "example.com".to_string(),
                ssh_user: "user".to_string(),
                port: 22,
                docs_path: "/srv/docs".to_string(),
            },
            user: UserConfig {
                name: whoami().unwrap_or_else(|| "anonymous".to_string()),
            },
        };

        let content = toml::to_string_pretty(&default)
            .map_err(|e| format!("Failed to serialize config: {}", e))?;
        fs::write(&path, content)
            .map_err(|e| format!("Failed to write {}: {}", path.display(), e))?;

        Ok(path)
    }
}

fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("~/.config"))
        .join("wiremd")
        .join("config.toml")
}

fn whoami() -> Option<String> {
    std::env::var("USER").ok().or_else(|| std::env::var("USERNAME").ok())
}
