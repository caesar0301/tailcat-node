# tailcat-node

A small cross-platform daemon that wraps [Tailcat](https://github.com/caesar0301/tailcat) to provide node lifecycle management, peer discovery, and a local IPC API — designed as the networking substrate for agent runtimes.

> **Design principle:** `tailcat-node` owns node lifecycle, peer management, and agent-level semantics. Tailcat owns encrypted connectivity (WireGuard, magicsock, DERP, NAT traversal). The daemon stays extremely thin — it is the small bridge between the agent runtime and the network.

## Architecture

```
┌──────────────────────────────────────────────┐
│               Agent Runtime                  │
│  ACP · Agent sessions · Tasks · Scheduling   │
└──────────────────────┬───────────────────────┘
                       │  Mesh API / SDK
┌──────────────────────▼───────────────────────┐
│                 tailcat-node                 │
│  Identity · Peers · Services · ACL · IPC     │
│  Lifecycle · Discovery · Connection Manager  │
└──────────────────────┬───────────────────────┘
                       │  Tailcat API
┌──────────────────────▼───────────────────────┐
│                   Tailcat                    │
│  WireGuard · magicsock · NAT · DERP          │
└──────────────────────┬───────────────────────┘
                       │  encrypted P2P network
         ┌─────────────┼─────────────┐
         ▼             ▼             ▼
      Node A         Node B         Node C
```

## Features

- **Persistent identity** — cryptographic keypair survives restarts
- **Peer registry** — TOML-based peer management with enable/disable
- **Service registry** — advertise local services (HTTP, ACP, SSH, …)
- **Lazy connections** — only connect on demand, not N² at startup
- **Local IPC API** — JSON over Unix socket for programmatic control
- **Hierarchical CLI** — `peer`, `service`, `connect`, `ping`, `doctor`
- **Cross-platform** — Linux, macOS (Windows planned)
- **Mock backend** — works without the `tailcat` binary for development

## Prerequisites

### 1. Install Tailcat

`tailcat-node` delegates all encrypted connectivity to the `tailcat` binary. Install it first:

```bash
# From source (requires Go 1.21+)
git clone https://github.com/caesar0301/tailcat.git
cd tailcat
go install ./cmd/tailcat

# Or download a prebuilt binary from releases
curl -fsSL https://github.com/caesar0301/tailcat/releases/latest/download/tailcat-linux-amd64 -o /usr/local/bin/tailcat
chmod +x /usr/local/bin/tailcat
```

Verify:

```bash
tailcat version
```

> **Note:** If `tailcat` is not on your `PATH`, `tailcat-node` automatically falls back to a mock backend. This is useful for development and testing the daemon logic without a real network.

### 2. Install tailcat-node

**From source (requires Rust 1.74+):**

```bash
git clone https://github.com/caesar0301/tailcat-node.git
cd tailcat-node
make install    # installs to /usr/local/bin/tailcat-node
```

**From prebuilt binaries:**

```bash
# Linux x86_64
curl -fsSL https://github.com/caesar0301/tailcat-node/releases/latest/download/tailcat-node-x86_64-linux-gnu.tar.gz | tar xz
sudo mv tailcat-node /usr/local/bin/

# macOS Apple Silicon
curl -fsSL https://github.com/caesar0301/tailcat-node/releases/latest/download/tailcat-node-aarch64-apple-darwin.tar.gz | tar xz
sudo mv tailcat-node /usr/local/bin/
```

Verify:

```bash
tailcat-node version
```

## Quick Start

### Initialize a node

```bash
tailcat-node init --name node-a
```

This creates the config directory (default: `~/.config/tailcat-node/`):

```
~/.config/tailcat-node/
├── config.toml       # this node's configuration
├── identity.key      # persistent cryptographic identity
├── peers.toml        # other nodes (empty)
├── services.toml     # local services (empty)
├── state/            # runtime state (pid, cache)
└── logs/
    └── tailcat-node.log
```

### Get your join token

```bash
tailcat-node token
```

Output:

```
tc-a1b2c3d4e5f6...
```

### On another machine

```bash
tailcat-node init --name node-b
tailcat-node peer add node-a tc-a1b2c3d4e5f6...
```

Back on the first machine, add the second node:

```bash
tailcat-node peer add node-b tc-f6e5d4c3b2a1...
```

### Start the daemon

```bash
tailcat-node start
```

### Check status

```bash
tailcat-node status
```

```
Node:
  id:         agent-a1b2c3d4
  name:       node-a
  public_key: a1b2c3d4e5f6...
  version:    0.0.1

Peers:
  agent-b2c3d4e5: connected (direct 12ms)
```

### Ping a peer

```bash
tailcat-node ping node-b
```

```
node-b: reachable
  path:     direct
  latency:  9ms
```

## CLI Reference

```text
tailcat-node init [--name <name>] [--id <id>] [--force]
tailcat-node start
tailcat-node stop
tailcat-node status
tailcat-node identity
tailcat-node version
tailcat-node token

tailcat-node peer list
tailcat-node peer add <id> <token> [--name <name>]
tailcat-node peer remove <id>
tailcat-node peer show <id>
tailcat-node peer enable <id>
tailcat-node peer disable <id>

tailcat-node connect <id>
tailcat-node disconnect <id>
tailcat-node ping <id>

tailcat-node service list
tailcat-node service add <name> <port> [protocol]
tailcat-node service remove <name>

tailcat-node doctor
tailcat-node logs
```

Override the config directory with `--config-dir <path>` or `TAILCAT_NODE_CONFIG_DIR=<path>`.

## Configuration

### `config.toml` — this node

```toml
version = 1

[node]
id = "agent-001"
name = "node-a"

[daemon]
listen_port = 4242

[daemon.logging]
level = "info"

[network]
mode = "lazy"

[network.derp]
enabled = true

[security]
require_peer_auth = true
```

### `peers.toml` — other nodes

```toml
version = 1

[[peers]]
id = "agent-002"
name = "node-b"
token = "tc-xxxxxxxx"
enabled = true
```

### `services.toml` — local services

```toml
version = 1

[[services]]
name = "agent"
port = 8080
protocol = "http"

[[services]]
name = "acp"
port = 9000
protocol = "acp"
```

## Local IPC API

The daemon exposes a JSON API over a Unix socket (`~/.config/tailcat-node/tailcat-node.sock`):

| Method   | Path                | Description            |
|----------|---------------------|------------------------|
| `GET`    | `/v1/node`          | Node info             |
| `GET`    | `/v1/peers`         | List peers            |
| `GET`    | `/v1/peers/:id`     | Show a peer           |
| `POST`   | `/v1/peers`         | Add a peer            |
| `DELETE` | `/v1/peers/:id`     | Remove a peer         |
| `POST`   | `/v1/connect/:id`   | Connect to a peer     |
| `POST`   | `/v1/disconnect/:id`| Disconnect from a peer|
| `GET`    | `/v1/services`      | List services         |
| `GET`    | `/v1/status`        | Peer connection status|

Example:

```bash
curl --unix-socket ~/.config/tailcat-node/tailcat-node.sock http://localhost/v1/node
```

```json
{
  "node_id": "agent-001",
  "node_name": "node-a",
  "public_key": "a1b2c3d4e5f6...",
  "version": "0.0.1"
}
```

## Development

```bash
make build       # debug build
make release     # release build
make test        # run tests
make lint        # clippy + fmt check
make run ARGS="status"   # run with args
```

## Connection Model

`tailcat-node` uses **lazy connections**. Peers are *known* but not *connected* until explicitly requested. For 1,000 known peers, only the few you actively use get connections — not the ~500,000 that a full mesh would require.

Connection states:

```
Disabled → Available → Connecting → Connected → Idle → Disconnected
                                      ↓
                                   Failed
```

The daemon reports whether a connection uses **direct P2P** or **DERP relay**:

```bash
tailcat-node ping node-b
#   path: direct   ← direct UDP tunnel
#   path: derp     ← relayed via DERP server
```

## Security Model

| Layer | File             | Purpose                              |
|-------|------------------|--------------------------------------|
| 1     | `identity.key`   | Private key, never transmitted        |
| 2     | `peers.toml`     | Only configured peers can connect    |
| 3     | `acl.toml` *(planned)* | Per-service access control    |

> **Important:** Do not rely on `tailcat-node` ACLs alone to prevent Internet access. Enforce network isolation at the MicroVM / network namespace / firewall level.

## Roadmap

- **Phase 1** (current): identity, peers, Tailcat process management, CLI, start/stop
- **Phase 2** (current): local IPC, service registry, connection manager
- **Phase 3** (planned): Mesh Controller, automatic peer sync, ACL, service discovery
- **Phase 4** (planned): Agent Runtime SDK, ACP integration, MicroVM networking, network policy

## License

[MIT](LICENSE) © 2026 Xiaming Chen
