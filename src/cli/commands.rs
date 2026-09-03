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
        Command::Start { foreground } => start(&dir, foreground).await,
        Command::Stop => stop(&dir).await,
        Command::Restart => restart(&dir).await,
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
        Command::Install { force, method } => install(force, method).await,
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

async fn start(dir: &Path, foreground: bool) -> Result<()> {
    // Load config and identity.
    let config = Config::load(dir)?;
    let identity = IdentityStore::new(dir.to_path_buf()).load()?;

    // If not foreground, spawn a detached child with --foreground and exit.
    if !foreground {
        // Warn if the tailcat binary is missing — the daemon will start in
        // degraded mode (peer/service management works, but no P2P connectivity).
        if which::which("tailcat").is_err() {
            eprintln!();
            eprintln!("⚠ tailcat binary not found — starting in DEGRADED MODE");
            eprintln!("  Peer and service management will work, but network operations");
            eprintln!("  (connect, disconnect, ping) will fail until tailcat is installed.");
            eprintln!("  Install tailcat:  https://github.com/caesar0301/tailcat");
            eprintln!();
        }

        // Check if already running.
        let pid_path = dir.join("state/daemon.pid");
        if pid_path.exists() {
            let pid_str = std::fs::read_to_string(&pid_path)?;
            if let Ok(pid) = pid_str.trim().parse::<i32>() {
                #[cfg(unix)]
                if unsafe { libc_kill(pid, 0) } == 0 {
                    return Err(Error::Daemon(format!(
                        "daemon already running (pid={})",
                        pid
                    )));
                }
            }
            // Stale PID file — remove it.
            let _ = std::fs::remove_file(&pid_path);
        }

        let exe = std::env::current_exe()?;
        let mut cmd = std::process::Command::new(&exe);
        // Global args must come before the subcommand.
        if let Some(dir_str) = dir.to_str() {
            cmd.arg("--config-dir").arg(dir_str);
        }
        cmd.arg("start").arg("--foreground");
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // Detach from terminal: create new session, become session leader.
            unsafe {
                cmd.pre_exec(|| {
                    libc::setsid();
                    Ok(())
                });
            }
        }

        let child = cmd.spawn()?;
        let child_pid = child.id();
        // Don't wait — drop the handle to let it run independently.
        drop(child);

        // Wait briefly for the child to write its PID file and bind the socket.
        let socket_path = dir.join("tailcat-node.sock");
        for _ in 0..50 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if socket_path.exists() {
                break;
            }
        }

        println!("tailcat-node daemon started (pid={})", child_pid);
        return Ok(());
    }

    // --- Foreground mode (the actual daemon process) ---

    // Warn if the tailcat binary is missing — the daemon will start in
    // degraded mode (peer/service management works, but no P2P connectivity).
    if which::which("tailcat").is_err() {
        eprintln!();
        eprintln!("⚠ tailcat binary not found — starting in DEGRADED MODE");
        eprintln!("  Peer and service management will work, but network operations");
        eprintln!("  (connect, disconnect, ping) will fail until tailcat is installed.");
        eprintln!("  Install tailcat:  https://github.com/caesar0301/tailcat");
        eprintln!();
    }

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

    // Set up signal handling for graceful shutdown.
    let pid_path_clone = pid_path.clone();
    let socket_path_clone = socket_path.clone();
    tokio::spawn(async move {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{signal, SignalKind};
            let mut sigterm = signal(SignalKind::terminate()).expect("install SIGTERM handler");
            let mut sigint = signal(SignalKind::interrupt()).expect("install SIGINT handler");
            tokio::select! {
                _ = sigterm.recv() => {
                    tracing::info!("Received SIGTERM, shutting down");
                }
                _ = sigint.recv() => {
                    tracing::info!("Received SIGINT, shutting down");
                }
            }
            // Clean up.
            let _ = std::fs::remove_file(&pid_path_clone);
            let _ = std::fs::remove_file(&socket_path_clone);
            std::process::exit(0);
        }
        #[cfg(not(unix))]
        {
            tokio::signal::ctrl_c().await.ok();
            let _ = std::fs::remove_file(&pid_path_clone);
            let _ = std::fs::remove_file(&socket_path_clone);
            std::process::exit(0);
        }
    });

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

// -- restart --

async fn restart(dir: &Path) -> Result<()> {
    let pid_path = dir.join("state/daemon.pid");
    let socket_path = dir.join("tailcat-node.sock");

    // Stop the daemon if it's running.
    if pid_path.exists() {
        let pid_str = std::fs::read_to_string(&pid_path)?;
        if let Ok(pid) = pid_str.trim().parse::<i32>() {
            #[cfg(unix)]
            {
                if unsafe { libc_kill(pid, 0) } == 0 {
                    unsafe { libc_kill(pid, 15) };
                    println!("Stopping daemon (pid={})...", pid);
                    // Wait for the process to exit and clean up.
                    for _ in 0..50 {
                        std::thread::sleep(std::time::Duration::from_millis(100));
                        if unsafe { libc_kill(pid, 0) } != 0 {
                            break;
                        }
                    }
                }
            }
        }
        let _ = std::fs::remove_file(&pid_path);
        let _ = std::fs::remove_file(&socket_path);
    }

    // Start the daemon fresh.
    start(dir, false).await
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
            let backend_ok = value
                .get("backend_available")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if backend_ok {
                println!("  backend:    ✓ tailcat (connected)");
            } else {
                println!("  backend:    ✗ tailcat not found (DEGRADED MODE)");
            }
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
        Err(e) => {
            // If the daemon is not running, do an offline check.
            if matches!(e, Error::NotRunning(_)) {
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
            } else {
                // Daemon is running but the operation failed (e.g. degraded mode).
                return Err(e);
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
        Err(_) => println!("✗ (not found — daemon will run in DEGRADED MODE)"),
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

// -- install --

/// Install the tailcat binary (the networking substrate that tailcat-node depends on).
///
/// Per https://github.com/tailscale/tailcat, tailcat can be installed via:
///   - Homebrew (macOS): `brew install tailcat`
///   - Go: `go install github.com/tailscale/tailcat/cmd/tailcat@latest`
///   - Nix: `nix profile install github:tailscale/tailcat`
///   - AUR (Arch Linux): `yay -S tailcat-bin`
///   - Prebuilt binaries from GitHub Releases
///
/// This command auto-detects the best available method for the current platform.
async fn install(force: bool, method: Option<String>) -> Result<()> {
    println!("tailcat-node install — installing tailcat binary");
    println!("{}", "=".repeat(50));
    println!();

    // Check if already installed.
    if !force {
        if let Ok(path) = which::which("tailcat") {
            println!("tailcat is already installed at: {}", path.display());
            println!("Use --force to reinstall.");
            return Ok(());
        }
    }

    // If a specific method was requested, use it.
    if let Some(m) = method {
        return install_by_method(&m, force).await;
    }

    // Auto-detect the best method for this platform.
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    println!("Detected platform: {}-{}", arch, os);
    println!();

    // Try methods in order of preference for this platform.
    let methods = detect_methods(os);
    if methods.is_empty() {
        return Err(Error::InvalidArgument(format!(
            "no supported install method detected for {}-{}. \
             Please install tailcat manually: https://github.com/tailscale/tailcat",
            arch, os
        )));
    }

    println!("Available install methods (in order of preference):");
    for (i, m) in methods.iter().enumerate() {
        println!("  {}. {}", i + 1, m);
    }
    println!();

    // Try each method until one succeeds.
    for m in &methods {
        println!("Trying method: {}...", m);
        match install_by_method(m, force).await {
            Ok(()) => {
                // Verify the install worked.
                if let Ok(path) = which::which("tailcat") {
                    println!();
                    println!("✓ tailcat installed successfully at: {}", path.display());
                    println!();
                    println!("You can now start the daemon with full P2P connectivity:");
                    println!("  tailcat-node start");
                    return Ok(());
                }
                eprintln!("  Method '{}' completed but tailcat not found on PATH.", m);
                eprintln!(
                    "  You may need to restart your shell or add the install location to PATH."
                );
                return Ok(());
            }
            Err(e) => {
                eprintln!("  Method '{}' failed: {}", m, e.inner_message());
                eprintln!("  Trying next method...");
                println!();
            }
        }
    }

    Err(Error::InvalidArgument(
        "All automatic install methods failed. Please install tailcat manually: https://github.com/tailscale/tailcat".to_string(),
    ))
}

/// Detect available install methods for the given OS, in order of preference.
fn detect_methods(os: &str) -> Vec<&'static str> {
    let mut methods = Vec::new();

    match os {
        "macos" => {
            if which::which("brew").is_ok() {
                methods.push("brew");
            }
            if which::which("go").is_ok() {
                methods.push("go");
            }
            if which::which("nix").is_ok() {
                methods.push("nix");
            }
            // Always offer binary download as a fallback.
            methods.push("binary");
        }
        "linux" => {
            // Check for Arch/AUR.
            if which::which("yay").is_ok() {
                methods.push("aur");
            }
            if which::which("go").is_ok() {
                methods.push("go");
            }
            if which::which("nix").is_ok() {
                methods.push("nix");
            }
            // Binary download works on all Linux distros.
            methods.push("binary");
        }
        "freebsd" | "openbsd" | "netbsd" => {
            if which::which("go").is_ok() {
                methods.push("go");
            }
            methods.push("binary");
        }
        "windows" => {
            // Windows has prebuilt binaries from GitHub Releases.
            methods.push("binary");
        }
        _ => {
            if which::which("go").is_ok() {
                methods.push("go");
            }
            methods.push("binary");
        }
    }

    methods
}

/// Install tailcat using a specific method.
async fn install_by_method(method: &str, _force: bool) -> Result<()> {
    match method {
        "brew" => {
            println!("Running: brew install tailcat");
            let output = std::process::Command::new("brew")
                .args(["install", "tailcat"])
                .stdout(std::process::Stdio::inherit())
                .stderr(std::process::Stdio::inherit())
                .output()
                .map_err(Error::Io)?;
            if !output.status.success() {
                return Err(Error::InvalidArgument(
                    "brew install tailcat failed".to_string(),
                ));
            }
            Ok(())
        }
        "go" => {
            println!("Running: go install github.com/tailscale/tailcat/cmd/tailcat@latest");
            let output = std::process::Command::new("go")
                .args(["install", "github.com/tailscale/tailcat/cmd/tailcat@latest"])
                .stdout(std::process::Stdio::inherit())
                .stderr(std::process::Stdio::inherit())
                .output()
                .map_err(Error::Io)?;
            if !output.status.success() {
                return Err(Error::InvalidArgument(
                    "go install failed — make sure GOPATH/bin is on your PATH".to_string(),
                ));
            }
            // Try to add GOPATH/bin to PATH hint.
            if let Ok(home) = std::env::var("HOME") {
                let gopath_bin = format!("{}/go/bin", home);
                if which::which("tailcat").is_err() {
                    println!();
                    println!("Note: tailcat was installed to {}/", gopath_bin);
                    println!("Add it to your PATH:");
                    println!("  export PATH=\"$PATH:{}\"", gopath_bin);
                }
            }
            Ok(())
        }
        "nix" => {
            println!("Running: nix profile install github:tailscale/tailcat");
            let output = std::process::Command::new("nix")
                .args(["profile", "install", "github:tailscale/tailcat"])
                .stdout(std::process::Stdio::inherit())
                .stderr(std::process::Stdio::inherit())
                .output()
                .map_err(Error::Io)?;
            if !output.status.success() {
                return Err(Error::InvalidArgument(
                    "nix profile install failed".to_string(),
                ));
            }
            Ok(())
        }
        "aur" => {
            println!("Running: yay -S --noconfirm tailcat-bin");
            let output = std::process::Command::new("yay")
                .args(["-S", "--noconfirm", "tailcat-bin"])
                .stdout(std::process::Stdio::inherit())
                .stderr(std::process::Stdio::inherit())
                .output()
                .map_err(Error::Io)?;
            if !output.status.success() {
                return Err(Error::InvalidArgument(
                    "yay -S tailcat-bin failed".to_string(),
                ));
            }
            Ok(())
        }
        "binary" => install_from_binary().await,
        _ => Err(Error::InvalidArgument(format!(
            "unknown install method '{}'. Valid: brew, go, nix, aur, binary",
            method
        ))),
    }
}

/// Download and install a prebuilt tailcat binary from GitHub Releases.
async fn install_from_binary() -> Result<()> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    // Determine the asset name pattern for this platform.
    // tailcat releases use: tailcat_<version>_<os>_<arch>.tar.gz (Linux)
    // and tailcat_<version>_<os>_<arch>.zip (Windows)
    let (os_name, arch_name, ext) = match (os, arch) {
        ("linux", "x86_64") => ("linux", "amd64", "tar.gz"),
        ("linux", "aarch64") => ("linux", "arm64", "tar.gz"),
        ("linux", "arm") => ("linux", "armv7", "tar.gz"),
        ("darwin", "x86_64") => ("darwin", "amd64", "tar.gz"),
        ("darwin", "aarch64") => ("darwin", "arm64", "tar.gz"),
        ("windows", "x86_64") => ("windows", "amd64", "zip"),
        ("windows", "aarch64") => ("windows", "arm64", "zip"),
        _ => {
            return Err(Error::InvalidArgument(format!(
                "no prebuilt binary available for {}-{}",
                arch, os
            )));
        }
    };

    // Query the GitHub API for the latest release.
    println!("Fetching latest tailcat release info from GitHub...");
    let api_url = "https://api.github.com/repos/tailscale/tailcat/releases/latest";
    let response = curl_text(api_url, "application/vnd.github+json")?;

    let release: serde_json::Value = serde_json::from_str(&response)?;

    let tag = release
        .get("tag_name")
        .and_then(|v| v.as_str())
        .unwrap_or("latest");
    println!("Latest release: {}", tag);

    // Find the matching asset.
    let assets = release
        .get("assets")
        .and_then(|v| v.as_array())
        .ok_or_else(|| Error::InvalidArgument("no assets in release".to_string()))?;

    let asset = assets.iter().find(|a| {
        let name = a.get("name").and_then(|v| v.as_str()).unwrap_or("");
        name.contains(&arch_name)
            && name.contains(os_name)
            && (name.ends_with(".tar.gz") || name.ends_with(".zip"))
    });

    let asset = asset.ok_or_else(|| {
        Error::InvalidArgument(format!(
            "no matching asset found for {}-{} in release {} (looked for pattern containing '{}' and '{}')",
            arch, os, tag, arch_name, os_name
        ))
    })?;

    let download_url = asset
        .get("browser_download_url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::InvalidArgument("download URL not found in asset".to_string()))?;

    let asset_name = asset
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("tailcat_download");

    println!("Downloading: {} ({})", asset_name, download_url);

    // Download to a temp file.
    let tmp_dir = std::env::temp_dir().join("tailcat-node-install");
    std::fs::create_dir_all(&tmp_dir)?;
    let archive_path = tmp_dir.join(asset_name);
    curl_download_file(download_url, &archive_path)?;
    let file_size = std::fs::metadata(&archive_path)?.len();
    println!("Downloaded {} bytes", file_size);

    // Extract the binary.
    let extract_dir = tmp_dir.join("extracted");
    std::fs::create_dir_all(&extract_dir)?;

    if ext == "tar.gz" {
        println!("Extracting tar.gz...");
        let output = std::process::Command::new("tar")
            .args([
                "-xzf",
                &archive_path.to_string_lossy(),
                "-C",
                &extract_dir.to_string_lossy(),
            ])
            .output()
            .map_err(Error::Io)?;
        if !output.status.success() {
            return Err(Error::InvalidArgument(format!(
                "tar extraction failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
    } else {
        // .zip — use unzip on Unix, Expand-Archive on Windows.
        #[cfg(unix)]
        {
            println!("Extracting zip...");
            let output = std::process::Command::new("unzip")
                .args([
                    "-o",
                    &archive_path.to_string_lossy(),
                    "-d",
                    &extract_dir.to_string_lossy(),
                ])
                .output()
                .map_err(Error::Io)?;
            if !output.status.success() {
                return Err(Error::InvalidArgument(format!(
                    "unzip failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                )));
            }
        }
        #[cfg(windows)]
        {
            println!("Extracting zip...");
            let output = std::process::Command::new("powershell")
                .args([
                    "-NoProfile",
                    "-Command",
                    &format!(
                        "Expand-Archive -Path '{}' -DestinationPath '{}' -Force",
                        archive_path.display(),
                        extract_dir.display()
                    ),
                ])
                .output()
                .map_err(Error::Io)?;
            if !output.status.success() {
                return Err(Error::InvalidArgument("Expand-Archive failed".to_string()));
            }
        }
    }

    // Find the tailcat binary in the extracted files.
    let binary_name = if os == "windows" {
        "tailcat.exe"
    } else {
        "tailcat"
    };
    let binary_path = find_binary(&extract_dir, binary_name)?;

    // Install to a suitable location.
    let install_dir = determine_install_dir()?;
    std::fs::create_dir_all(&install_dir)?;
    let dest = install_dir.join(binary_name);

    // Copy the binary.
    std::fs::copy(&binary_path, &dest)?;

    // Make executable on Unix.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o755);
        std::fs::set_permissions(&dest, perms)?;
    }

    println!("Installed tailcat to: {}", dest.display());
    if which::which("tailcat").is_err() {
        println!();
        println!("Note: {} is not on your PATH.", install_dir.display());
        println!("Add it to your PATH:");
        println!("  export PATH=\"$PATH:{}\"", install_dir.display());
    }

    // Clean up temp files.
    let _ = std::fs::remove_dir_all(&tmp_dir);

    Ok(())
}

/// HTTP GET via curl, returns response body as text (for JSON API calls).
fn curl_text(url: &str, accept: &str) -> Result<String> {
    let output = std::process::Command::new("curl")
        .args([
            "-fsSL",
            "-H",
            &format!("Accept: {}", accept),
            "-H",
            "User-Agent: tailcat-node-installer",
            url,
        ])
        .output()
        .map_err(|e| Error::InvalidArgument(format!("curl not available: {}", e)))?;

    if !output.status.success() {
        return Err(Error::InvalidArgument(format!(
            "HTTP request failed (curl exit {}): {}",
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Download a file via curl -o (for binary downloads).
fn curl_download_file(url: &str, dest: &Path) -> Result<()> {
    let output = std::process::Command::new("curl")
        .args([
            "-fsSL",
            "-H",
            "User-Agent: tailcat-node-installer",
            "-o",
            &dest.to_string_lossy(),
            url,
        ])
        .output()
        .map_err(|e| Error::InvalidArgument(format!("curl not available: {}", e)))?;

    if !output.status.success() {
        return Err(Error::InvalidArgument(format!(
            "download failed (curl exit {}): {}",
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    Ok(())
}

/// Recursively search for a binary by name in a directory.
fn find_binary(dir: &Path, name: &str) -> Result<PathBuf> {
    for entry in walk_dir(dir) {
        if entry.file_name().and_then(|n| n.to_str()) == Some(name) {
            return Ok(entry);
        }
    }
    Err(Error::InvalidArgument(format!(
        "binary '{}' not found in extracted archive",
        name
    )))
}

/// Simple recursive directory walker.
fn walk_dir(dir: &Path) -> Vec<PathBuf> {
    let mut results = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                results.extend(walk_dir(&path));
            } else {
                results.push(path);
            }
        }
    }
    results
}

/// Determine where to install the binary.
fn determine_install_dir() -> Result<PathBuf> {
    // Try /usr/local/bin (common on macOS and Linux).
    let usr_local = PathBuf::from("/usr/local/bin");
    if usr_local.exists() {
        // Check if we can write to it.
        if std::fs::metadata(&usr_local).is_ok() {
            let test = usr_local.join(".tailcat_write_test");
            if std::fs::write(&test, "test").is_ok() {
                let _ = std::fs::remove_file(&test);
                return Ok(usr_local);
            }
        }
    }

    // Fall back to ~/.local/bin (Linux) or ~/bin.
    if let Ok(home) = std::env::var("HOME") {
        let local_bin = PathBuf::from(&home).join(".local/bin");
        std::fs::create_dir_all(&local_bin).ok();
        return Ok(local_bin);
    }

    // Last resort: /tmp
    Ok(PathBuf::from("/tmp"))
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
