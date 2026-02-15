use std::{fmt::Display, path::Path, time::SystemTime};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub mod analytics;

pub type NodeId = [u8; 16];
pub type EdgeId = [u8; 16];

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum TrafficClass {
    Api = 0,
    HeavyCompute = 1,
    Background = 2,
    HealthCheck = 3,
}

impl Display for TrafficClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use TrafficClass::*;
        match self {
            Api => write!(f, "api"),
            HeavyCompute => write!(f, "heavy compute"),
            Background => write!(f, "background"),
            HealthCheck => write!(f, "health check"),
        }
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum EndpointDomain {
    Internal = 0,
    External = 1,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum NodeDomain {
    Internal = 0,
    External = 1,
}

impl From<EndpointDomain> for NodeDomain {
    fn from(value: EndpointDomain) -> Self {
        match value {
            EndpointDomain::Internal => NodeDomain::Internal,
            EndpointDomain::External => NodeDomain::External,
        }
    }
}

impl From<NodeDomain> for EndpointDomain {
    fn from(value: NodeDomain) -> Self {
        match value {
            NodeDomain::Internal => EndpointDomain::Internal,
            NodeDomain::External => EndpointDomain::External,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct RegisterNodePacket {
    pub node_id: NodeId,
    pub desc: [u8; 16],
    pub domain: NodeDomain,
    pub timestamp_us: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct UnregisterNodePacket {
    pub node_id: NodeId,
    pub timestamp_us: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct DataPacket {
    /// Source endpoint for the edge. Preferred for topology mode.
    pub src_node_id: NodeId,
    /// Destination endpoint for the edge. Preferred for topology mode.
    pub dst_node_id: NodeId,
    /// Legacy sender node id kept for compatibility with existing client controls.
    pub node_id: NodeId,
    pub global_seq: u32,
    pub class_seq: u32,
    pub class: TrafficClass,
    pub timestamp_us: u64,
    pub declared_bytes: u32,
    /// Legacy route information kept for compatibility.
    pub src_domain: EndpointDomain,
    /// Legacy route information kept for compatibility.
    pub dst_domain: EndpointDomain,
    pub desc: [u8; 16],
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct AckPacket {
    pub original_seq: u32,
    pub server_timestamp_us: u64,
    pub server_processing_us: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WireMessage {
    RegisterNode(RegisterNodePacket),
    UnregisterNode(UnregisterNodePacket),
    Data(DataPacket),
    Ack(AckPacket),
    RequestTopology,
    Topology(analytics::TopologySnapshot),
    RequestAnalytics,
    Analytics(analytics::AnalyticsSnapshot),
}

pub fn now_timestamp_us() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("System time before UNIX epoch")
        .as_micros() as u64
}

pub fn make_data_packet(
    node_id: NodeId,
    global_seq: u32,
    class_seq: u32,
    class: TrafficClass,
    declared_bytes: u32,
    src_domain: EndpointDomain,
    dst_domain: EndpointDomain,
    desc: [u8; 16],
) -> DataPacket {
    make_data_packet_with_endpoints(
        node_id,
        synthetic_domain_node_id(dst_domain),
        node_id,
        global_seq,
        class_seq,
        class,
        declared_bytes,
        src_domain,
        dst_domain,
        desc,
    )
}

pub fn make_data_packet_with_endpoints(
    src_node_id: NodeId,
    dst_node_id: NodeId,
    node_id: NodeId,
    global_seq: u32,
    class_seq: u32,
    class: TrafficClass,
    declared_bytes: u32,
    src_domain: EndpointDomain,
    dst_domain: EndpointDomain,
    desc: [u8; 16],
) -> DataPacket {
    DataPacket {
        src_node_id,
        dst_node_id,
        node_id,
        global_seq,
        class_seq,
        class,
        timestamp_us: now_timestamp_us(),
        declared_bytes,
        src_domain,
        dst_domain,
        desc,
    }
}

pub fn make_register_node_packet(
    node_id: NodeId,
    desc: [u8; 16],
    domain: NodeDomain,
) -> RegisterNodePacket {
    RegisterNodePacket {
        node_id,
        desc,
        domain,
        timestamp_us: now_timestamp_us(),
    }
}

pub fn make_unregister_node_packet(node_id: NodeId) -> UnregisterNodePacket {
    UnregisterNodePacket {
        node_id,
        timestamp_us: now_timestamp_us(),
    }
}

pub fn synthetic_domain_node_id(domain: EndpointDomain) -> NodeId {
    match domain {
        EndpointDomain::Internal => *b"__internal-node_",
        EndpointDomain::External => *b"__external-node_",
    }
}

pub fn encode_message(message: &WireMessage) -> postcard::Result<Vec<u8>> {
    postcard::to_stdvec(message)
}

pub fn decode_message(bytes: &[u8]) -> postcard::Result<WireMessage> {
    postcard::from_bytes(bytes)
}

pub fn load_or_create_id(path: &Path) -> std::io::Result<NodeId> {
    if path.exists() {
        let existing = std::fs::read_to_string(path)?;
        if let Ok(parsed) = Uuid::parse_str(existing.trim()) {
            return Ok(*parsed.as_bytes());
        }
    }

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let id = Uuid::new_v4();
    std::fs::write(path, format!("{id}\n"))?;
    Ok(*id.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_data_message() {
        let node_id: NodeId = *b"ABCDEFGHIJLMNOPQ";
        let desc: [u8; 16] = *b"test-node-------";
        let msg = WireMessage::Data(make_data_packet(
            node_id,
            10,
            5,
            TrafficClass::Background,
            1200,
            EndpointDomain::External,
            EndpointDomain::Internal,
            desc,
        ));
        let bytes = encode_message(&msg).expect("should encode");
        let decoded = decode_message(&bytes).expect("should decode");
        match decoded {
            WireMessage::Data(packet) => {
                assert_eq!(packet.src_node_id, node_id);
                assert_eq!(packet.dst_node_id, synthetic_domain_node_id(EndpointDomain::Internal));
                assert_eq!(packet.node_id, node_id);
                assert_eq!(packet.global_seq, 10);
                assert_eq!(packet.class_seq, 5);
                assert_eq!(packet.class, TrafficClass::Background);
                assert_eq!(packet.declared_bytes, 1200);
                assert_eq!(packet.src_domain, EndpointDomain::External);
                assert_eq!(packet.dst_domain, EndpointDomain::Internal);
                assert_eq!(packet.desc, desc);
            }
            _ => panic!("expected data message"),
        }
    }

    #[test]
    fn round_trip_register_node_message() {
        let node_id: NodeId = *b"ABCDEFGHIJLMNOPQ";
        let desc: [u8; 16] = *b"test-node-------";
        let msg = WireMessage::RegisterNode(make_register_node_packet(
            node_id,
            desc,
            NodeDomain::Internal,
        ));
        let bytes = encode_message(&msg).expect("should encode");
        let decoded = decode_message(&bytes).expect("should decode");
        match decoded {
            WireMessage::RegisterNode(packet) => {
                assert_eq!(packet.node_id, node_id);
                assert_eq!(packet.desc, desc);
                assert_eq!(packet.domain, NodeDomain::Internal);
            }
            _ => panic!("expected register-node message"),
        }
    }

    #[test]
    fn round_trip_topology_message() {
        let node_id: NodeId = *b"ABCDEFGHIJLMNOPQ";
        let edge_id: EdgeId = *b"QRSTUVWXYZABCDEF";
        let snapshot = analytics::TopologySnapshot {
            snapshot_seq: 1,
            snapshot_timestamp_epoch_us: 123_456,
            snapshot_interval_us: 10_000,
            nodes: vec![analytics::NodeSnapshot {
                node_id,
                desc: *b"test-node-------",
                domain: NodeDomain::Internal,
                first_seen_us: 10,
                last_seen_us: 20,
                active: true,
                total_packets: 1,
                total_bytes: 1200,
                total_pps: 0.2,
                total_bps: 240.0,
                latency: analytics::LatencyMetrics {
                    min_rtt_us: 100,
                    max_rtt_us: 100,
                    mean_rtt_us: 100.0,
                    mean_jitter_us: 0.0,
                    samples: 1,
                },
                loss: analytics::LossMetrics {
                    missing_sequences: 0,
                    out_of_order: 0,
                    duplicates: 0,
                    total_gaps: 0,
                },
            }],
            edges: vec![analytics::EdgeSnapshot {
                edge_id,
                src_node_id: node_id,
                dst_node_id: synthetic_domain_node_id(EndpointDomain::Internal),
                class: TrafficClass::Api,
                packets: 1,
                bytes: 1200,
                packets_per_second: 0.2,
                bytes_per_second: 240.0,
                delta_packets_per_second: 0.2,
                delta_bytes_per_second: 240.0,
                latency_ewma_us: 100.0,
                latency_delta_us: 0.0,
                jitter_ewma_us: 0.0,
                loss_rate_window: 0.0,
                active: true,
            }],
            removed_nodes: Vec::new(),
            removed_edges: Vec::new(),
            global_stats: analytics::GlobalStats {
                total_packets: 1,
                total_bytes: 1200,
                packets_by_class: [1, 0, 0, 0],
                bytes_by_class: [1200, 0, 0, 0],
                route_stats: [
                    analytics::RouteStats { packets: 0, bytes: 0 },
                    analytics::RouteStats { packets: 0, bytes: 0 },
                    analytics::RouteStats {
                        packets: 1,
                        bytes: 1200,
                    },
                    analytics::RouteStats { packets: 0, bytes: 0 },
                ],
                unique_clients: 1,
            },
        };

        let msg = WireMessage::Topology(snapshot);
        let bytes = encode_message(&msg).expect("should encode");
        let decoded = decode_message(&bytes).expect("should decode");
        match decoded {
            WireMessage::Topology(decoded_snapshot) => {
                assert_eq!(decoded_snapshot.snapshot_seq, 1);
                assert_eq!(decoded_snapshot.nodes.len(), 1);
                assert_eq!(decoded_snapshot.edges.len(), 1);
                assert_eq!(decoded_snapshot.edges[0].edge_id, edge_id);
            }
            _ => panic!("expected topology message"),
        }
    }
}
