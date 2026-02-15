use common::{NodeDomain, NodeId, TrafficClass, WireMessage};
use server::analytics::AnalyticsManager;
use std::net::SocketAddr;
use std::str::FromStr;
use std::time::{Duration, Instant};

fn test_addr() -> SocketAddr {
    SocketAddr::from_str("127.0.0.1:59090").expect("valid socket")
}

fn handle_message(
    analytics: &mut AnalyticsManager,
    message: WireMessage,
    src: SocketAddr,
    now: Instant,
) -> Option<WireMessage> {
    match message {
        WireMessage::RegisterNode(packet) => {
            analytics.on_node_registered(&packet, src, now);
            None
        }
        WireMessage::UnregisterNode(packet) => {
            analytics.on_node_unregistered(&packet, now);
            None
        }
        WireMessage::Data(packet) => {
            let ack = analytics.on_packet_received(src, &packet, now);
            Some(WireMessage::Ack(ack))
        }
        WireMessage::RequestTopology => {
            let topology = analytics.export_topology_snapshot(now);
            Some(WireMessage::Topology(topology))
        }
        WireMessage::RequestAnalytics => {
            let snapshot = analytics.export_snapshot();
            Some(WireMessage::Analytics(snapshot))
        }
        WireMessage::Ack(_) | WireMessage::Topology(_) | WireMessage::Analytics(_) => None,
    }
}

fn dispatch(
    analytics: &mut AnalyticsManager,
    outbound: WireMessage,
    src: SocketAddr,
    now: Instant,
) -> Option<WireMessage> {
    let encoded = common::encode_message(&outbound).expect("message should encode");
    let decoded = common::decode_message(&encoded).expect("message should decode");
    let response = handle_message(analytics, decoded, src, now)?;
    let response_bytes = common::encode_message(&response).expect("response should encode");
    Some(common::decode_message(&response_bytes).expect("response should decode"))
}

#[test]
fn register_send_remove_request_topology_flow() {
    let mut analytics = AnalyticsManager::new(5, 100);
    let base = Instant::now();
    let src = test_addr();

    let src_node_id: NodeId = *b"FLOW-SRC-NODE001";
    let dst_node_id: NodeId = *b"FLOW-DST-NODE002";
    let src_desc = *b"flow-src--------";
    let dst_desc = *b"flow-dst--------";

    dispatch(
        &mut analytics,
        WireMessage::RegisterNode(common::make_register_node_packet(
            src_node_id,
            src_desc,
            NodeDomain::Internal,
        )),
        src,
        base,
    );
    dispatch(
        &mut analytics,
        WireMessage::RegisterNode(common::make_register_node_packet(
            dst_node_id,
            dst_desc,
            NodeDomain::External,
        )),
        src,
        base + Duration::from_millis(1),
    );

    let packet = common::make_data_packet(
        src_node_id,
        dst_node_id,
        1,
        1,
        TrafficClass::Api,
        1200,
        src_desc,
    );
    let ack = dispatch(
        &mut analytics,
        WireMessage::Data(packet),
        src,
        base + Duration::from_millis(10),
    )
    .expect("ack expected");
    match ack {
        WireMessage::Ack(packet) => assert_eq!(packet.original_seq, 1),
        _ => panic!("expected ack"),
    }

    let snapshot_before_remove = dispatch(
        &mut analytics,
        WireMessage::RequestTopology,
        src,
        base + Duration::from_millis(20),
    )
    .expect("topology expected");
    let topology_before_remove = match snapshot_before_remove {
        WireMessage::Topology(snapshot) => snapshot,
        _ => panic!("expected topology snapshot"),
    };
    assert!(
        topology_before_remove
            .nodes
            .iter()
            .any(|node| node.node_id == src_node_id)
    );
    assert!(
        topology_before_remove
            .nodes
            .iter()
            .any(|node| node.node_id == dst_node_id)
    );
    assert!(topology_before_remove.edges.iter().any(|edge| {
        edge.src_node_id == src_node_id
            && edge.dst_node_id == dst_node_id
            && edge.class == TrafficClass::Api
    }));

    dispatch(
        &mut analytics,
        WireMessage::UnregisterNode(common::make_unregister_node_packet(src_node_id)),
        src,
        base + Duration::from_millis(30),
    );

    let snapshot_after_remove = dispatch(
        &mut analytics,
        WireMessage::RequestTopology,
        src,
        base + Duration::from_millis(40),
    )
    .expect("topology expected");
    let topology_after_remove = match snapshot_after_remove {
        WireMessage::Topology(snapshot) => snapshot,
        _ => panic!("expected topology snapshot"),
    };
    assert!(topology_after_remove.removed_nodes.contains(&src_node_id));
    assert!(topology_after_remove.edges.is_empty());
    assert!(!topology_after_remove.removed_edges.is_empty());
}
