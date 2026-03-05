# Deployment Guide

Sigillum runs in three modes. Choose based on your needs.

## Mode 1: Embedded Library (Local)

The simplest deployment. Your application links `sigillum-core` directly and accesses vault files on the local filesystem. No daemon, no network.

```
┌─────────────────┐
│  Your App       │
│  ┌────────────┐ │
│  │  FileVault │ │──── ~/.sigillum/vault.enc
│  └────────────┘ │──── ~/.sigillum/api_keys.json
└─────────────────┘
```

### When to use

- Single application accessing secrets
- No need for web UI or audit logging
- CI/CD pipelines
- Development environments

### Setup

```toml
# Cargo.toml
[dependencies]
sigillum = "0.1"
```

```rust
use sigillum::{FileVault, VaultConfig, SecretStore};

let vault = FileVault::new(VaultConfig::default());

// Tier 1: immediate access
vault.set_api_key("github_token", "ghp_...")?;

// Tier 2: unlock first, then access
// vault.load_master_key(key);  // from FIDO2 or passphrase
// vault.set_secret("db_pass", "...")?;
```

### Limitations

- Master key lives in your application's memory
- No web UI
- No audit log (unless you add one)
- Every process that needs secrets must unlock independently

---

## Mode 2: Local Daemon

A long-running daemon on the same machine. Unlock once, all local applications share the session. Includes web UI and audit logging.

```
┌─────────┐  ┌─────────┐  ┌─────────┐
│  App A  │  │  App B  │  │ Browser │
└────┬────┘  └────┬────┘  └────┬────┘
     │            │            │
     └────────────┼────────────┘
                  │ Unix socket / localhost
           ┌──────▼──────┐
           │   Sigillum  │
           │   Daemon    │──── Web UI on :9743
           └──────┬──────┘
                  │
           ~/.sigillum/
```

### When to use

- Multiple applications on one machine need shared secrets
- You want the web UI for management
- You want centralized audit logging
- Development workstation with hardware keys

### Setup

```bash
# Start daemon
sigillum daemon --port 9743

# Or bind to Unix socket only (no TCP exposure)
sigillum daemon --socket /run/sigillum.sock
```

**Unlock via CLI:**
```bash
sigillum unlock              # FIDO2 tap
sigillum unlock --passphrase # Passphrase fallback
```

**Unlock via web UI:**

Open `http://localhost:9743` in a browser. The UI prompts for FIDO2 tap via WebAuthn or passphrase input.

**Connect from your app:**
```rust
use sigillum_client::RemoteVault;
use sigillum_core::SecretStore;

let vault = RemoteVault::connect_unix("/run/sigillum.sock")?;
// Or: RemoteVault::connect("http://localhost:9743")?;

let secret = vault.get_secret("db_password");
```

### systemd Service (Linux)

```ini
# /etc/systemd/system/sigillum.service
[Unit]
Description=Sigillum Vault Daemon
After=network.target

[Service]
Type=simple
ExecStart=/usr/local/bin/sigillum daemon --socket /run/sigillum.sock
Restart=on-failure
RestartSec=5

# Security hardening
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=read-only
ReadWritePaths=/run/sigillum.sock
ReadWritePaths=%h/.sigillum
PrivateTmp=true
MemoryDenyWriteExecute=true

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl enable --now sigillum
```

### launchd Service (macOS)

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.caelator.sigillum</string>
    <key>ProgramArguments</key>
    <array>
        <string>/usr/local/bin/sigillum</string>
        <string>daemon</string>
        <string>--port</string>
        <string>9743</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>/tmp/sigillum.stdout.log</string>
    <key>StandardErrorPath</key>
    <string>/tmp/sigillum.stderr.log</string>
</dict>
</plist>
```

```bash
cp com.caelator.sigillum.plist ~/Library/LaunchAgents/
launchctl load ~/Library/LaunchAgents/com.caelator.sigillum.plist
```

---

## Mode 3: Remote Daemon

The daemon runs on a dedicated machine (or container). Applications on other machines connect over TLS.

```
┌─────────┐       ┌─────────┐       ┌─────────┐
│ Server A │       │ Server B │       │ Laptop  │
│  App     │       │  App     │       │ Browser │
└────┬─────┘       └────┬─────┘       └────┬────┘
     │                  │                  │
     └──────────────────┼──────────────────┘
                        │ HTTPS / mTLS
                 ┌──────▼──────┐
                 │   Sigillum  │
                 │   Daemon    │ (dedicated host)
                 └──────┬──────┘
                        │
                 ~/.sigillum/
```

### When to use

- Multiple machines need access to the same secrets
- You want a dedicated secrets server
- You need centralized audit logging across your infrastructure
- You want the FIDO2 unlock to happen on a controlled machine

### Setup

**On the vault host:**
```bash
# Generate TLS certificates
sigillum tls init --ca-dir /etc/sigillum/pki

# Start daemon with TLS
sigillum daemon \
    --bind 0.0.0.0:9743 \
    --tls-cert /etc/sigillum/pki/server.crt \
    --tls-key /etc/sigillum/pki/server.key \
    --client-ca /etc/sigillum/pki/ca.crt  # mTLS
```

**Register a client:**
```bash
# On the vault host — generates a client cert
sigillum client add "server-a" --output /tmp/server-a.crt

# Copy cert to client machine
scp /tmp/server-a.crt server-a:/etc/sigillum/client.crt
```

**On the client machine:**
```rust
use sigillum_client::RemoteVault;

let vault = RemoteVault::builder()
    .url("https://vault.internal:9743")
    .client_cert("/etc/sigillum/client.crt")
    .client_key("/etc/sigillum/client.key")
    .ca_cert("/etc/sigillum/ca.crt")
    .build()?;

let secret = vault.get_secret("db_password");
```

### Docker

```dockerfile
FROM rust:1.85-slim AS builder
WORKDIR /build
COPY . .
RUN cargo build --release -p sigillum-cli

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/target/release/sigillum /usr/local/bin/
VOLUME /data
EXPOSE 9743
CMD ["sigillum", "daemon", "--bind", "0.0.0.0:9743", "--data-dir", "/data"]
```

```bash
docker build -t sigillum .
docker run -d \
    --name sigillum \
    -p 9743:9743 \
    -v sigillum-data:/data \
    sigillum
```

### Docker Compose

```yaml
services:
  sigillum:
    build: .
    ports:
      - "9743:9743"
    volumes:
      - sigillum-data:/data
      - ./pki:/etc/sigillum/pki:ro
    environment:
      - SIGILLUM_TLS_CERT=/etc/sigillum/pki/server.crt
      - SIGILLUM_TLS_KEY=/etc/sigillum/pki/server.key
    restart: unless-stopped

volumes:
  sigillum-data:
```

---

## Comparison

| Feature | Embedded | Local Daemon | Remote Daemon |
|---------|----------|--------------|---------------|
| Dependencies | `sigillum` crate | `sigillum-client` + daemon binary | `sigillum-client` + daemon on remote host |
| Network | None | Unix socket / localhost | TLS over network |
| Web UI | No | Yes | Yes |
| Audit log | No (DIY) | Yes | Yes |
| Shared unlock | No (per-process) | Yes (all local apps) | Yes (all connected clients) |
| FIDO2 unlock | Direct USB | Direct USB on daemon host | Direct USB on daemon host |
| Setup complexity | Minimal | Low | Medium (TLS/certs) |
| Security isolation | App-level | Process-level | Host-level |
| Best for | Single app, CI | Dev workstation | Production infrastructure |

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `SIGILLUM_DATA_DIR` | `~/.sigillum` | Base directory for vault files |
| `SIGILLUM_PORT` | `9743` | Daemon TCP port |
| `SIGILLUM_SOCKET` | — | Unix socket path (overrides TCP) |
| `SIGILLUM_TLS_CERT` | — | TLS certificate path |
| `SIGILLUM_TLS_KEY` | — | TLS private key path |
| `SIGILLUM_CLIENT_CA` | — | CA cert for mTLS client validation |
| `SIGILLUM_LOG` | `info` | Log level (trace, debug, info, warn, error) |
