# Ripple

A network traffic simulator and real-time topology visualization testbed written in Rust.

**simd** simulates a distributed system of communicating nodes and feeds a server-side analytics engine that tracks the full network graph — nodes, edges, latency, loss, and throughput — for testing force-directed topology visualizers.

---

## Overview

The project is structured as a Cargo workspace with three crates:

```
simd/
├── crates/common/   # Shared protocol types and serialization
├── crates/server/   # UDP server and analytics engine
└── crates/client/   # Interactive CLI simulator
```

The server listens for UDP packets from one or more clients. Clients register themselves, declare peers, and emit typed traffic packets. The server aggregates these into a live graph model and serves topology snapshots on demand. The snapshots are designed to drive external visualization tools with smooth incremental updates.

---

## Architecture

### `common`

Defines the wire protocol shared between server and client.

- **Serialization**: [postcard](https://github.com/jamesmunns/postcard) (compact binary, Serde-backed)
- **Message types**: `RegisterNode`, `UnregisterNode`, `Data`, `Ack`, `RequestTopology`, `Topology`, `RequestAnalytics`, `Analytics`
- **Core types**: `NodeId` (16-byte stable identity), `TrafficClass`, `NodeDomain`, `EndpointDomain`
- **`TopologySnapshot`**: graph-first snapshot format including nodes, edges, removed items, delta rates, and global stats

### `server`

UDP listener and analytics engine.

- Binds a UDP socket and dispatches all `WireMessage` variants
- **`AnalyticsManager`**: core in-memory graph engine
  - `NodeId → NodeState`: per-node metrics by traffic class, domain, first/last seen
  - `(src, dst, class) → EdgeState`: per-edge packet/byte counters, EWMA latency and jitter, loss tracking
  - `RateCalculator`: 5-second sliding window with 1-second buckets
  - `SequenceTracker`: detects loss, out-of-order, and duplicate packets per traffic class
- Periodic cleanup every 1 second (node TTL: 60 s, edge TTL: 30 s)
- Exports both graph-native (`TopologySnapshot`) and legacy (`AnalyticsSnapshot`) formats

### `client`

Interactive keyboard-driven CLI simulator.

- Loads or creates a persistent `NodeId` (UUID stored in `client_id.txt`)
- Sends `RegisterNode`/`UnregisterNode` to manage its own lifecycle
- Manages a list of peers (each also registered as nodes)
- Emits `DataPacket` messages toward a selected peer
- Receives `Ack` packets and displays RTT statistics
- Requests and displays topology/analytics snapshots in the terminal

---

## Protocol

Transport is UDP. All messages are encoded with postcard. Communication is fire-and-forget except for `Ack` (used for RTT measurement only, not reliability).

```
Client                                   Server
  │
  ├─→ RegisterNode(node_id, desc, domain)  → create/update NodeState
  │
  ├─→ Data(src, dst, class, seq, bytes)    → update edge metrics
  │    ← Ack(seq, server_ts, proc_us)
  │
  ├─→ UnregisterNode(node_id)              → remove node and connected edges
  │
  ├─→ RequestTopology                      → export TopologySnapshot
  │    ← Topology(snapshot)
  │
  └─→ RequestAnalytics (legacy)            → export AnalyticsSnapshot
       ← Analytics(snapshot)
```

### Key protocol properties

- **Explicit node lifecycle**: `RegisterNode`/`UnregisterNode` give the server immediate topology awareness
- **Endpoint-routed data**: `DataPacket` specifies `src`/`dst` node IDs directly, not derived from the UDP source address
- **Multi-class traffic**: each packet is tagged with a `TrafficClass` — `Api`, `HeavyCompute`, `Background`, or `HealthCheck`
- **Domain-aware**: nodes are marked `Internal` or `External`, enabling route classification
- **Graph deltas**: `TopologySnapshot` includes `removed_nodes` and `removed_edges` for incremental visualization updates

---

## Getting Started

### Prerequisites

- Rust toolchain (stable, 2021 edition or later)

### Build

```sh
cargo build --release
```

### Run the server

```sh
cargo run -p server -- [-s <host>] [-p <port>]
```

Defaults to `127.0.0.1:8080`.

### Run the client

```sh
cargo run -p client -- [-s <host>] [-p <port>]
```

Connects to `127.0.0.1:8080` by default.

---

## Client Keyboard Controls

The client uses an alternate terminal screen with keyboard commands:

| Key | Action |
|-----|--------|
| `Space` | Send a single packet to the active peer |
| `b` | Send a burst of packets |
| `1`–`9` | Set burst count |
| `a` | Start continuous API traffic |
| `h` | Start continuous HeavyCompute traffic |
| `g` | Start continuous Background traffic |
| `s` | Stop continuous mode |
| `v` | Register self |
| `x` | Unregister self |
| `n` | Add an internal peer |
| `j` | Add an external peer |
| `c` | Cycle to next peer |
| `m` | Remove active peer |
| `i` / `e` | Set source domain to Internal / External |
| `k` / `l` | Set destination domain to Internal / External |
| `f` | Traffic profile: steady rate |
| `z` | Traffic profile: burst |
| `w` | Traffic profile: ramp (20 → 220 pps) |
| `o` | Traffic profile: oscillation (40 ↔ 240 pps) |
| `r` | Request analytics snapshot |
| `p` | Request topology snapshot |
| `t` | Run topology smoke test |
| `y` | Run topology node-removal test |
| `u` | Run topology mixed-classes test |
| `q` | Quit |

---

## Topology Snapshot Format

`TopologySnapshot` is the primary output of the server, designed for visualization consumption:

```rust
TopologySnapshot {
    seq: u64,
    timestamp_us: u64,
    nodes: Vec<NodeSnapshot>,
    edges: Vec<EdgeSnapshot>,
    removed_nodes: Vec<NodeId>,
    removed_edges: Vec<EdgeId>,
    global_stats: GlobalStats,
}
```

Each `EdgeSnapshot` includes:
- Packet and byte delta rates (pps, bps)
- EWMA latency and jitter
- Latency delta (trend indicator)
- Loss rate over the last window

Removed items are included once in the snapshot immediately following cleanup, enabling visualizers to animate node/edge removal without polling.

---

## Metrics

### Per-edge
- EWMA latency and jitter (α = 0.2)
- Latency delta (current sample vs. EWMA, for trend)
- Packet and byte rates (5-second sliding window)
- Loss rate per traffic class

### Per-node
- Total and per-class packet/byte counts
- Active state (seen within 3× window)
- Domain (Internal / External)

---

## Testing

```sh
cargo test
```

Tests cover:
- Protocol round-trips (all message types encode/decode correctly)
- Node registration and domain stability
- Edge creation and metric updates
- Cleanup and TTL-based removal
- Latency and delta rate calculations
- Full integration flow: register → send → unregister → topology request

---

## Design Notes

- **Single-threaded**: both server and client use a blocking event loop with a 250 ms poll timeout; no async runtime
- **No reliability layer**: ACKs are informational only (RTT measurement); lost packets are counted but not retransmitted
- **Server-side TTL cleanup**: prevents ghost nodes if clients crash without unregistering
- **Stable identity**: `NodeId` is a 16-byte value (typically a UUID) persisted on the client, decoupled from the UDP source address

---

## Current Limitations

- **No persistence**: server state is in-memory; lost on restart
- **No visualization frontend**: the server streams snapshots; a renderer is expected externally
- **No encryption or authentication**: plain UDP
- **Single server**: no federation or replication
- **Simulator only**: no live packet capture from real network interfaces

---

## License

This project is licensed under the [MIT License](LICENSE).

Copyright (c) 2026 Marcio Garcia
