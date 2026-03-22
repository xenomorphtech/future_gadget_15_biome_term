# biome_term

A persistent PTY session server with a Phoenix LiveView frontend.

Each **pane** is an independent shell process backed by a VT100 emulator.
The server maintains authoritative terminal state, so clients can reconnect
without losing history. A WebSocket stream delivers live output; an HTTP
snapshot API lets new clients catch up instantly.

```
┌─────────────────────┐     HTTP + WS      ┌──────────────────────┐
│  Phoenix LiveView   │ ←────────────────→ │   Rust PTY Server    │
│  (port 4000)        │                    │   (port 3000)        │
│                     │  WebSockex client  │                      │
│  PaneSocket ────────┼────────────────→   │  /panes/{id}/stream  │
│  PubSub + LiveView  │                    │  vt100 emulator      │
└─────────────────────┘                    └──────────────────────┘
```

## Quick Start

```bash
# Terminal 1 — Rust PTY server
cd server && cargo run

# Terminal 2 — Phoenix LiveView
cd client && mix phx.server
```

Open `http://localhost:4000`. Click **+ New** to spawn a pane.

## Project Layout

```
biome_term/
├── server/          # Rust — axum HTTP/WebSocket server
│   ├── src/
│   │   ├── main.rs
│   │   ├── lib.rs          # build_router()
│   │   ├── pane.rs         # PTY lifecycle, vt100 parser, event log
│   │   ├── state.rs        # DashMap<Uuid, Arc<Pane>>
│   │   ├── event.rs        # append-only seq-numbered event log
│   │   ├── openapi.rs      # utoipa OpenAPI spec (source of truth for docs)
│   │   └── handlers/       # one file per route
│   ├── src/bin/
│   │   └── gen_docs.rs     # generates docs/api.md from the OpenAPI spec
│   └── tests/
│       └── integration.rs  # E2E tests: echo, events, WebSocket, CRUD
└── client/          # Elixir — Phoenix LiveView
    ├── lib/terminal_ui/
    │   ├── application.ex      # supervision tree
    │   ├── pane_supervisor.ex  # DynamicSupervisor for PaneSockets
    │   ├── pane_socket.ex      # WebSockex → PubSub bridge
    │   └── terminal_client.ex  # Req HTTP wrapper
    └── lib/terminal_ui_web/
        ├── router.ex
        └── live/
            ├── terminal_live.ex
            └── terminal_live.html.heex
```

## API

The Rust server exposes a REST + WebSocket API on **port 3000**.

See **[docs/api.md](docs/api.md)** for the full reference.

The spec is also served live at `GET /openapi.json`.

To regenerate `docs/api.md` after changing handlers:

```bash
cd server && cargo run --bin gen-docs
```

## Running Tests

```bash
cd server && cargo test
```

Four integration tests exercise the full stack: shell spawning, echo,
event log, WebSocket stream, and pane CRUD.
