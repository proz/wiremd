mod browser;
mod config;
mod editor;
mod sync;

use std::env;
use std::fs;
use std::io::{self, stdout};

use crossterm::{
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{prelude::CrosstermBackend, Terminal};

fn main() -> io::Result<()> {
    let args: Vec<String> = env::args().collect();

    // wiremd --init
    if args.get(1).map(|s| s.as_str()) == Some("--init") {
        match config::Config::init() {
            Ok(path) => {
                eprintln!("Config created at {}", path.display());
                eprintln!("Edit it to set your server details.");
            }
            Err(e) => eprintln!("Error: {}", e),
        }
        return Ok(());
    }

    // wiremd --remote <file>
    if args.get(1).map(|s| s.as_str()) == Some("--remote") {
        let file = args.get(2).unwrap_or_else(|| {
            eprintln!("Usage: wiremd --remote <file.md>");
            std::process::exit(1);
        });
        return run_remote_file(file);
    }

    // wiremd <file.md> — local mode
    if args.len() >= 2 && !args[1].starts_with('-') {
        let path = &args[1];
        return run_local(path);
    }

    // wiremd (no args) — remote browser mode
    if args.len() == 1 {
        return run_browser();
    }

    eprintln!("Usage: wiremd [file.md]");
    eprintln!("       wiremd --remote <file.md>   Open remote file");
    eprintln!("       wiremd --init                Create config");
    eprintln!("       wiremd                        Browse remote docs");
    Ok(())
}

/// Local mode: edit a local file, optionally sync if config exists
fn run_local(path: &str) -> io::Result<()> {
    let content = fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("Error reading {}: {}", path, e);
        std::process::exit(1);
    });

    let sync_client = try_connect();

    let relative_path = std::path::Path::new(path)
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or(path)
        .to_string();

    let mut editor = editor::Editor::new(
        path.to_string(),
        content,
        sync_client,
        relative_path,
    );

    let mut terminal = setup_terminal()?;
    let result = editor.run(&mut terminal);
    teardown_terminal()?;
    result
}

/// Remote mode: open a specific file from the server
fn run_remote_file(relative_path: &str) -> io::Result<()> {
    let cfg = config::Config::load().unwrap_or_else(|e| {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    });

    let client = sync::SyncClient::new(&cfg);
    if let Err(e) = client.test_connection() {
        eprintln!("Cannot connect to server: {}", e);
        std::process::exit(1);
    }

    let content = client.read_remote_file(relative_path).unwrap_or_else(|e| {
        eprintln!("Error reading remote file: {}", e);
        std::process::exit(1);
    });

    // Write to a local cache for editing
    let cache_dir = dirs::cache_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join("wiremd");
    let _ = fs::create_dir_all(&cache_dir);
    let local_path = cache_dir.join(relative_path.replace('/', "_"));
    fs::write(&local_path, &content)?;

    let mut editor = editor::Editor::new(
        local_path.to_string_lossy().to_string(),
        content,
        Some(client),
        relative_path.to_string(),
    );

    let mut terminal = setup_terminal()?;
    let result = editor.run(&mut terminal);
    teardown_terminal()?;
    result
}

/// Browser mode: browse remote docs, select a file, open in editor
fn run_browser() -> io::Result<()> {
    let cfg = config::Config::load().unwrap_or_else(|e| {
        eprintln!("Error: {}. Run `wiremd --init` first.", e);
        std::process::exit(1);
    });

    let client = sync::SyncClient::new(&cfg);
    if let Err(e) = client.test_connection() {
        eprintln!("Cannot connect to server: {}", e);
        std::process::exit(1);
    }

    let mut browser = browser::Browser::new(client).unwrap_or_else(|e| {
        eprintln!("Error listing remote files: {}", e);
        std::process::exit(1);
    });

    let mut terminal = setup_terminal()?;

    loop {
        match browser.run(&mut terminal)? {
            Some(relative_path) => {
                // Fetch file content
                let content = match browser.fetch_file(&relative_path) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("Error: {}", e);
                        continue;
                    }
                };

                // Write to local cache
                let cache_dir = dirs::cache_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
                    .join("wiremd");
                let _ = fs::create_dir_all(&cache_dir);
                let local_path = cache_dir.join(relative_path.replace('/', "_"));
                fs::write(&local_path, &content)?;

                // Create a temporary SyncClient for the editor
                let cfg = config::Config::load().unwrap();
                let editor_client = sync::SyncClient::new(&cfg);

                let mut editor = editor::Editor::new(
                    local_path.to_string_lossy().to_string(),
                    content,
                    Some(editor_client),
                    relative_path,
                );

                editor.run(&mut terminal)?;
                // After editor exits, loop back to browser
            }
            None => break,
        }
    }

    teardown_terminal()?;
    Ok(())
}

/// Try to connect to server from config, return None if unavailable
fn try_connect() -> Option<sync::SyncClient> {
    let cfg = config::Config::load().ok()?;
    let client = sync::SyncClient::new(&cfg);
    match client.test_connection() {
        Ok(_) => Some(client),
        Err(e) => {
            eprintln!("Warning: server not reachable ({}). Working offline.", e);
            None
        }
    }
}

fn setup_terminal() -> io::Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    Terminal::new(CrosstermBackend::new(stdout()))
}

fn teardown_terminal() -> io::Result<()> {
    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    Ok(())
}
