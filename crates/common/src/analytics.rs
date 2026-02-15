use crate::{EdgeId, NodeDomain, NodeId, TrafficClass};
use serde::{Deserialize, Serialize};

/// Graph-first snapshot for force-directed topology visualizers.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TopologySnapshot {
    /// Monotonic sequence so consumers can detect dropped snapshots.
    pub snapshot_seq: u64,

    /// Absolute wall-clock timestamp (microseconds since UNIX epoch).
    pub snapshot_timestamp_epoch_us: u64,

    /// Time since the previous snapshot in microseconds.
    pub snapshot_interval_us: u64,

    /// Active nodes at snapshot time.
    pub nodes: Vec<NodeSnapshot>,

    /// Active edges at snapshot time.
    pub edges: Vec<EdgeSnapshot>,

    /// Nodes removed since the previous topology snapshot.
    pub removed_nodes: Vec<NodeId>,

    /// Edges removed since the previous topology snapshot.
    pub removed_edges: Vec<EdgeId>,

    /// Global aggregate statistics (kept for dashboard/summary views).
    pub global_stats: GlobalStats,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct NodeSnapshot {
    pub node_id: NodeId,
    pub desc: [u8; 16],
    pub domain: NodeDomain,
    pub first_seen_us: u64,
    pub last_seen_us: u64,
    pub active: bool,
    pub total_packets: u64,
    pub total_bytes: u64,
    pub total_pps: f64,
    pub total_bps: f64,
    pub latency: LatencyMetrics,
    pub loss: LossMetrics,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct EdgeSnapshot {
    pub edge_id: EdgeId,
    pub src_node_id: NodeId,
    pub dst_node_id: NodeId,
    pub class: TrafficClass,
    pub packets: u64,
    pub bytes: u64,
    pub packets_per_second: f64,
    pub bytes_per_second: f64,
    pub delta_packets_per_second: f64,
    pub delta_bytes_per_second: f64,
    pub latency_ewma_us: f64,
    pub latency_delta_us: f64,
    pub jitter_ewma_us: f64,
    pub loss_rate_window: f64,
    pub active: bool,
}

/// Top-level analytics snapshot sent to visualizer
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AnalyticsSnapshot {
    /// Absolute timestamp when snapshot was taken (microseconds since epoch would be better, but using relative for simplicity)
    pub snapshot_timestamp_us: u64,

    /// How long the server has been running
    pub server_uptime_us: u64,

    /// Global aggregate statistics
    pub global_stats: GlobalStats,

    /// Per-client breakdown
    pub per_client_stats: Vec<ClientStats>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GlobalStats {
    /// Total packets received across all clients
    pub total_packets: u64,

    /// Total bytes received across all clients
    pub total_bytes: u64,

    /// Packets broken down by traffic class [Api, HeavyCompute, Background, HealthCheck]
    pub packets_by_class: [u64; 4],

    /// Bytes broken down by traffic class
    pub bytes_by_class: [u64; 4],

    /// Packets/bytes grouped by route:
    /// [internal->internal, internal->external, external->internal, external->external]
    pub route_stats: [RouteStats; 4],

    /// Number of unique client addresses seen
    pub unique_clients: usize,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ClientStats {
    /// id
    pub node_id: [u8; 16],

    /// description
    pub desc: [u8; 16],

    /// Client address as string (e.g., "127.0.0.1:52341")
    pub addr: String,

    /// When this client first sent a packet (relative to server start, in microseconds)
    pub first_seen_us: u64,

    /// When this client last sent a packet (relative to server start)
    pub last_seen_us: u64,

    /// How long this client has been active
    pub session_duration_us: u64,

    /// Statistics broken down by traffic class
    pub class_stats: [ClassStats; 4],

    /// Latency measurements (RTT and jitter)
    pub latency: LatencyMetrics,

    /// Packet loss detection results
    pub loss: LossMetrics,

    /// Packets/bytes grouped by route:
    /// [internal->internal, internal->external, external->internal, external->external]
    pub route_stats: [RouteStats; 4],
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, Default)]
pub struct RouteStats {
    pub packets: u64,
    pub bytes: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, Default)]
pub struct ClassStats {
    /// Total packets received for this class
    pub packets: u64,

    /// Total bytes received for this class
    pub bytes: u64,

    /// Current rate (5-second sliding window)
    pub packets_per_second: f64,

    /// Current bandwidth (5-second sliding window)
    pub bytes_per_second: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, Default)]
pub struct LatencyMetrics {
    /// Minimum RTT observed (microseconds)
    pub min_rtt_us: u64,

    /// Maximum RTT observed (microseconds)
    pub max_rtt_us: u64,

    /// Average RTT (mean)
    pub mean_rtt_us: f64,

    /// Average jitter (mean absolute difference between consecutive RTTs)
    pub mean_jitter_us: f64,

    /// Number of RTT samples collected
    pub samples: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct LossMetrics {
    /// Total number of missing packets detected
    pub missing_sequences: u64,

    /// Number of out-of-order packets
    pub out_of_order: u64,

    /// Number of duplicate packets
    pub duplicates: u64,

    /// Number of gaps in sequence (each gap may contain multiple missing packets)
    pub total_gaps: usize,
}
