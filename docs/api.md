# Terminal Server API

> **Version:** 0.1.0

HTTP + WebSocket API for managing persistent PTY sessions backed by a VT100 emulator. Each pane is an independent shell process; the server maintains authoritative terminal state so clients can reconnect without losing history.

---

## Endpoints

### `GET /panes`

List all active panes.

**Responses**

| Status | Description |
|--------|-------------|
| `200` | Array of active panes |

**Response Body (array items)**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `cols` | int32 | ✓ |  |
| `id` | uuid | ✓ | Unique pane identifier |
| `name` | string | — | Human-readable label, if provided at creation |
| `rows` | int32 | ✓ |  |
| `terminated` | boolean | ✓ | True when the shell process has exited |

---

### `POST /panes`

Create a new PTY pane.

Spawns a shell process attached to a pseudo-terminal. The pane ID is used
in all subsequent requests. The VT100 emulator begins processing output
immediately; connect via `/panes/{id}/stream` to receive live updates.

**Request Body** (`application/json`)

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `cols` | int32 | — | Terminal width in columns (default: 220) |
| `name` | string | — | Human-readable label for this pane (optional) |
| `rows` | int32 | — | Terminal height in rows (default: 50) |
| `shell` | string | — | Shell executable path (default: /bin/bash) |

**Responses**

| Status | Description |
|--------|-------------|
| `200` | Pane created |
| `500` | Failed to open PTY or spawn shell |

**Response Body**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `cols` | int32 | ✓ |  |
| `id` | uuid | ✓ | Unique pane identifier |
| `name` | string | — | Human-readable label, if provided at creation |
| `rows` | int32 | ✓ |  |

---

### `DELETE /panes/{id}`

Kill and remove a pane.

Sends SIGKILL to the shell process and removes the pane from the active set.
Any connected WebSocket subscribers will receive a `Closed` error.

**Path Parameters**

| Name | Type | Description |
|------|------|-------------|
| `id` | uuid | Pane ID |

**Responses**

| Status | Description |
|--------|-------------|
| `204` | Pane killed and removed |
| `404` | Pane not found |

---

### `GET /panes/{id}/events`

Fetch PTY output events for a pane.

Returns the append-only event log since sequence number `after`.
Sequence numbers are 1-indexed; `after=0` returns all events.
For a live stream use `GET /panes/{id}/stream` (WebSocket).

**Path Parameters**

| Name | Type | Description |
|------|------|-------------|
| `id` | uuid | Pane ID |
| `after` | int64 | Return only events with `seq` greater than this value. Use `0` (default) to get all events. |

**Responses**

| Status | Description |
|--------|-------------|
| `200` | Events since `after` |
| `404` | Pane not found |

**Response Body (array items)**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `data` | string | ✓ | Base64-encoded raw PTY output bytes |
| `seq` | int64 | ✓ | Monotonically increasing sequence number (1-indexed) |
| `timestamp_ms` | int64 | ✓ | Unix timestamp in milliseconds |

---

### `POST /panes/{id}/input`

Write bytes to a pane's PTY stdin.

`data` must be base64-encoded. Any byte sequence is accepted, including
ANSI/VT escape sequences (e.g. `\x1b[A` for arrow-up).

**Path Parameters**

| Name | Type | Description |
|------|------|-------------|
| `id` | uuid | Pane ID |

**Request Body** (`application/json`)

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `data` | string | ✓ | Base64-encoded bytes to write verbatim to the PTY (supports escape sequences) |

**Responses**

| Status | Description |
|--------|-------------|
| `204` | Input written to PTY |
| `400` | Invalid base64 |
| `404` | Pane not found |

---

### `POST /panes/{id}/resize`

Resize a pane's terminal dimensions.

Resizes the PTY master (triggering SIGWINCH so the shell redraws), then
replaces the VT100 parser with a fresh instance at the new size.
The shell prompt is typically redrawn within milliseconds.

**Path Parameters**

| Name | Type | Description |
|------|------|-------------|
| `id` | uuid | Pane ID |

**Request Body** (`application/json`)

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `cols` | int32 | ✓ |  |
| `rows` | int32 | ✓ |  |

**Responses**

| Status | Description |
|--------|-------------|
| `204` | Pane resized |
| `404` | Pane not found |
| `500` | PTY resize syscall failed |

---

### `GET /panes/{id}/screen`

Get the current screen state of a pane.

Returns the authoritative VT100-emulated screen buffer. Each entry in
`rows` is one terminal line with trailing whitespace stripped.
This is a snapshot; subscribe to `/panes/{id}/stream` for live updates.

**Path Parameters**

| Name | Type | Description |
|------|------|-------------|
| `id` | uuid | Pane ID |

**Responses**

| Status | Description |
|--------|-------------|
| `200` | Current screen state |
| `404` | Pane not found |

**Response Body**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `cursor_col` | int32 | ✓ | Zero-based cursor column |
| `cursor_row` | int32 | ✓ | Zero-based cursor row |
| `num_cols` | int32 | ✓ |  |
| `num_rows` | int32 | ✓ |  |
| `rows` | string[] | ✓ | One string per terminal row, trailing whitespace trimmed |

---

### `GET /panes/{id}/stream`

Subscribe to live PTY output for a pane.

Upgrades the connection to a WebSocket. Historical events are sent first
(subscribe before reading history avoids a race), then new events are
forwarded in real time.

**Frame format** (text, JSON):
```json
{ "seq": 42, "timestamp_ms": 1700000000000, "data": "<base64>" }
```
`data` is base64-encoded raw PTY output bytes.

**Path Parameters**

| Name | Type | Description |
|------|------|-------------|
| `id` | uuid | Pane ID |

**Responses**

| Status | Description |
|--------|-------------|
| `101` | WebSocket upgrade — streams `{seq, timestamp_ms, data}` JSON frames |
| `404` | Pane not found |

---

## Schemas

### `CreatePaneRequest`

Request body for creating a new pane.

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `cols` | int32 | — | Terminal width in columns (default: 220) |
| `name` | string | — | Human-readable label for this pane (optional) |
| `rows` | int32 | — | Terminal height in rows (default: 50) |
| `shell` | string | — | Shell executable path (default: /bin/bash) |

### `CreatePaneResponse`

Created pane descriptor.

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `cols` | int32 | ✓ |  |
| `id` | uuid | ✓ | Unique pane identifier |
| `name` | string | — | Human-readable label, if provided at creation |
| `rows` | int32 | ✓ |  |

### `EventResponse`

A single PTY output event.

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `data` | string | ✓ | Base64-encoded raw PTY output bytes |
| `seq` | int64 | ✓ | Monotonically increasing sequence number (1-indexed) |
| `timestamp_ms` | int64 | ✓ | Unix timestamp in milliseconds |

### `InputRequest`

Input to write to a pane's PTY stdin.

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `data` | string | ✓ | Base64-encoded bytes to write verbatim to the PTY (supports escape sequences) |

### `PaneInfo`

Summary of an active pane.

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `cols` | int32 | ✓ |  |
| `id` | uuid | ✓ | Unique pane identifier |
| `name` | string | — | Human-readable label, if provided at creation |
| `rows` | int32 | ✓ |  |
| `terminated` | boolean | ✓ | True when the shell process has exited |

### `ResizeRequest`

New terminal dimensions.

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `cols` | int32 | ✓ |  |
| `rows` | int32 | ✓ |  |

### `ScreenResponse`

Authoritative VT100 screen state of a pane.

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `cursor_col` | int32 | ✓ | Zero-based cursor column |
| `cursor_row` | int32 | ✓ | Zero-based cursor row |
| `num_cols` | int32 | ✓ |  |
| `num_rows` | int32 | ✓ |  |
| `rows` | string[] | ✓ | One string per terminal row, trailing whitespace trimmed |

