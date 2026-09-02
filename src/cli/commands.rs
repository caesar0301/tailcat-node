//! CLI command implementations.

use crate::cli::{Cli, Command, PeerCommand, ServiceCommand};
use crate::config::Config;
use crate::daemon::Daemon;
use crate::error::{Error, Result};
use crate::identity::IdentityStore;
use crate::ipc::IpcClient;
use crate::peer::{Peer, PeerRegistry};
use crate::service::{Service, ServiceRegistry};
use clap::Parser;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Wrapper around libc kill(2) for sending signals.
#[cfg(unix)]
unsafe fn libc_kill(pid: i32, sig: i32) -> i32 {
    extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    kill(pid, sig)
}

/// Entry point for the CLI.
pub async fn run() -> Result<()> {
    let cli = Cli::parse();
    let dir = resolve_config_dir(cli.config_dir)?;
    match cli.command {
        Command::Init { name, id, force } => init_inner(&dir, name, id, force).await,
        Command::Start => start(&dir).await,
        Command::Stop => stop(&dir).await,
        Command::Status => status(&dir).await,
        Command::Identity => identity(&dir).await,
        Command::Version => {
            println!("tailcat-node {}", crate::VERSION);
            Ok(())
        }
        Command::Token => token(&dir).await,
        Command::Peer { command } => peer(&dir, command).await,
        Command::Connect { id } => connect(&dir, id).await,
        Command::Disconnect { id } => disconnect(&dir, id).await,
        Command::Ping { id } => ping(&dir, id).await,
        Command::Service { command } => service(&dir, command).await,
        Command::Doctor => doctor(&dir).await,
        Command::Logs => logs(&dir).await,
    }
}

/// Resolve the config directory.
fn resolve_config_dir(override_dir: Option<String>) -> Result<PathBuf> {
    if let Some(dir) = override_dir {
        return Ok(PathBuf::from(dir));
    }
    let home = dirs::config_dir()
        .ok_or_else(|| Error::InvalidArgument("cannot determine config directory".to_string()))?;
    Ok(home.join("tailcat-node"))
}

// -- init --

async fn init_inner(
    dir: &Path,
    name: Option<String>,
    id: Option<String>,
    force: bool,
) -> Result<()> {
    // Check if already initialized.
    let config_path = dir.join("config.toml");
    if config_path.exists() && !force {
        return Err(Error::AlreadyInitialized(format!(
            "{} already exists. Use --force to overwrite.",
            config_path.display()
        )));
    }

    // Create the directory structure.
    std::fs::create_dir_all(dir)?;
    std::fs::create_dir_all(dir.join("state"))?;
    std::fs::create_dir_all(dir.join("state/cache"))?;
    std::fs::create_dir_all(dir.join("logs"))?;

    // Generate node ID.
    let node_id = id.unwrap_or_else(|| format!("agent-{}", &uuid::Uuid::new_v4().to_string()[..8]));

    let node_name = name.unwrap_or_else(|| {
        hostname::get()
            .ok()
            .and_then(|h| h.into_string().ok())
            .unwrap_or_else(|| node_id.clone())
    });

    // Create config.
    let config = Config::default_for(&node_id, Some(&node_name));
    config.save(dir)?;

    // Create identity.
    let identity = crate::identity::store::generate(&node_id);
    let store = IdentityStore::new(dir.to_path_buf());
    store.save(&identity)?;

    // Create empty peers.toml.
    let peers_file = dir.join("peers.toml");
    std::fs::write(
        &peers_file,
        format!(
            "# tailcat-node peers\nversion = {}\n",
            crate::CONFIG_VERSION
        ),
    )?;

    // Create empty services.toml.
    let services_file = dir.join("services.toml");
    std::fs::write(
        &services_file,
        format!(
            "# tailcat-node services\nversion = {}\n",
            crate::CONFIG_VERSION
        ),
    )?;

    println!("Initialized tailcat-node in {}", dir.display());
    println!("  node id:   {}", node_id);
    println!("  node name: {}", node_name);
    println!();
    println!("Config files:");
    println!("  {}", config_path.display());
    println!("  {}", dir.join("identity.key").display());
    println!("  {}", peers_file.display());
    println!("  {}", services_file.display());
    println!();
    println!("To start the daemon:");
    println!("  tailcat-node start");

    Ok(())
}

// -- start --

async fn start(dir: &Path) -> Result<()> {
    // Load config and identity.
    let config = Config::load(dir)?;
    let identity = IdentityStore::new(dir.to_path_buf()).load()?;

    // Initialize the daemon.
    let daemon = Arc::new(Daemon::new(dir.to_path_buf(), config, identity)?);

    // Start the backend.
    daemon.backend_start().await?;

    // Write the PID file.
    let pid_path = dir.join("state/daemon.pid");
    std::fs::write(&pid_path, std::process::id().to_string())?;

    // Set up logging.
    let log_dir = dir.join("logs");
    let _ = std::fs::create_dir_all(&log_dir);
    let log_file = log_dir.join("tailcat-node.log");
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file)?;
    let log_level = daemon.config.daemon.logging.level.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(parse_log_level(&log_level))
        .with_writer(log_file)
        .finish();
    tracing::subscriber::set_global_default(subscriber).ok();

    // Start the IPC server.
    let socket_path = dir.join("tailcat-node.sock");
    tracing::info!("Starting tailcat-node daemon (pid={})", std::process::id());
    println!("tailcat-node daemon started (pid={})", std::process::id());

    // Serve until killed.
    crate::ipc::serve(daemon, socket_path).await?;

    // Clean up PID file.
    let _ = std::fs::remove_file(&pid_path);

    Ok(())
}

fn parse_log_level(level: &str) -> tracing::Level {
    match level.to_lowercase().as_str() {
        "trace" => tracing::Level::TRACE,
        "debug" => tracing::Level::DEBUG,
        "info" => tracing::Level::INFO,
        "warn" => tracing::Level::WARN,
        "error" => tracing::Level::ERROR,
        _ => tracing::Level::INFO,
    }
}

// -- stop --

async fn stop(dir: &Path) -> Result<()> {
    let pid_path = dir.join("state/daemon.pid");
    if !pid_path.exists() {
        return Err(Error::NotRunning("pid file not found".to_string()));
    }
    let pid_str = std::fs::read_to_string(&pid_path)?;
    let pid: u32 = pid_str
        .trim()
        .parse()
        .map_err(|_| Error::InvalidArgument(format!("invalid pid: {}", pid_str)))?;

    #[cfg(unix)]
    {
        unsafe { libc_kill(pid as i32, 15) };
        println!("Sent SIGTERM to pid {}", pid);
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
    }

    // Clean up PID file.
    let _ = std::fs::remove_file(&pid_path);

    Ok(())
}

// -- status --

async fn status(dir: &Path) -> Result<()> {
    match IpcClient::new(dir).get("/v1/node").await {
        Ok(value) => {
            println!("Node:");
            println!(
                "  id:         {}",
                value.get("node_id").and_then(|v| v.as_str()).unwrap_or("?")
            );
            println!(
                "  name:       {}",
                value
                    .get("node_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?")
            );
            println!(
                "  public_key: {}",
                value
                    .get("public_key")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?")
            );
            println!(
                "  version:    {}",
                value.get("version").and_then(|v| v.as_str()).unwrap_or("?")
            );
            println!();

            match IpcClient::new(dir).get("/v1/status").await {
                Ok(statuses) => {
                    println!("Peers:");
                    if let Some(arr) = statuses.as_array() {
                        for s in arr {
                            println!(
                                "  {}: {} ({} {}ms)",
                                s.get("peer_id").and_then(|v| v.as_str()).unwrap_or("?"),
                                s.get("state").and_then(|v| v.as_str()).unwrap_or("?"),
                                s.get("path").and_then(|v| v.as_str()).unwrap_or("?"),
                                s.get("latency_ms").and_then(|v| v.as_u64()).unwrap_or(0),
                            );
                        }
                    }
                }
                Err(_) => {
                    println!("  (no peer status available)");
                }
            }
        }
        Err(_) => {
            // Daemon not running — show config-level info.
            let config = Config::load(dir)?;
            let identity = IdentityStore::new(dir.to_path_buf()).load()?;
            println!("Node (offline):");
            println!("  id:         {}", config.node.id);
            println!(
                "  name:       {}",
                config.node.name.as_deref().unwrap_or("?")
            );
            println!("  public_key: {}", identity.public_key);
            println!("  version:    {}", crate::VERSION);
            println!();
            println!("Daemon: not running");
        }
    }
    Ok(())
}

// -- identity --

async fn identity(dir: &Path) -> Result<()> {
    let id = IdentityStore::new(dir.to_path_buf()).load()?;
    println!("node_id:     {}", id.node_id);
    println!("private_key: {}", id.private_key);
    println!("public_key:  {}", id.public_key);
    Ok(())
}

// -- token --

async fn token(dir: &Path) -> Result<()> {
    let id = IdentityStore::new(dir.to_path_buf()).load()?;
    // The join token is derived from the public key.
    let token = format!("tc-{}", id.public_key);
    println!("{}", token);
    Ok(())
}

// -- peer --

async fn peer(dir: &Path, command: PeerCommand) -> Result<()> {
    match command {
        PeerCommand::List => peer_list(dir).await,
        PeerCommand::Add { id, token, name } => peer_add(dir, id, token, name).await,
        PeerCommand::Remove { id } => peer_remove(dir, id).await,
        PeerCommand::Show { id } => peer_show(dir, id).await,
        PeerCommand::Enable { id } => peer_enable(dir, id).await,
        PeerCommand::Disable { id } => peer_disable(dir, id).await,
    }
}

async fn peer_list(dir: &Path) -> Result<()> {
    let client = IpcClient::new(dir);
    match client.get("/v1/peers").await {
        Ok(value) => {
            if let Some(arr) = value.as_array() {
                if arr.is_empty() {
                    println!("No peers configured.");
                    return Ok(());
                }
                print_peer_header();
                println!("{}", "-".repeat(70));
                for peer in arr {
                    let id = peer.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                    let name = peer.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                    let enabled = peer
                        .get("enabled")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true);
                    let token = peer.get("token").and_then(|v| v.as_str()).unwrap_or("?");
                    print_peer_row(id, name, enabled, token);
                }
            }
        }
        Err(_) => {
            // Fallback to reading peers.toml directly.
            let mut registry = PeerRegistry::new(dir.to_path_buf());
            registry.load()?;
            let peers = registry.list();
            if peers.is_empty() {
                println!("No peers configured.");
                return Ok(());
            }
            print_peer_header();
            println!("{}", "-".repeat(70));
            for peer in peers {
                print_peer_row(&peer.id, &peer.name, peer.enabled, &peer.token);
            }
        }
    }
    Ok(())
}

fn print_peer_header() {
    println!("{:<20} {:<20} {:<8} Token", "ID", "Name", "Enabled");
}

fn print_peer_row(id: &str, name: &str, enabled: bool, token: &str) {
    println!(
        "{:<20} {:<20} {:<8} {}",
        id,
        name,
        if enabled { "yes" } else { "no" },
        token
    );
}

async fn peer_add(dir: &Path, id: String, token: String, name: Option<String>) -> Result<()> {
    let peer_name = name.clone().unwrap_or_else(|| id.clone());
    // Try IPC first.
    let client = IpcClient::new(dir);
    let peer_json = serde_json::json!({
        "id": id,
        "name": peer_name,
        "token": token,
        "enabled": true,
        "metadata": {}
    });
    let body = serde_json::to_string(&peer_json)?;
    match client.post("/v1/peers", Some(&body)).await {
        Ok(_) => {
            println!("Added peer {} via daemon", id);
        }
        Err(_) => {
            // Fallback to direct file write.
            let mut registry = PeerRegistry::new(dir.to_path_buf());
            registry.load()?;
            let peer = Peer::new(id.clone(), name.unwrap_or_else(|| id.clone()), token);
            registry.add(peer);
            registry.save()?;
            println!("Added peer {} to {}", id, dir.join("peers.toml").display());
        }
    }
    Ok(())
}

async fn peer_remove(dir: &Path, id: String) -> Result<()> {
    let client = IpcClient::new(dir);
    match client.delete(&format!("/v1/peers/{}", id)).await {
        Ok(_) => {
            println!("Removed peer {}", id);
        }
        Err(_) => {
            let mut registry = PeerRegistry::new(dir.to_path_buf());
            registry.load()?;
            if registry.remove(&id) {
                registry.save()?;
                println!("Removed peer {}", id);
            } else {
                return Err(Error::PeerNotFound(id));
            }
        }
    }
    Ok(())
}

async fn peer_show(dir: &Path, id: String) -> Result<()> {
    let client = IpcClient::new(dir);
    match client.get(&format!("/v1/peers/{}", id)).await {
        Ok(value) => {
            println!(
                "ID:       {}",
                value.get("id").and_then(|v| v.as_str()).unwrap_or("?")
            );
            println!(
                "Name:     {}",
                value.get("name").and_then(|v| v.as_str()).unwrap_or("?")
            );
            println!(
                "Token:    {}",
                value.get("token").and_then(|v| v.as_str()).unwrap_or("?")
            );
            println!(
                "Enabled:  {}",
                value
                    .get("enabled")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true)
            );
        }
        Err(_) => {
            let mut registry = PeerRegistry::new(dir.to_path_buf());
            registry.load()?;
            match registry.get(&id) {
                Some(peer) => {
                    println!("ID:       {}", peer.id);
                    println!("Name:     {}", peer.name);
                    println!("Token:    {}", peer.token);
                    println!("Enabled:  {}", peer.enabled);
                }
                None => return Err(Error::PeerNotFound(id)),
            }
        }
    }
    Ok(())
}

async fn peer_enable(dir: &Path, id: String) -> Result<()> {
    let mut registry = PeerRegistry::new(dir.to_path_buf());
    registry.load()?;
    if registry.enable(&id) {
        registry.save()?;
        println!("Enabled peer {}", id);
        Ok(())
    } else {
        Err(Error::PeerNotFound(id))
    }
}

async fn peer_disable(dir: &Path, id: String) -> Result<()> {
    let mut registry = PeerRegistry::new(dir.to_path_buf());
    registry.load()?;
    if registry.disable(&id) {
        registry.save()?;
        println!("Disabled peer {}", id);
        Ok(())
    } else {
        Err(Error::PeerNotFound(id))
    }
}

// -- connect/disconnect/ping --

async fn connect(dir: &Path, id: String) -> Result<()> {
    let client = IpcClient::new(dir);
    match client.post(&format!("/v1/connect/{}", id), None).await {
        Ok(value) => {
            println!("Connected to {}", id);
            println!(
                "  state:    {}",
                value.get("state").and_then(|v| v.as_str()).unwrap_or("?")
            );
            println!(
                "  path:     {}",
                value.get("path").and_then(|v| v.as_str()).unwrap_or("?")
            );
            println!(
                "  latency:  {}ms",
                value
                    .get("latency_ms")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0)
            );
        }
        Err(e) => return Err(e),
    }
    Ok(())
}

async fn disconnect(dir: &Path, id: String) -> Result<()> {
    let client = IpcClient::new(dir);
    match client.post(&format!("/v1/disconnect/{}", id), None).await {
        Ok(_) => {
            println!("Disconnected from {}", id);
        }
        Err(e) => return Err(e),
    }
    Ok(())
}

async fn ping(dir: &Path, id: String) -> Result<()> {
    // Try IPC first.
    let client = IpcClient::new(dir);
    match client.post(&format!("/v1/connect/{}", id), None).await {
        Ok(value) => {
            println!("{}: reachable", id);
            println!(
                "  path:     {}",
                value.get("path").and_then(|v| v.as_str()).unwrap_or("?")
            );
            println!(
                "  latency:  {}ms",
                value
                    .get("latency_ms")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0)
            );
        }
        Err(_) => {
            // Fallback: check if peer exists.
            let mut registry = PeerRegistry::new(dir.to_path_buf());
            registry.load()?;
            match registry.get(&id) {
                Some(peer) => {
                    if peer.enabled {
                        println!("{}: reachable (offline check — peer is configured)", id);
                    } else {
                        println!("{}: not reachable (peer is disabled)", id);
                    }
                }
                None => return Err(Error::PeerNotFound(id)),
            }
        }
    }
    Ok(())
}

// -- service --

async fn service(dir: &Path, command: ServiceCommand) -> Result<()> {
    match command {
        ServiceCommand::List => service_list(dir).await,
        ServiceCommand::Add {
            name,
            port,
            protocol,
        } => service_add(dir, name, port, protocol).await,
        ServiceCommand::Remove { name } => service_remove(dir, name).await,
    }
}

async fn service_list(dir: &Path) -> Result<()> {
    let client = IpcClient::new(dir);
    match client.get("/v1/services").await {
        Ok(value) => {
            if let Some(arr) = value.as_array() {
                if arr.is_empty() {
                    println!("No services configured.");
                    return Ok(());
                }
                print_service_header();
                println!("{}", "-".repeat(40));
                for svc in arr {
                    let name = svc.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                    let port = svc.get("port").and_then(|v| v.as_u64()).unwrap_or(0);
                    let protocol = svc.get("protocol").and_then(|v| v.as_str()).unwrap_or("?");
                    print_service_row(name, port, protocol);
                }
            }
        }
        Err(_) => {
            let mut registry = ServiceRegistry::new(dir.to_path_buf());
            registry.load()?;
            let services = registry.list();
            if services.is_empty() {
                println!("No services configured.");
                return Ok(());
            }
            print_service_header();
            println!("{}", "-".repeat(40));
            for svc in services {
                print_service_row(&svc.name, svc.port as u64, &svc.protocol);
            }
        }
    }
    Ok(())
}

fn print_service_header() {
    println!("{:<20} {:<8} Protocol", "Name", "Port");
}

fn print_service_row(name: &str, port: u64, protocol: &str) {
    println!("{:<20} {:<8} {}", name, port, protocol);
}

async fn service_add(dir: &Path, name: String, port: u16, protocol: String) -> Result<()> {
    let mut registry = ServiceRegistry::new(dir.to_path_buf());
    registry.load()?;
    let service = Service::new(name.clone(), port, protocol);
    registry.add(service);
    registry.save()?;
    println!(
        "Added service {} to {}",
        name,
        dir.join("services.toml").display()
    );
    Ok(())
}

async fn service_remove(dir: &Path, name: String) -> Result<()> {
    let mut registry = ServiceRegistry::new(dir.to_path_buf());
    registry.load()?;
    if registry.remove(&name) {
        registry.save()?;
        println!("Removed service {}", name);
        Ok(())
    } else {
        Err(Error::ServiceNotFound(name))
    }
}

// -- doctor --

async fn doctor(dir: &Path) -> Result<()> {
    println!("tailcat-node doctor");
    println!("{}", "=".repeat(50));
    println!();

    // Check config directory.
    print!("Config directory: {} ", dir.display());
    if dir.exists() {
        println!("✓");
    } else {
        println!("✗ (not found)");
        return Ok(());
    }

    // Check config.toml.
    print!("config.toml:       ");
    if dir.join("config.toml").exists() {
        println!("✓");
    } else {
        println!("✗ (not found — run `tailcat-node init`)");
    }

    // Check identity.key.
    print!("identity.key:      ");
    if dir.join("identity.key").exists() {
        println!("✓");
    } else {
        println!("✗ (not found — run `tailcat-node init`)");
    }

    // Check peers.toml.
    print!("peers.toml:        ");
    if dir.join("peers.toml").exists() {
        println!("✓");
    } else {
        println!("✗ (not found)");
    }

    // Check services.toml.
    print!("services.toml:     ");
    if dir.join("services.toml").exists() {
        println!("✓");
    } else {
        println!("✗ (not found)");
    }

    // Check daemon.
    print!("Daemon running:    ");
    let pid_path = dir.join("state/daemon.pid");
    if pid_path.exists() {
        let pid_str = std::fs::read_to_string(&pid_path).unwrap_or_default();
        let pid: i32 = pid_str.trim().parse().unwrap_or(-1);
        if pid > 0 {
            #[cfg(unix)]
            {
                // Check if process exists by sending signal 0.
                let result = unsafe { libc_kill(pid, 0) };
                if result == 0 {
                    println!("✓ (pid={})", pid);
                } else {
                    println!("✗ (stale pid file)");
                }
            }
            #[cfg(not(unix))]
            {
                println!("? (pid={})", pid);
            }
        } else {
            println!("✗ (invalid pid)");
        }
    } else {
        println!("✗ (not running)");
    }

    // Check tailcat binary.
    print!("tailcat binary:    ");
    match which::which("tailcat") {
        Ok(path) => println!("✓ ({})", path.display()),
        Err(_) => println!("✗ (not found — using mock backend)"),
    }

    // Check IPC socket.
    print!("IPC socket:        ");
    let sock = dir.join("tailcat-node.sock");
    if sock.exists() {
        println!("✓");
    } else {
        println!("✗ (not found)");
    }

    Ok(())
}

// -- logs --

async fn logs(dir: &Path) -> Result<()> {
    let log_path = dir.join("logs/tailcat-node.log");
    if !log_path.exists() {
        println!("No log file found at {}", log_path.display());
        return Ok(());
    }
    // Read the last 50 lines.
    let output = std::process::Command::new("tail")
        .arg("-n")
        .arg("50")
        .arg(&log_path)
        .output()
        .map_err(Error::Io)?;
    println!("{}", String::from_utf8_lossy(&output.stdout));
    Ok(())
}
