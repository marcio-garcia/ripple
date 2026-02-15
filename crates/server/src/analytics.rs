use crate::client::{LatencyStats, LossEvent, RateCalculator, SequenceTracker};
use common::{
    AckPacket, DataPacket, EdgeId, NodeDomain, NodeId, RegisterNodePacket, TrafficClass,
    UnregisterNodePacket,
};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::net::SocketAddr;
use std::time::{Duration, Instant, SystemTime};

/// Main analytics engine tracking graph topology and aggregate stats.
pub struct AnalyticsManager {
    start_time: Instant,
    nodes: HashMap<NodeId, NodeState>,
    edges: HashMap<EdgeKey, EdgeState>,
    total_packets: u64,
    total_bytes: u64,
    packets_by_class: [u64; 4],
    bytes_by_class: [u64; 4],
    route_packets: [u64; 4],
    route_bytes: [u64; 4],
    rate_window_secs: u32,
    max_nodes: usize,
    snapshot_seq: u64,
    last_topology_epoch_us: u64,
    removed_nodes_since_last_snapshot: Vec<NodeId>,
    removed_edges_since_last_snapshot: Vec<EdgeId>,
}

#[derive(Hash, Eq, PartialEq, Clone, Copy)]
struct EdgeKey {
    src_node_id: NodeId,
    dst_node_id: NodeId,
    class: TrafficClass,
}

struct NodeState {
    node_id: NodeId,
    desc: [u8; 16],
    domain: NodeDomain,
    addr: SocketAddr,
    first_seen: Instant,
    last_seen: Instant,
    seq_trackers: [SequenceTracker; 4],
    packets_by_class: [u64; 4],
    bytes_by_class: [u64; 4],
    route_packets: [u64; 4],
    route_bytes: [u64; 4],
    latency_stats: LatencyStats,
    rate_calculators: [RateCalculator; 4],
}

impl NodeState {
    fn new(
        node_id: NodeId,
        desc: [u8; 16],
        domain: NodeDomain,
        addr: SocketAddr,
        now: Instant,
        window_secs: u32,
    ) -> Self {
        Self {
            node_id,
            desc,
            domain,
            addr,
            first_seen: now,
            last_seen: now,
            seq_trackers: Default::default(),
            packets_by_class: [0; 4],
            bytes_by_class: [0; 4],
            route_packets: [0; 4],
            route_bytes: [0; 4],
            latency_stats: LatencyStats::new(),
            rate_calculators: [
                RateCalculator::new(window_secs),
                RateCalculator::new(window_secs),
                RateCalculator::new(window_secs),
                RateCalculator::new(window_secs),
            ],
        }
    }
}

struct EdgeState {
    edge_id: EdgeId,
    src_node_id: NodeId,
    dst_node_id: NodeId,
    class: TrafficClass,
    last_seen: Instant,
    packets: u64,
    bytes: u64,
    rate_calculator: RateCalculator,
    seq_tracker: SequenceTracker,
    prev_packets_per_second: f64,
    prev_bytes_per_second: f64,
    latency_ewma_us: f64,
    jitter_ewma_us: f64,
    latency_delta_us: f64,
    last_latency_sample_us: Option<f64>,
    window_packets: u64,
    window_missing: u64,
}

impl EdgeState {
    fn new(key: EdgeKey, now: Instant, window_secs: u32) -> Self {
        Self {
            edge_id: edge_id_from_key(key),
            src_node_id: key.src_node_id,
            dst_node_id: key.dst_node_id,
            class: key.class,
            last_seen: now,
            packets: 0,
            bytes: 0,
            rate_calculator: RateCalculator::new(window_secs),
            seq_tracker: SequenceTracker::default(),
            prev_packets_per_second: 0.0,
            prev_bytes_per_second: 0.0,
            latency_ewma_us: 0.0,
            jitter_ewma_us: 0.0,
            latency_delta_us: 0.0,
            last_latency_sample_us: None,
            window_packets: 0,
            window_missing: 0,
        }
    }
}

impl AnalyticsManager {
    pub fn new(window_secs: u32, max_nodes: usize) -> Self {
        let start_epoch_us = epoch_timestamp_us();
        Self {
            start_time: Instant::now(),
            nodes: HashMap::new(),
            edges: HashMap::new(),
            total_packets: 0,
            total_bytes: 0,
            packets_by_class: [0; 4],
            bytes_by_class: [0; 4],
            route_packets: [0; 4],
            route_bytes: [0; 4],
            rate_window_secs: window_secs,
            max_nodes,
            snapshot_seq: 0,
            last_topology_epoch_us: start_epoch_us,
            removed_nodes_since_last_snapshot: Vec::new(),
            removed_edges_since_last_snapshot: Vec::new(),
        }
    }

    pub fn on_node_registered(
        &mut self,
        packet: &RegisterNodePacket,
        src: SocketAddr,
        now: Instant,
    ) {
        if !self.nodes.contains_key(&packet.node_id) && self.nodes.len() >= self.max_nodes {
            return;
        }

        let node = self.nodes.entry(packet.node_id).or_insert_with(|| {
            NodeState::new(
                packet.node_id,
                packet.desc,
                packet.domain,
                src,
                now,
                self.rate_window_secs,
            )
        });

        node.addr = src;
        node.desc = packet.desc;
        node.domain = packet.domain;
        node.last_seen = now;
    }

    pub fn on_node_unregistered(&mut self, packet: &UnregisterNodePacket, _now: Instant) {
        self.remove_node_and_edges(packet.node_id);
    }

    pub fn on_packet_received(
        &mut self,
        src: SocketAddr,
        packet: &DataPacket,
        now: Instant,
    ) -> AckPacket {
        let src_node_id = packet.src_node_id;
        let dst_node_id = packet.dst_node_id;
        let class_idx = packet.class as usize;

        self.ensure_node(
            src_node_id,
            packet.desc,
            infer_domain(src_node_id),
            src,
            now,
            true,
        );
        self.ensure_node(
            dst_node_id,
            domain_desc(infer_domain(dst_node_id)),
            infer_domain(dst_node_id),
            src,
            now,
            false,
        );
        let src_domain = self
            .nodes
            .get(&src_node_id)
            .map(|node| node.domain)
            .unwrap_or_else(|| infer_domain(src_node_id));
        let dst_domain = self
            .nodes
            .get(&dst_node_id)
            .map(|node| node.domain)
            .unwrap_or_else(|| infer_domain(dst_node_id));
        let route_idx = route_index(src_domain, dst_domain);

        self.total_packets += 1;
        self.total_bytes += packet.declared_bytes as u64;
        self.packets_by_class[class_idx] += 1;
        self.bytes_by_class[class_idx] += packet.declared_bytes as u64;
        self.route_packets[route_idx] += 1;
        self.route_bytes[route_idx] += packet.declared_bytes as u64;

        if let Some(node) = self.nodes.get_mut(&src_node_id) {
            node.last_seen = now;
            node.addr = src;
            node.desc = packet.desc;
            node.packets_by_class[class_idx] += 1;
            node.bytes_by_class[class_idx] += packet.declared_bytes as u64;
            node.route_packets[route_idx] += 1;
            node.route_bytes[route_idx] += packet.declared_bytes as u64;

            let loss_event = node.seq_trackers[class_idx].process_sequence(packet.class_seq, now);
            if let LossEvent::Loss { count } = loss_event {
                println!(
                    "Loss detected on node {:?}: {} packets missing",
                    src_node_id, count
                );
            }

            node.rate_calculators[class_idx].record_packet(now, packet.declared_bytes);
        }

        if let Some(node) = self.nodes.get_mut(&dst_node_id) {
            node.last_seen = now;
        }

        let key = EdgeKey {
            src_node_id,
            dst_node_id,
            class: packet.class,
        };
        let edge = self
            .edges
            .entry(key)
            .or_insert_with(|| EdgeState::new(key, now, self.rate_window_secs));
        edge.last_seen = now;
        edge.packets += 1;
        edge.bytes += packet.declared_bytes as u64;
        edge.window_packets += 1;
        edge.rate_calculator
            .record_packet(now, packet.declared_bytes);

        let edge_loss_event = edge.seq_tracker.process_sequence(packet.class_seq, now);
        if let LossEvent::Loss { count } = edge_loss_event {
            edge.window_missing += count;
        }

        let server_timestamp_us = epoch_timestamp_us();
        if server_timestamp_us >= packet.timestamp_us {
            let latency_us = (server_timestamp_us - packet.timestamp_us) as f64;
            if let Some(src_node) = self.nodes.get_mut(&src_node_id) {
                src_node.latency_stats.add_rtt_sample(latency_us as u64);
            }
            update_edge_latency(edge, latency_us);
        }

        AckPacket {
            original_seq: packet.global_seq,
            server_timestamp_us,
            server_processing_us: 0,
        }
    }

    pub fn cleanup_stale(&mut self, node_ttl: Duration, edge_ttl: Duration, now: Instant) {
        let stale_nodes: Vec<NodeId> = self
            .nodes
            .iter()
            .filter(|(_, node)| now.duration_since(node.last_seen) >= node_ttl)
            .map(|(node_id, _)| *node_id)
            .collect();

        for node_id in stale_nodes {
            self.remove_node_and_edges(node_id);
        }

        let stale_edges: Vec<EdgeKey> = self
            .edges
            .iter()
            .filter(|(_, edge)| now.duration_since(edge.last_seen) >= edge_ttl)
            .map(|(key, _)| *key)
            .collect();

        for key in stale_edges {
            if let Some(edge) = self.edges.remove(&key) {
                self.removed_edges_since_last_snapshot.push(edge.edge_id);
            }
        }
    }

    pub fn cleanup_stale_clients(&mut self, timeout: Duration) {
        self.cleanup_stale(timeout, timeout, Instant::now());
    }

    pub fn export_topology_snapshot(
        &mut self,
        now: Instant,
    ) -> common::analytics::TopologySnapshot {
        self.snapshot_seq = self.snapshot_seq.saturating_add(1);
        let snapshot_timestamp_epoch_us = epoch_timestamp_us();
        let snapshot_interval_us =
            snapshot_timestamp_epoch_us.saturating_sub(self.last_topology_epoch_us);
        self.last_topology_epoch_us = snapshot_timestamp_epoch_us;
        let activity_ttl = Duration::from_secs((self.rate_window_secs as u64).saturating_mul(3));

        let nodes: Vec<_> = self
            .nodes
            .values()
            .map(|node| {
                let (total_pps, total_bps) = total_rate_for_node(node, now);
                common::analytics::NodeSnapshot {
                    node_id: node.node_id,
                    desc: node.desc,
                    domain: node.domain,
                    first_seen_us: node.first_seen.duration_since(self.start_time).as_micros()
                        as u64,
                    last_seen_us: node.last_seen.duration_since(self.start_time).as_micros() as u64,
                    active: now.duration_since(node.last_seen) < activity_ttl,
                    total_packets: node.packets_by_class.iter().sum(),
                    total_bytes: node.bytes_by_class.iter().sum(),
                    total_pps,
                    total_bps,
                    latency: latency_metrics_from_stats(&node.latency_stats),
                    loss: loss_metrics_from_trackers(&node.seq_trackers),
                }
            })
            .collect();

        let mut edges: Vec<common::analytics::EdgeSnapshot> = Vec::with_capacity(self.edges.len());
        for edge in self.edges.values_mut() {
            let (pps, bps) = edge.rate_calculator.calculate_rate(now);
            let delta_pps = pps - edge.prev_packets_per_second;
            let delta_bps = bps - edge.prev_bytes_per_second;
            edge.prev_packets_per_second = pps;
            edge.prev_bytes_per_second = bps;
            let loss_rate_window = if edge.window_packets == 0 {
                0.0
            } else {
                edge.window_missing as f64 / edge.window_packets as f64
            };
            edge.window_packets = 0;
            edge.window_missing = 0;

            edges.push(common::analytics::EdgeSnapshot {
                edge_id: edge.edge_id,
                src_node_id: edge.src_node_id,
                dst_node_id: edge.dst_node_id,
                class: edge.class,
                packets: edge.packets,
                bytes: edge.bytes,
                packets_per_second: pps,
                bytes_per_second: bps,
                delta_packets_per_second: delta_pps,
                delta_bytes_per_second: delta_bps,
                latency_ewma_us: edge.latency_ewma_us,
                latency_delta_us: edge.latency_delta_us,
                jitter_ewma_us: edge.jitter_ewma_us,
                loss_rate_window,
                active: now.duration_since(edge.last_seen) < activity_ttl,
            });
        }

        common::analytics::TopologySnapshot {
            snapshot_seq: self.snapshot_seq,
            snapshot_timestamp_epoch_us,
            snapshot_interval_us,
            nodes,
            edges,
            removed_nodes: std::mem::take(&mut self.removed_nodes_since_last_snapshot),
            removed_edges: std::mem::take(&mut self.removed_edges_since_last_snapshot),
            global_stats: self.global_stats(),
        }
    }

    pub fn export_snapshot(&self) -> common::analytics::AnalyticsSnapshot {
        let now = Instant::now();
        let uptime = now.duration_since(self.start_time).as_micros() as u64;
        let per_client_stats = self
            .nodes
            .values()
            .map(|node| {
                let class_stats: [common::analytics::ClassStats; 4] = std::array::from_fn(|i| {
                    let (pps, bps) = node.rate_calculators[i].calculate_rate(now);
                    common::analytics::ClassStats {
                        packets: node.packets_by_class[i],
                        bytes: node.bytes_by_class[i],
                        packets_per_second: pps,
                        bytes_per_second: bps,
                    }
                });

                common::analytics::ClientStats {
                    node_id: node.node_id,
                    desc: node.desc,
                    addr: node.addr.to_string(),
                    first_seen_us: node.first_seen.duration_since(self.start_time).as_micros()
                        as u64,
                    last_seen_us: node.last_seen.duration_since(self.start_time).as_micros() as u64,
                    session_duration_us: node.last_seen.duration_since(node.first_seen).as_micros()
                        as u64,
                    class_stats,
                    latency: latency_metrics_from_stats(&node.latency_stats),
                    loss: loss_metrics_from_trackers(&node.seq_trackers),
                    route_stats: std::array::from_fn(|i| common::analytics::RouteStats {
                        packets: node.route_packets[i],
                        bytes: node.route_bytes[i],
                    }),
                }
            })
            .collect();

        common::analytics::AnalyticsSnapshot {
            snapshot_timestamp_us: uptime,
            server_uptime_us: uptime,
            global_stats: self.global_stats(),
            per_client_stats,
        }
    }

    fn ensure_node(
        &mut self,
        node_id: NodeId,
        desc: [u8; 16],
        domain: NodeDomain,
        addr: SocketAddr,
        now: Instant,
        refresh_desc: bool,
    ) {
        if !self.nodes.contains_key(&node_id) && self.nodes.len() >= self.max_nodes {
            return;
        }

        let node = self.nodes.entry(node_id).or_insert_with(|| {
            NodeState::new(node_id, desc, domain, addr, now, self.rate_window_secs)
        });

        if refresh_desc {
            node.desc = desc;
        }
        node.addr = addr;
        node.last_seen = now;
    }

    fn remove_node_and_edges(&mut self, node_id: NodeId) {
        if self.nodes.remove(&node_id).is_none() {
            return;
        }

        self.removed_nodes_since_last_snapshot.push(node_id);
        let mut removed_edge_ids = HashSet::new();
        let to_remove: Vec<EdgeKey> = self
            .edges
            .iter()
            .filter(|(_, edge)| edge.src_node_id == node_id || edge.dst_node_id == node_id)
            .map(|(key, _)| *key)
            .collect();

        for key in to_remove {
            if let Some(edge) = self.edges.remove(&key) {
                removed_edge_ids.insert(edge.edge_id);
            }
        }

        self.removed_edges_since_last_snapshot
            .extend(removed_edge_ids);
    }

    fn global_stats(&self) -> common::analytics::GlobalStats {
        common::analytics::GlobalStats {
            total_packets: self.total_packets,
            total_bytes: self.total_bytes,
            packets_by_class: self.packets_by_class,
            bytes_by_class: self.bytes_by_class,
            route_stats: std::array::from_fn(|i| common::analytics::RouteStats {
                packets: self.route_packets[i],
                bytes: self.route_bytes[i],
            }),
            unique_clients: self.nodes.len(),
        }
    }
}

fn total_rate_for_node(node: &NodeState, now: Instant) -> (f64, f64) {
    node.rate_calculators.iter().fold((0.0, 0.0), |acc, calc| {
        let (pps, bps) = calc.calculate_rate(now);
        (acc.0 + pps, acc.1 + bps)
    })
}

fn latency_metrics_from_stats(latency_stats: &LatencyStats) -> common::analytics::LatencyMetrics {
    common::analytics::LatencyMetrics {
        min_rtt_us: latency_stats.min_rtt_us,
        max_rtt_us: latency_stats.max_rtt_us,
        mean_rtt_us: latency_stats.mean_rtt_us(),
        mean_jitter_us: latency_stats.mean_jitter_us(),
        samples: latency_stats.count,
    }
}

fn loss_metrics_from_trackers(
    seq_trackers: &[SequenceTracker; 4],
) -> common::analytics::LossMetrics {
    let mut missing_seqs = 0u64;
    let mut out_of_order = 0u64;
    let mut duplicates = 0u64;
    let mut total_gaps = 0usize;

    for tracker in seq_trackers {
        for gap in &tracker.missing_sequences {
            missing_seqs += (gap.end - gap.start + 1) as u64;
        }
        total_gaps += tracker.missing_sequences.len();
        out_of_order += tracker.out_of_order_count;
        duplicates += tracker.duplicate_count;
    }

    common::analytics::LossMetrics {
        missing_sequences: missing_seqs,
        out_of_order,
        duplicates,
        total_gaps,
    }
}

fn domain_desc(domain: NodeDomain) -> [u8; 16] {
    match domain {
        NodeDomain::Internal => *b"internal-node---",
        NodeDomain::External => *b"external-node---",
    }
}

fn route_index(src_domain: NodeDomain, dst_domain: NodeDomain) -> usize {
    match (src_domain, dst_domain) {
        (NodeDomain::Internal, NodeDomain::Internal) => 0,
        (NodeDomain::Internal, NodeDomain::External) => 1,
        (NodeDomain::External, NodeDomain::Internal) => 2,
        (NodeDomain::External, NodeDomain::External) => 3,
    }
}

fn infer_domain(node_id: NodeId) -> NodeDomain {
    if &node_id[..4] == b"peer" && node_id[4] == b'e' {
        return NodeDomain::External;
    }
    if &node_id[..4] == b"peer" && node_id[4] == b'i' {
        return NodeDomain::Internal;
    }
    NodeDomain::Internal
}

fn update_edge_latency(edge: &mut EdgeState, latency_us: f64) {
    const LATENCY_ALPHA: f64 = 0.2;
    const JITTER_ALPHA: f64 = 0.2;

    if edge.latency_ewma_us == 0.0 {
        edge.latency_ewma_us = latency_us;
    } else {
        edge.latency_ewma_us =
            LATENCY_ALPHA * latency_us + (1.0 - LATENCY_ALPHA) * edge.latency_ewma_us;
    }

    if let Some(prev) = edge.last_latency_sample_us {
        let jitter_sample = (latency_us - prev).abs();
        if edge.jitter_ewma_us == 0.0 {
            edge.jitter_ewma_us = jitter_sample;
        } else {
            edge.jitter_ewma_us =
                JITTER_ALPHA * jitter_sample + (1.0 - JITTER_ALPHA) * edge.jitter_ewma_us;
        }
    }

    edge.latency_delta_us = latency_us - edge.latency_ewma_us;
    edge.last_latency_sample_us = Some(latency_us);
}

fn edge_id_from_key(key: EdgeKey) -> EdgeId {
    let mut first = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut first);
    let first_hash = first.finish();

    let mut second = std::collections::hash_map::DefaultHasher::new();
    0x9E37_79B9_7F4A_7C15u64.hash(&mut second);
    key.hash(&mut second);
    let second_hash = second.finish();

    let mut edge_id = [0u8; 16];
    edge_id[..8].copy_from_slice(&first_hash.to_be_bytes());
    edge_id[8..].copy_from_slice(&second_hash.to_be_bytes());
    edge_id
}

fn epoch_timestamp_us() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("System time before UNIX epoch")
        .as_micros() as u64
}

#[cfg(test)]
mod tests {
    use super::AnalyticsManager;
    use common::{NodeDomain, NodeId, TrafficClass, WireMessage};
    use std::net::SocketAddr;
    use std::str::FromStr;
    use std::time::{Duration, Instant};

    fn test_addr() -> SocketAddr {
        SocketAddr::from_str("127.0.0.1:41001").expect("valid socket")
    }

    fn register_node(
        analytics: &mut AnalyticsManager,
        node_id: NodeId,
        domain: NodeDomain,
        now: Instant,
    ) {
        let desc = if domain == NodeDomain::External {
            *b"node-external---"
        } else {
            *b"node-internal---"
        };
        let register = common::make_register_node_packet(node_id, desc, domain);
        analytics.on_node_registered(&register, test_addr(), now);
    }

    #[test]
    fn register_creates_node_with_stable_domain() {
        let mut analytics = AnalyticsManager::new(5, 100);
        let now = Instant::now();
        let src_node_id: NodeId = *b"NODE-ALPHA-00001";
        let dst_node_id: NodeId = *b"NODE-BRAVO-00002";
        let addr = test_addr();

        let src_desc = *b"probe-node------";
        let dst_desc = *b"target-node-----";
        let src_register =
            common::make_register_node_packet(src_node_id, src_desc, NodeDomain::External);
        let dst_register =
            common::make_register_node_packet(dst_node_id, dst_desc, NodeDomain::Internal);
        analytics.on_node_registered(&src_register, addr, now);
        analytics.on_node_registered(&dst_register, addr, now);

        let packet = common::make_data_packet(
            src_node_id,
            dst_node_id,
            1,
            1,
            TrafficClass::Api,
            1200,
            src_desc,
        );
        analytics.on_packet_received(addr, &packet, now + Duration::from_millis(10));

        let snapshot = analytics.export_topology_snapshot(now + Duration::from_millis(20));
        let src = snapshot
            .nodes
            .iter()
            .find(|node| node.node_id == src_node_id)
            .expect("source node should exist");
        assert_eq!(src.domain, NodeDomain::External);
    }

    #[test]
    fn data_packet_creates_or_updates_edge() {
        let mut analytics = AnalyticsManager::new(5, 100);
        let now = Instant::now();
        let src_node_id: NodeId = *b"NODE-EDGEA-00001";
        let dst_node_id: NodeId = *b"NODE-EDGEB-00002";
        let src_desc = *b"src-node--------";
        let addr = test_addr();

        register_node(&mut analytics, src_node_id, NodeDomain::Internal, now);
        register_node(&mut analytics, dst_node_id, NodeDomain::External, now);

        let packet1 = common::make_data_packet(
            src_node_id,
            dst_node_id,
            10,
            1,
            TrafficClass::Api,
            1000,
            src_desc,
        );
        let ack1 = analytics.on_packet_received(addr, &packet1, now + Duration::from_millis(10));
        assert_eq!(ack1.original_seq, 10);

        let packet2 = common::make_data_packet(
            src_node_id,
            dst_node_id,
            11,
            2,
            TrafficClass::Api,
            2000,
            src_desc,
        );
        let ack2 = analytics.on_packet_received(addr, &packet2, now + Duration::from_millis(20));
        assert_eq!(ack2.original_seq, 11);

        let packet3 = common::make_data_packet(
            src_node_id,
            dst_node_id,
            12,
            1,
            TrafficClass::HealthCheck,
            300,
            src_desc,
        );
        analytics.on_packet_received(addr, &packet3, now + Duration::from_millis(30));

        let snapshot = analytics.export_topology_snapshot(now + Duration::from_millis(40));
        let api_edge = snapshot
            .edges
            .iter()
            .find(|edge| {
                edge.src_node_id == src_node_id
                    && edge.dst_node_id == dst_node_id
                    && edge.class == TrafficClass::Api
            })
            .expect("api edge should exist");
        assert_eq!(api_edge.packets, 2);
        assert_eq!(api_edge.bytes, 3000);

        let health_edge = snapshot
            .edges
            .iter()
            .find(|edge| {
                edge.src_node_id == src_node_id
                    && edge.dst_node_id == dst_node_id
                    && edge.class == TrafficClass::HealthCheck
            })
            .expect("health edge should exist");
        assert_eq!(health_edge.packets, 1);
    }

    #[test]
    fn cleanup_expires_nodes_and_edges_and_emits_removed_ids() {
        let mut analytics = AnalyticsManager::new(5, 100);
        let now = Instant::now();
        let src_node_id: NodeId = *b"NODE-CLEAN-00001";
        let dst_node_id: NodeId = *b"NODE-CLEAN-00002";
        let src_desc = *b"cleanup-source--";
        let addr = test_addr();

        register_node(&mut analytics, src_node_id, NodeDomain::Internal, now);
        register_node(&mut analytics, dst_node_id, NodeDomain::External, now);

        let packet = common::make_data_packet(
            src_node_id,
            dst_node_id,
            1,
            1,
            TrafficClass::Background,
            1200,
            src_desc,
        );
        analytics.on_packet_received(addr, &packet, now + Duration::from_millis(10));

        let cleanup_time = now + Duration::from_secs(2);
        analytics.cleanup_stale(Duration::from_secs(1), Duration::from_secs(1), cleanup_time);
        let snapshot = analytics.export_topology_snapshot(cleanup_time);

        assert!(snapshot.nodes.is_empty());
        assert!(snapshot.edges.is_empty());
        assert_eq!(snapshot.removed_nodes.len(), 2);
        assert!(snapshot.removed_nodes.contains(&src_node_id));
        assert!(snapshot.removed_nodes.contains(&dst_node_id));
        assert_eq!(snapshot.removed_edges.len(), 1);
    }

    #[test]
    fn snapshot_contains_delta_rates_and_latency_trends() {
        let mut analytics = AnalyticsManager::new(5, 100);
        let now = Instant::now();
        let src_node_id: NodeId = *b"NODE-LATEN-00001";
        let dst_node_id: NodeId = *b"NODE-LATEN-00002";
        let src_desc = *b"latency-source--";
        let addr = test_addr();

        register_node(&mut analytics, src_node_id, NodeDomain::Internal, now);
        register_node(&mut analytics, dst_node_id, NodeDomain::External, now);

        let mut packet1 = common::make_data_packet(
            src_node_id,
            dst_node_id,
            1,
            1,
            TrafficClass::Api,
            1200,
            src_desc,
        );
        packet1.timestamp_us = common::now_timestamp_us().saturating_sub(100_000);
        analytics.on_packet_received(addr, &packet1, now + Duration::from_millis(10));

        let snapshot1 = analytics.export_topology_snapshot(now + Duration::from_millis(100));
        let edge1 = snapshot1
            .edges
            .iter()
            .find(|edge| {
                edge.src_node_id == src_node_id
                    && edge.dst_node_id == dst_node_id
                    && edge.class == TrafficClass::Api
            })
            .expect("edge should exist in snapshot1");
        assert!(edge1.packets_per_second > 0.0);
        assert!(edge1.latency_ewma_us > 0.0);

        let mut packet2 = common::make_data_packet(
            src_node_id,
            dst_node_id,
            2,
            2,
            TrafficClass::Api,
            1200,
            src_desc,
        );
        packet2.timestamp_us = common::now_timestamp_us().saturating_sub(1_000);
        analytics.on_packet_received(addr, &packet2, now + Duration::from_millis(900));

        let snapshot2 = analytics.export_topology_snapshot(now + Duration::from_secs(1));
        let edge2 = snapshot2
            .edges
            .iter()
            .find(|edge| {
                edge.src_node_id == src_node_id
                    && edge.dst_node_id == dst_node_id
                    && edge.class == TrafficClass::Api
            })
            .expect("edge should exist in snapshot2");
        assert_ne!(edge2.delta_packets_per_second, 0.0);
        assert_ne!(edge2.latency_delta_us, 0.0);
        assert!(edge2.jitter_ewma_us > 0.0);
    }

    #[test]
    fn request_topology_wire_roundtrip_includes_graph_state() {
        let mut analytics = AnalyticsManager::new(5, 100);
        let now = Instant::now();
        let node_id = *b"NODE-ALPHA-00001";
        let dst_node_id = *b"NODE-BRAVO-00002";
        let desc = *b"probe-node------";
        let addr = test_addr();

        let register = common::make_register_node_packet(node_id, desc, NodeDomain::Internal);
        analytics.on_node_registered(&register, addr, now);
        register_node(&mut analytics, dst_node_id, NodeDomain::External, now);

        let packet =
            common::make_data_packet(node_id, dst_node_id, 1, 1, TrafficClass::Api, 1200, desc);
        let ack = analytics.on_packet_received(addr, &packet, now);
        assert_eq!(ack.original_seq, 1);

        let req_bytes =
            common::encode_message(&WireMessage::RequestTopology).expect("request should encode");
        let req = common::decode_message(&req_bytes).expect("request should decode");
        let response = match req {
            WireMessage::RequestTopology => {
                WireMessage::Topology(analytics.export_topology_snapshot(Instant::now()))
            }
            _ => panic!("expected topology request"),
        };

        let res_bytes = common::encode_message(&response).expect("response should encode");
        let decoded = common::decode_message(&res_bytes).expect("response should decode");
        match decoded {
            WireMessage::Topology(snapshot) => {
                assert!(snapshot.snapshot_seq >= 1);
                assert_eq!(snapshot.global_stats.total_packets, 1);
                assert!(snapshot.nodes.iter().any(|n| n.node_id == node_id));
                assert!(!snapshot.edges.is_empty());
                assert!(
                    snapshot
                        .edges
                        .iter()
                        .any(|e| e.src_node_id == node_id && e.class == TrafficClass::Api)
                );
            }
            _ => panic!("expected topology response"),
        }
    }
}
