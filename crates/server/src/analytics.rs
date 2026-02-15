use common::{AckPacket, DataPacket, EndpointDomain, NodeId};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant, SystemTime};

use crate::client::{Client, LossEvent};

/// Main analytics engine tracking all clients and stats
pub struct AnalyticsManager {
    /// When the server started (for relative timestamps)
    start_time: Instant,

    /// All connected clients, keyed by stable node id
    clients: HashMap<NodeId, Client>,

    /// Global counters
    total_packets: u64,
    total_bytes: u64,
    packets_by_class: [u64; 4],
    bytes_by_class: [u64; 4],
    route_packets: [u64; 4],
    route_bytes: [u64; 4],

    /// Configuration
    rate_window_secs: u32,
    #[allow(dead_code)]
    max_clients: usize,
}

impl AnalyticsManager {
    pub fn new(window_secs: u32, max_clients: usize) -> Self {
        AnalyticsManager {
            start_time: Instant::now(),
            clients: HashMap::new(),
            total_packets: 0,
            total_bytes: 0,
            packets_by_class: [0; 4],
            bytes_by_class: [0; 4],
            route_packets: [0; 4],
            route_bytes: [0; 4],
            rate_window_secs: window_secs,
            max_clients,
        }
    }

    pub fn on_packet_received(
        &mut self,
        src: SocketAddr,
        packet: &DataPacket,
        now: Instant,
    ) -> AckPacket {
        let client = self.clients.entry(packet.node_id).or_insert_with(|| {
            Client::new(packet.node_id, packet.desc, src, now, self.rate_window_secs)
        });

        client.last_seen = now;
        client.addr = src;
        client.desc = packet.desc;

        let class_idx = packet.class as usize;
        let route_idx = route_index(packet.src_domain, packet.dst_domain);
        self.total_packets += 1;
        self.total_bytes += packet.declared_bytes as u64;
        self.packets_by_class[class_idx] += 1;
        self.bytes_by_class[class_idx] += packet.declared_bytes as u64;
        self.route_packets[route_idx] += 1;
        self.route_bytes[route_idx] += packet.declared_bytes as u64;

        client.packets_by_class[class_idx] += 1;
        client.bytes_by_class[class_idx] += packet.declared_bytes as u64;
        client.route_packets[route_idx] += 1;
        client.route_bytes[route_idx] += packet.declared_bytes as u64;

        let loss_event = client.seq_trackers[class_idx].process_sequence(packet.class_seq, now);
        if let LossEvent::Loss { count } = loss_event {
            println!("Loss detected: {} packets missing", count);
        }

        client.rate_calculators[class_idx].record_packet(now, packet.declared_bytes);

        let server_timestamp_us = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("System time before UNIX epoch")
            .as_micros() as u64;

        let client_timestamp_us = packet.timestamp_us;
        if server_timestamp_us >= client_timestamp_us {
            let latency_us = server_timestamp_us - client_timestamp_us;
            client.latency_stats.add_rtt_sample(latency_us);
        }

        AckPacket {
            original_seq: packet.global_seq,
            server_timestamp_us,
            server_processing_us: 0,
        }
    }

    pub fn cleanup_stale_clients(&mut self, timeout: Duration) {
        let now = Instant::now();
        self.clients
            .retain(|_node_id, client| now.duration_since(client.last_seen) < timeout);
    }

    pub fn export_snapshot(&self) -> common::analytics::AnalyticsSnapshot {
        let now = Instant::now();
        let uptime = now.duration_since(self.start_time).as_micros() as u64;

        let global_stats = common::analytics::GlobalStats {
            total_packets: self.total_packets,
            total_bytes: self.total_bytes,
            packets_by_class: self.packets_by_class,
            bytes_by_class: self.bytes_by_class,
            route_stats: std::array::from_fn(|i| common::analytics::RouteStats {
                packets: self.route_packets[i],
                bytes: self.route_bytes[i],
            }),
            unique_clients: self.clients.len(),
        };

        let per_client_stats: Vec<_> = self
            .clients
            .values()
            .map(|client| {
                let class_stats: [common::analytics::ClassStats; 4] = std::array::from_fn(|i| {
                    let (pps, bps) = client.rate_calculators[i].calculate_rate(now);
                    common::analytics::ClassStats {
                        packets: client.packets_by_class[i],
                        bytes: client.bytes_by_class[i],
                        packets_per_second: pps,
                        bytes_per_second: bps,
                    }
                });

                let latency = common::analytics::LatencyMetrics {
                    min_rtt_us: client.latency_stats.min_rtt_us,
                    max_rtt_us: client.latency_stats.max_rtt_us,
                    mean_rtt_us: client.latency_stats.mean_rtt_us(),
                    mean_jitter_us: client.latency_stats.mean_jitter_us(),
                    samples: client.latency_stats.count,
                };

                let mut missing_seqs = 0u64;
                let mut out_of_order = 0u64;
                let mut duplicates = 0u64;
                let mut total_gaps = 0usize;

                for tracker in &client.seq_trackers {
                    for gap in &tracker.missing_sequences {
                        missing_seqs += (gap.end - gap.start + 1) as u64;
                    }
                    total_gaps += tracker.missing_sequences.len();
                    out_of_order += tracker.out_of_order_count;
                    duplicates += tracker.duplicate_count;
                }

                let loss = common::analytics::LossMetrics {
                    missing_sequences: missing_seqs,
                    out_of_order,
                    duplicates,
                    total_gaps,
                };

                common::analytics::ClientStats {
                    node_id: client.node_id,
                    desc: client.desc,
                    addr: client.addr.to_string(),
                    first_seen_us: client
                        .first_seen
                        .duration_since(self.start_time)
                        .as_micros() as u64,
                    last_seen_us: client.last_seen.duration_since(self.start_time).as_micros()
                        as u64,
                    session_duration_us: client
                        .last_seen
                        .duration_since(client.first_seen)
                        .as_micros() as u64,
                    class_stats,
                    latency,
                    loss,
                    route_stats: std::array::from_fn(|i| common::analytics::RouteStats {
                        packets: client.route_packets[i],
                        bytes: client.route_bytes[i],
                    }),
                }
            })
            .collect();

        common::analytics::AnalyticsSnapshot {
            snapshot_timestamp_us: uptime,
            server_uptime_us: uptime,
            global_stats,
            per_client_stats,
        }
    }
}

fn route_index(src_domain: EndpointDomain, dst_domain: EndpointDomain) -> usize {
    match (src_domain, dst_domain) {
        (EndpointDomain::Internal, EndpointDomain::Internal) => 0,
        (EndpointDomain::Internal, EndpointDomain::External) => 1,
        (EndpointDomain::External, EndpointDomain::Internal) => 2,
        (EndpointDomain::External, EndpointDomain::External) => 3,
    }
}
