use common::{
    EndpointDomain, NodeDomain, NodeId, TrafficClass, WireMessage,
    analytics::{AnalyticsSnapshot, TopologySnapshot},
    make_data_packet_with_endpoints, make_register_node_packet, make_unregister_node_packet,
};
use crossterm::{ExecutableCommand, cursor, terminal};
use std::{
    collections::{HashMap, VecDeque},
    io::{Error, Result, stdout},
    net::UdpSocket,
    time::{Duration, Instant},
};

#[derive(Clone, Copy)]
pub struct ScheduledSend {
    pub at: Instant,
    pub class: TrafficClass,
    pub declared_bytes: u32,
}

pub struct ContinuousState {
    pub class: TrafficClass,
    pub next_send_at: Instant,
    pub interval: Duration,
}

#[derive(Clone, Copy)]
pub struct PeerNode {
    pub node_id: NodeId,
    pub domain: EndpointDomain,
    pub desc: [u8; 16],
}

pub struct ClientState {
    pub node_id: NodeId,
    pub desc: [u8; 16],
    pub node_domain: NodeDomain,
    pub src_domain: EndpointDomain,
    pub dst_domain: EndpointDomain,
    pub peers: Vec<PeerNode>,
    pub active_peer_index: usize,
    pub next_peer_counter: u64,
    pub burst_count: u32,
    pub next_global_seq: u32,
    pub next_class_seq: HashMap<TrafficClass, u32>,
    pub queue: VecDeque<ScheduledSend>,
    pub pending_acks: HashMap<u32, Instant>,
    pub total_acks: u64,
    pub min_rtt: Duration,
    pub max_rtt: Duration,
    pub sum_rtt: Duration,
    pub continuous_state: Option<ContinuousState>,
    pub pending_topology_expectation: Option<TopologyExpectation>,
}

impl ClientState {
    pub fn new(node_id: NodeId, desc: [u8; 16]) -> Self {
        let mut init_class_seq = HashMap::new();
        init_class_seq.insert(TrafficClass::Api, 0);
        init_class_seq.insert(TrafficClass::Background, 0);
        init_class_seq.insert(TrafficClass::HeavyCompute, 0);
        init_class_seq.insert(TrafficClass::HealthCheck, 0);
        Self {
            node_id,
            desc,
            node_domain: NodeDomain::External,
            src_domain: EndpointDomain::External,
            dst_domain: EndpointDomain::Internal,
            peers: vec![
                PeerNode {
                    node_id: *b"peer-internal---",
                    domain: EndpointDomain::Internal,
                    desc: *b"peer-int-default",
                },
                PeerNode {
                    node_id: *b"peer-external---",
                    domain: EndpointDomain::External,
                    desc: *b"peer-ext-default",
                },
            ],
            active_peer_index: 0,
            next_peer_counter: 1,
            burst_count: 200,
            next_global_seq: 0,
            next_class_seq: init_class_seq,
            queue: VecDeque::new(),
            pending_acks: HashMap::new(),
            total_acks: 0,
            min_rtt: Duration::MAX,
            max_rtt: Duration::ZERO,
            sum_rtt: Duration::ZERO,
            continuous_state: None,
            pending_topology_expectation: None,
        }
    }
}

#[derive(Clone, Copy)]
pub enum TopologyExpectation {
    Smoke { node_id: NodeId },
    Removal { node_id: NodeId },
    MixedClasses { node_id: NodeId },
}

fn encode_wire_message(message: &WireMessage) -> Result<Vec<u8>> {
    common::encode_message(message).map_err(Error::other)
}

fn active_peer(state: &ClientState) -> Option<PeerNode> {
    state.peers.get(state.active_peer_index).copied()
}

fn make_peer_desc(domain: EndpointDomain, counter: u64) -> [u8; 16] {
    let mut desc = [0u8; 16];
    let domain_tag = match domain {
        EndpointDomain::Internal => "int",
        EndpointDomain::External => "ext",
    };
    let text = format!("peer-{domain_tag}-{counter:06}");
    let bytes = text.as_bytes();
    let len = bytes.len().min(desc.len());
    desc[..len].copy_from_slice(&bytes[..len]);
    desc
}

fn make_peer_id(state: &mut ClientState, domain: EndpointDomain) -> NodeId {
    let mut id = [0u8; 16];
    id[..4].copy_from_slice(b"peer");
    id[4] = match domain {
        EndpointDomain::Internal => b'i',
        EndpointDomain::External => b'e',
    };
    id[5..8].copy_from_slice(&state.node_id[..3]);
    id[8..].copy_from_slice(&state.next_peer_counter.to_be_bytes());
    state.next_peer_counter = state.next_peer_counter.saturating_add(1);
    id
}

fn add_peer_local(state: &mut ClientState, domain: EndpointDomain) -> PeerNode {
    let peer = PeerNode {
        node_id: make_peer_id(state, domain),
        domain,
        desc: make_peer_desc(domain, state.next_peer_counter.saturating_sub(1)),
    };
    state.peers.push(peer);
    state.active_peer_index = state.peers.len().saturating_sub(1);
    state.dst_domain = domain;
    peer
}

fn select_first_peer_for_domain(state: &mut ClientState, domain: EndpointDomain) -> Option<PeerNode> {
    let idx = state.peers.iter().position(|peer| peer.domain == domain)?;
    state.active_peer_index = idx;
    state.dst_domain = domain;
    state.peers.get(idx).copied()
}

fn destination_peer(state: &mut ClientState, requested_domain: EndpointDomain) -> PeerNode {
    if let Some(peer) = active_peer(state) {
        if peer.domain == requested_domain {
            return peer;
        }
    }

    if let Some(peer) = select_first_peer_for_domain(state, requested_domain) {
        return peer;
    }

    add_peer_local(state, requested_domain)
}

fn short_node_id(node_id: &NodeId) -> String {
    format!(
        "{:02x}{:02x}{:02x}{:02x}",
        node_id[0], node_id[1], node_id[2], node_id[3]
    )
}

fn render_peer_status(state: &ClientState) -> Result<()> {
    let mut out = stdout();
    out.execute(cursor::SavePosition)?;
    out.execute(cursor::MoveTo(0, 6))?;
    out.execute(terminal::Clear(terminal::ClearType::CurrentLine))?;
    if let Some(peer) = active_peer(state) {
        print!(
            "Peer: active={}/{} id={} domain={}",
            state.active_peer_index + 1,
            state.peers.len(),
            short_node_id(&peer.node_id),
            format_domain(peer.domain)
        );
    } else {
        print!("Peer: active=none");
    }
    out.execute(cursor::RestorePosition)?;
    Ok(())
}

fn format_domain(domain: EndpointDomain) -> &'static str {
    match domain {
        EndpointDomain::Internal => "internal",
        EndpointDomain::External => "external",
    }
}

fn send_data_packet(
    state: &mut ClientState,
    socket: &UdpSocket,
    server_addr: &str,
    class: TrafficClass,
    declared_bytes: u32,
    src_domain: EndpointDomain,
    dst_domain: EndpointDomain,
) -> Result<()> {
    let class_seq = state.next_class_seq.get(&class).copied().unwrap_or(0);
    let dst_peer = destination_peer(state, dst_domain);
    let pkt = make_data_packet_with_endpoints(
        state.node_id,
        dst_peer.node_id,
        state.node_id,
        state.next_global_seq,
        class_seq,
        class,
        declared_bytes,
        src_domain,
        dst_peer.domain,
        state.desc,
    );
    let bytes = encode_wire_message(&WireMessage::Data(pkt))?;
    let send_time = Instant::now();
    socket.send_to(&bytes, server_addr)?;

    state.pending_acks.insert(state.next_global_seq, send_time);
    state.next_global_seq = state.next_global_seq.wrapping_add(1);
    state.next_class_seq.insert(class, class_seq.wrapping_add(1));
    Ok(())
}

fn send_register_node(
    socket: &UdpSocket,
    server_addr: &str,
    node_id: NodeId,
    desc: [u8; 16],
    domain: NodeDomain,
) -> Result<()> {
    let pkt = make_register_node_packet(node_id, desc, domain);
    let bytes = encode_wire_message(&WireMessage::RegisterNode(pkt))?;
    socket.send_to(&bytes, server_addr)?;
    Ok(())
}

fn send_register_self(
    state: &ClientState,
    socket: &UdpSocket,
    server_addr: &str,
) -> Result<()> {
    send_register_node(
        socket,
        server_addr,
        state.node_id,
        state.desc,
        state.node_domain,
    )
}

fn send_unregister_node(
    socket: &UdpSocket,
    server_addr: &str,
    node_id: NodeId,
) -> Result<()> {
    let pkt = make_unregister_node_packet(node_id);
    let bytes = encode_wire_message(&WireMessage::UnregisterNode(pkt))?;
    socket.send_to(&bytes, server_addr)?;
    Ok(())
}

pub fn request_topology(socket: &UdpSocket, server_addr: &str) -> Result<()> {
    let pkt = encode_wire_message(&WireMessage::RequestTopology)?;
    socket.send_to(&pkt, server_addr)?;
    Ok(())
}

pub fn register_self(state: &ClientState, socket: &UdpSocket, server_addr: &str) -> Result<()> {
    send_register_self(state, socket, server_addr)
}

pub fn unregister_self(state: &ClientState, socket: &UdpSocket, server_addr: &str) -> Result<()> {
    send_unregister_node(socket, server_addr, state.node_id)
}

pub fn update_source_domain(
    state: &mut ClientState,
    domain: EndpointDomain,
    socket: &UdpSocket,
    server_addr: &str,
) -> Result<()> {
    state.src_domain = domain;
    state.node_domain = NodeDomain::from(domain);
    register_self(state, socket, server_addr)
}

pub fn add_peer(
    state: &mut ClientState,
    domain: EndpointDomain,
    socket: &UdpSocket,
    server_addr: &str,
) -> Result<()> {
    let peer = add_peer_local(state, domain);
    send_register_node(
        socket,
        server_addr,
        peer.node_id,
        peer.desc,
        NodeDomain::from(peer.domain),
    )?;
    render_peer_status(state)
}

pub fn cycle_active_peer(state: &mut ClientState) -> Result<()> {
    if state.peers.is_empty() {
        render_peer_status(state)?;
        return Ok(());
    }
    state.active_peer_index = (state.active_peer_index + 1) % state.peers.len();
    if let Some(peer) = active_peer(state) {
        state.dst_domain = peer.domain;
    }
    render_peer_status(state)
}

pub fn remove_active_peer(
    state: &mut ClientState,
    socket: &UdpSocket,
    server_addr: &str,
) -> Result<()> {
    if state.peers.is_empty() {
        render_peer_status(state)?;
        return Ok(());
    }

    let removed = state.peers.remove(state.active_peer_index);
    send_unregister_node(socket, server_addr, removed.node_id)?;

    if state.peers.is_empty() {
        let replacement = add_peer_local(state, EndpointDomain::Internal);
        send_register_node(
            socket,
            server_addr,
            replacement.node_id,
            replacement.desc,
            NodeDomain::from(replacement.domain),
        )?;
    } else {
        if state.active_peer_index >= state.peers.len() {
            state.active_peer_index = state.peers.len() - 1;
        }
        if let Some(peer) = active_peer(state) {
            state.dst_domain = peer.domain;
        }
    }

    render_peer_status(state)
}

pub fn select_or_add_peer_for_domain(
    state: &mut ClientState,
    domain: EndpointDomain,
    socket: &UdpSocket,
    server_addr: &str,
) -> Result<()> {
    if select_first_peer_for_domain(state, domain).is_none() {
        let peer = add_peer_local(state, domain);
        send_register_node(
            socket,
            server_addr,
            peer.node_id,
            peer.desc,
            NodeDomain::from(peer.domain),
        )?;
    }
    state.dst_domain = domain;
    render_peer_status(state)
}

pub fn run_topology_smoke_test(
    state: &mut ClientState,
    socket: &UdpSocket,
    server_addr: &str,
) -> Result<()> {
    state.queue.clear();
    state.continuous_state = None;
    state.node_domain = NodeDomain::Internal;
    send_register_self(state, socket, server_addr)?;
    select_or_add_peer_for_domain(state, EndpointDomain::External, socket, server_addr)?;
    send_data_packet(
        state,
        socket,
        server_addr,
        TrafficClass::Api,
        1200,
        EndpointDomain::Internal,
        EndpointDomain::External,
    )?;
    request_topology(socket, server_addr)?;
    state.pending_topology_expectation = Some(TopologyExpectation::Smoke {
        node_id: state.node_id,
    });
    render_topology_status("Topology smoke test dispatched (awaiting snapshot)")?;
    Ok(())
}

pub fn run_topology_removal_test(
    state: &mut ClientState,
    socket: &UdpSocket,
    server_addr: &str,
) -> Result<()> {
    state.queue.clear();
    state.continuous_state = None;
    state.node_domain = NodeDomain::Internal;
    send_register_self(state, socket, server_addr)?;
    send_unregister_node(socket, server_addr, state.node_id)?;
    request_topology(socket, server_addr)?;
    state.pending_topology_expectation = Some(TopologyExpectation::Removal {
        node_id: state.node_id,
    });
    render_topology_status("Topology removal test dispatched (awaiting snapshot)")?;
    Ok(())
}

pub fn run_topology_mixed_classes_test(
    state: &mut ClientState,
    socket: &UdpSocket,
    server_addr: &str,
) -> Result<()> {
    state.queue.clear();
    state.continuous_state = None;
    state.node_domain = NodeDomain::Internal;
    send_register_self(state, socket, server_addr)?;
    select_or_add_peer_for_domain(state, EndpointDomain::External, socket, server_addr)?;
    for class in [
        TrafficClass::Api,
        TrafficClass::HeavyCompute,
        TrafficClass::Background,
        TrafficClass::HealthCheck,
    ] {
        send_data_packet(
            state,
            socket,
            server_addr,
            class,
            1200,
            EndpointDomain::Internal,
            EndpointDomain::External,
        )?;
    }
    request_topology(socket, server_addr)?;
    state.pending_topology_expectation = Some(TopologyExpectation::MixedClasses {
        node_id: state.node_id,
    });
    render_topology_status("Topology mixed-classes test dispatched (awaiting snapshot)")?;
    Ok(())
}

pub fn send_scheduled_packets(
    state: &mut ClientState,
    socket: &UdpSocket,
    server_addr: &str,
    now: Instant,
) -> Result<()> {
    loop {
        let front = match state.queue.front() {
            Some(f) if f.at <= now => *f,
            _ => break,
        };

        state.queue.pop_front();

        send_data_packet(
            state,
            socket,
            server_addr,
            front.class,
            front.declared_bytes,
            state.src_domain,
            state.dst_domain,
        )?;
    }

    Ok(())
}

pub fn schedule_burst(
    q: &mut VecDeque<ScheduledSend>,
    now: Instant,
    count: u32,
    interval_ms: u32,
    class: TrafficClass,
    declared_bytes: u32,
) {
    let interval = Duration::from_millis(interval_ms as u64);
    for i in 0..count {
        q.push_back(ScheduledSend {
            at: now + interval * i,
            class,
            declared_bytes,
        });
    }
}

pub fn receive_acks(state: &mut ClientState, socket: &UdpSocket) -> Result<()> {
    let mut buf = [0u8; 8192];

    loop {
        match socket.recv_from(&mut buf) {
            Ok((amt, _src)) => {
                if let Ok(message) = common::decode_message(&buf[..amt]) {
                    match message {
                        WireMessage::Ack(ack) => {
                            if let Some(send_time) = state.pending_acks.remove(&ack.original_seq) {
                                let rtt = Instant::now() - send_time;

                                state.total_acks += 1;
                                state.min_rtt = state.min_rtt.min(rtt);
                                state.max_rtt = state.max_rtt.max(rtt);
                                state.sum_rtt += rtt;

                                let mut out = stdout();
                                out.execute(cursor::SavePosition)?;
                                out.execute(cursor::MoveTo(0, 4))?;
                                out.execute(terminal::Clear(terminal::ClearType::CurrentLine))?;
                                print!(
                                    "Stats: ACK seq={:5} | RTT={:4}µs | min={:4} max={:4} avg={:4}",
                                    ack.original_seq,
                                    rtt.as_micros(),
                                    state.min_rtt.as_micros(),
                                    state.max_rtt.as_micros(),
                                    (state.sum_rtt.as_micros() / state.total_acks as u128)
                                );
                                out.execute(cursor::RestorePosition)?;
                            }
                        }
                        WireMessage::Analytics(snapshot) => display_analytics(&snapshot),
                        WireMessage::Topology(snapshot) => {
                            display_topology_snapshot(state, &snapshot)?;
                        }
                        WireMessage::Data(_)
                        | WireMessage::RequestAnalytics
                        | WireMessage::RegisterNode(_)
                        | WireMessage::UnregisterNode(_)
                        | WireMessage::RequestTopology => {}
                    }
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(e) => {
                eprintln!("Error receiving: {}", e);
                break;
            }
        }
    }

    Ok(())
}

pub fn send_continuous_packets(
    state: &mut ClientState,
    socket: &UdpSocket,
    server_addr: &str,
) -> Result<()> {
    while let Some((class, next_send_at, interval)) = state
        .continuous_state
        .as_ref()
        .map(|s| (s.class, s.next_send_at, s.interval))
    {
        if Instant::now() < next_send_at {
            break;
        }

        send_data_packet(
            state,
            socket,
            server_addr,
            class,
            1200,
            state.src_domain,
            state.dst_domain,
        )?;

        if let Some(s) = state.continuous_state.as_mut() {
            s.next_send_at += interval;
        }
    }

    Ok(())
}

fn display_analytics(snapshot: &AnalyticsSnapshot) {
    use crossterm::{ExecutableCommand, cursor, terminal};
    use std::io::{Write, stdout};

    let mut out = stdout();
    out.execute(cursor::SavePosition).ok();
    out.execute(cursor::MoveTo(0, 20)).ok();
    out.execute(terminal::Clear(terminal::ClearType::FromCursorDown))
        .ok();

    let mut output = String::new();
    output.push_str("=== Analytics Snapshot ===\r\n");
    output.push_str(&format!(
        "Server uptime: {:.2}s\r\n",
        snapshot.server_uptime_us as f64 / 1_000_000.0
    ));
    output.push_str(&format!(
        "Total packets: {}\r\n",
        snapshot.global_stats.total_packets
    ));
    output.push_str(&format!(
        "Total bytes: {}\r\n",
        snapshot.global_stats.total_bytes
    ));
    output.push_str(&format!(
        "Unique clients: {}\r\n",
        snapshot.global_stats.unique_clients
    ));

    output.push_str("\r\nPer-class breakdown:\r\n");
    let classes = ["Api", "HeavyCompute", "Background", "HealthCheck"];
    for (i, name) in classes.iter().enumerate() {
        let pkts = snapshot.global_stats.packets_by_class[i];
        let bytes = snapshot.global_stats.bytes_by_class[i];
        if pkts > 0 {
            output.push_str(&format!(
                "  {}: {} packets, {} bytes\r\n",
                name, pkts, bytes
            ));
        }
    }

    if let Some(client) = snapshot.per_client_stats.first() {
        output.push_str(&format!(
            "\r\nClient: id={} desc={} addr={}\r\n",
            format_node_id_as_uuid(&client.node_id),
            format_desc(&client.desc),
            client.addr
        ));
        output.push_str("Routes:\r\n");
        for (i, route) in client.route_stats.iter().enumerate() {
            if route.packets > 0 {
                output.push_str(&format!(
                    "  {}: {} packets, {} bytes\r\n",
                    route_label(i),
                    route.packets,
                    route.bytes
                ));
            }
        }

        if client.latency.samples > 0 && client.latency.min_rtt_us != u64::MAX {
            output.push_str(&format!(
                "\r\nLatency: min={}µs max={}µs avg={:.0}µs\r\n",
                client.latency.min_rtt_us, client.latency.max_rtt_us, client.latency.mean_rtt_us
            ));
        } else {
            output.push_str("\r\nLatency: (no RTT data collected by server)\r\n");
        }

        output.push_str(&format!(
            "Loss: {} missing, {} out-of-order, {} duplicates\r\n",
            client.loss.missing_sequences, client.loss.out_of_order, client.loss.duplicates
        ));
    }

    output.push_str("========================\r\n");

    print!("{}", output);
    out.flush().ok();
    out.execute(cursor::RestorePosition).ok();
}

fn display_topology_snapshot(state: &mut ClientState, snapshot: &TopologySnapshot) -> Result<()> {
    let base = format!(
        "Topology: seq={} nodes={} edges={} packets={}",
        snapshot.snapshot_seq,
        snapshot.nodes.len(),
        snapshot.edges.len(),
        snapshot.global_stats.total_packets
    );

    let status = if let Some(expectation) = state.pending_topology_expectation.take() {
        validate_topology_expectation(expectation, snapshot)
    } else {
        format!("{base} (no active test)")
    };

    render_topology_status(&status)
}

fn validate_topology_expectation(
    expectation: TopologyExpectation,
    snapshot: &TopologySnapshot,
) -> String {
    match expectation {
        TopologyExpectation::Smoke { node_id } => {
            let node_present = snapshot.nodes.iter().any(|node| node.node_id == node_id);
            let edge_present = snapshot
                .edges
                .iter()
                .any(|edge| edge.src_node_id == node_id);
            let looks_like_external_target = snapshot
                .nodes
                .iter()
                .any(|node| node.domain == NodeDomain::External);
            let pass = node_present && edge_present && looks_like_external_target;
            format!(
                "Topology smoke [{}]: node={} edge={} external_node={} nodes={} edges={} packets={}",
                pass_label(pass),
                yes_no(node_present),
                yes_no(edge_present),
                yes_no(looks_like_external_target),
                snapshot.nodes.len(),
                snapshot.edges.len(),
                snapshot.global_stats.total_packets
            )
        }
        TopologyExpectation::Removal { node_id } => {
            let removed = snapshot.removed_nodes.contains(&node_id);
            format!(
                "Topology removal [{}]: removed_node={} removed_nodes={} removed_edges={}",
                pass_label(removed),
                yes_no(removed),
                snapshot.removed_nodes.len(),
                snapshot.removed_edges.len()
            )
        }
        TopologyExpectation::MixedClasses { node_id } => {
            let classes = [
                TrafficClass::Api,
                TrafficClass::HeavyCompute,
                TrafficClass::Background,
                TrafficClass::HealthCheck,
            ];
            let all_classes_present = classes.iter().all(|class| {
                snapshot
                    .edges
                    .iter()
                    .any(|edge| edge.src_node_id == node_id && edge.class == *class)
            });
            format!(
                "Topology mixed-classes [{}]: class_edges_found={} edges={}",
                pass_label(all_classes_present),
                yes_no(all_classes_present),
                snapshot.edges.len()
            )
        }
    }
}

fn render_topology_status(message: &str) -> Result<()> {
    let mut out = stdout();
    out.execute(cursor::SavePosition)?;
    out.execute(cursor::MoveTo(0, 5))?;
    out.execute(terminal::Clear(terminal::ClearType::CurrentLine))?;
    print!("{message}");
    out.execute(cursor::RestorePosition)?;
    Ok(())
}

fn pass_label(pass: bool) -> &'static str {
    if pass { "PASS" } else { "FAIL" }
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn format_node_id_as_uuid(node_id: &[u8; 16]) -> String {
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        node_id[0],
        node_id[1],
        node_id[2],
        node_id[3],
        node_id[4],
        node_id[5],
        node_id[6],
        node_id[7],
        node_id[8],
        node_id[9],
        node_id[10],
        node_id[11],
        node_id[12],
        node_id[13],
        node_id[14],
        node_id[15],
    )
}

fn format_desc(desc: &[u8; 16]) -> String {
    let rendered = String::from_utf8_lossy(desc);
    let trimmed = rendered.trim_end_matches('\0');
    if trimmed.is_empty() {
        "<empty>".to_string()
    } else {
        trimmed.to_string()
    }
}

fn route_label(index: usize) -> &'static str {
    match index {
        0 => "internal->internal",
        1 => "internal->external",
        2 => "external->internal",
        3 => "external->external",
        _ => "unknown",
    }
}
