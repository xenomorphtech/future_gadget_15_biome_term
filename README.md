# biome_term

A persistent PTY session server with multiple client frontends.

Each **pane** is an independent shell process backed by a VT100 emulator.
The server maintains authoritative terminal state, so clients can reconnect
without losing history. A WebSocket stream delivers live output; an HTTP
snapshot API lets new clients catch up instantly.

```
┌──────────────────────┐
│  biome-term-gui      │  native egui GUI (Linux binary in releases)
│  (Rust, egui)        │
└──────────┬───────────┘
           │
┌──────────┴───────────┐     HTTP + WS      ┌──────────────────────┐
│  biome-term-client   │ ←────────────────→ │   Rust PTY Server    │
│  (Rust client lib    │                    │   (port 3021)        │
│   + CLI binary)      │                    │  vt100 emulator      │
└──────────────────────┘                    └──────────────────────┘
           ▲
┌──────────┴───────────┐
│  Phoenix LiveView    │  web frontend (port 4000)
│  (Elixir/Phoenix)    │
└──────────────────────┘
```

## Clients

### Native GUI (`gui_client_rs`) — recommended

A native desktop app built with [egui](https://github.com/emilk/egui).

**Pre-built Linux binary** available in [Releases](../../releases).

Features:
- Multi-pane sidebar — create, switch, delete panes
- Full ANSI 256-colour VT100 rendering
- Click terminal to enter **direct input mode** — all keystrokes (Ctrl+C/D/Z, arrows, F-keys, etc.) go straight to the PTY
- Typed input bar with **↑/↓ global history** (zsh-style, draft preserved)
- Manual resize (cols × rows) — no auto-resize
- Horizontal scroll, no word-wrap
- Font size slider, configurable server URL

```bash
cd gui_client_rs && cargo run
# or set BIOME_TERM_URL=https://host:3027 BIOME_API_KEY=... BIOME_TLS_CA_CERT=/path/to/ca.pem cargo run
```

### CLI (`client_rs`)

A library + CLI tool for scripting and automation.

```bash
cd client_rs
cargo run --bin biome-term -- create --name myshell
cargo run --bin biome-term -- list
cargo run --bin biome-term -- input <ID> "echo hello\r"
cargo run --bin biome-term -- stream <ID>    # live PTY output to stdout
cargo run --bin biome-term -- lifecycle      # pane create/delete events as JSON

# HTTPS/WSS with API key + custom CA bundle
cargo run --bin biome-term -- \
  --url https://localhost:3027 \
  --api-key changeme \
  --ca-cert /path/to/server.crt \
  list
```

### Phoenix LiveView (`client`)

Browser-based frontend.

```bash
cd client && mix phx.server
```

Open `http://localhost:4000`.

## Quick Start

```bash
# Terminal 1 — start the PTY server
cd server && cargo run

# Terminal 2 — native GUI
cd gui_client_rs && cargo run

# — or — browser frontend
cd client && mix phx.server   # → http://localhost:4000
```

For the default local-only server wiring, the Rust backend listens on
`127.0.0.1:3021` and the Phoenix client talks to `http://localhost:3021`.

## Project Layout

```
biome_term/
├── server/          # Rust — axum HTTP/WebSocket PTY server
│   ├── src/
│   │   ├── pane.rs         # PTY lifecycle, vt100 parser, event log
│   │   ├── state.rs        # DashMap<Uuid, Arc<Pane>>
│   │   ├── event.rs        # append-only seq-numbered circular buffer
│   │   ├── openapi.rs      # utoipa OpenAPI spec
│   │   └── handlers/       # one file per route
│   └── tests/integration.rs
├── client_rs/       # Rust — async client library + CLI (biome-term)
├── gui_client_rs/   # Rust — native egui GUI (biome-term-gui)
└── client/          # Elixir — Phoenix LiveView web frontend
```

## API

The Rust server always exposes local HTTP + WebSocket access on `127.0.0.1:3021`
by default. If `BIOME_TLS_CERT` and `BIOME_TLS_KEY` are set, it also exposes
HTTPS/WSS on `BIOME_TLS_LISTEN_ADDR` (default `0.0.0.0:3027`), which must use
a different port than the local HTTP listener.

## Runtime Configuration

### Server

- `LISTEN_ADDR`: chooses the HTTP port, but the host is always forced to `127.0.0.1`. For example, `LISTEN_ADDR=0.0.0.0:3100` still binds HTTP to `127.0.0.1:3100`.
- `BIOME_API_KEY`: if set, all HTTP and WebSocket endpoints require either `Authorization: Bearer <key>` or `X-API-Key: <key>`. If unset, the server stays open for backwards compatibility.
- `BIOME_DEFAULT_MAX_EVENTS`: startup default for the per-pane event log retention limit. If unset, the server keeps `10_000` events per pane.
- `BIOME_TLS_CERT` and `BIOME_TLS_KEY`: enable HTTPS/WSS when both are set to certificate and private-key files.
- `BIOME_TLS_LISTEN_ADDR`: HTTPS bind address, default `0.0.0.0:3027`. This port must differ from the HTTP port.

### Client

- `BIOME_TERM_URL`: base URL for the Rust clients. `BIOME_URL` is also accepted as a fallback. Use `http://localhost:3021` for local HTTP or `https://host.example:3027` when TLS is enabled.
- `BIOME_API_KEY`: API key sent by the Rust CLI, Rust GUI, and Elixir client for both HTTP requests and websocket handshakes.
- `BIOME_TLS_CA_CERT`: PEM file containing one or more trusted root certificates for the Rust CLI and Rust GUI when connecting to a custom or self-signed TLS listener.
- `BIOME_TLS_INSECURE`: when set to `1`, `true`, `yes`, or `on`, the Rust CLI and Rust GUI skip TLS certificate and hostname validation. This is for local testing only.

### Example

```bash
# Local HTTP only, bound to localhost
cd server
cargo run

# HTTPS enabled on a separate port, while HTTP stays on localhost
LISTEN_ADDR=127.0.0.1:3021 \
BIOME_TLS_CERT=/path/to/server.crt \
BIOME_TLS_KEY=/path/to/server.key \
BIOME_TLS_LISTEN_ADDR=0.0.0.0:3027 \
BIOME_API_KEY=changeme \
cargo run

# Phoenix client pointed at the HTTPS listener
cd ../client
BIOME_URL=https://localhost:3027 BIOME_API_KEY=changeme mix phx.server
```

See **[docs/api.md](docs/api.md)** for the full reference (also live at `GET /openapi.json`).

```bash
cd server && cargo run --bin gen-docs   # regenerate docs/api.md
```

### Runtime API Configuration

The server now exposes `GET /config` and `PATCH /config` for runtime defaults.

- `default_cols` and `default_rows` control the terminal size used by `POST /panes` when the request omits `cols` or `rows`.
- `default_max_events` controls the event-log retention limit for panes created after the update.
- `apply_to_existing=true` immediately resizes all running panes to the new default size. This is user-visible and behaves like calling `POST /panes/{id}/resize` for every pane.
- `default_max_events` is not retroactive. Existing panes keep the event-log limit they were created with.

Examples:

```bash
# Inspect current runtime defaults
curl http://127.0.0.1:3021/config

# Change defaults for future panes only
curl -X PATCH http://127.0.0.1:3021/config \
  -H 'content-type: application/json' \
  -d '{"default_cols":160,"default_rows":40,"default_max_events":20000}'

# Change defaults and immediately resize every running pane
curl -X PATCH http://127.0.0.1:3021/config \
  -H 'content-type: application/json' \
  -d '{"default_cols":132,"default_rows":41,"apply_to_existing":true}'
```

## Running Tests

```bash
cd server && cargo test
```
