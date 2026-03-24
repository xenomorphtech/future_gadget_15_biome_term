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
│  (Rust client lib    │                    │   (port 3000)        │
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
# or set BIOME_TERM_URL=http://host:3000 cargo run
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

The Rust server exposes a REST + WebSocket API on **port 3000**.

See **[docs/api.md](docs/api.md)** for the full reference (also live at `GET /openapi.json`).

```bash
cd server && cargo run --bin gen-docs   # regenerate docs/api.md
```

## Running Tests

```bash
cd server && cargo test
```
