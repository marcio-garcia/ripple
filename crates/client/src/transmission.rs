use common::{
    NodeId, EndpointDomain, TrafficClass, WireMessage, analytics::AnalyticsSnapshot,
    make_data_packet,
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

pub struct ClientState {
    pub node_id: NodeId,
    pub desc: [u8; 16],
    pub src_domain: EndpointDomain,
    pub dst_domain: EndpointDomain,
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
            src_domain: EndpointDomain::External,
            dst_domain: EndpointDomain::Internal,
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
        }
    }
}

fn encode_wire_message(message: &WireMessage) -> Result<Vec<u8>> {
    common::encode_message(message).map_err(Error::other)
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

        let class_seq = state.next_class_seq.get(&front.class).unwrap_or(&0);

        let pkt = make_data_packet(
            state.node_id,
            state.next_global_seq,
            *class_seq,
            front.class,
            front.declared_bytes,
            state.src_domain,
            state.dst_domain,
            state.desc,
        );
        let bytes = encode_wire_message(&WireMessage::Data(pkt))?;
        let send_time = Instant::now();
        socket.send_to(&bytes, server_addr)?;

        state.pending_acks.insert(state.next_global_seq, send_time);
        state.next_global_seq = state.next_global_seq.wrapping_add(1);
        let next_class_seq = class_seq.wrapping_add(1);
        state.next_class_seq.insert(front.class, next_class_seq);
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
                        WireMessage::Data(_) | WireMessage::RequestAnalytics => {}
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
    if let Some(s) = &mut state.continuous_state {
        let now = Instant::now();

        while now >= s.next_send_at {
            let class_seq = state.next_class_seq.get(&s.class).unwrap_or(&0);

            let pkt = make_data_packet(
                state.node_id,
                state.next_global_seq,
                *class_seq,
                s.class,
                1200,
                state.src_domain,
                state.dst_domain,
                state.desc,
            );
            let bytes = encode_wire_message(&WireMessage::Data(pkt))?;
            let send_time = Instant::now();
            socket.send_to(&bytes, server_addr)?;

            state.pending_acks.insert(state.next_global_seq, send_time);
            state.next_global_seq = state.next_global_seq.wrapping_add(1);
            let next_class_seq = class_seq.wrapping_add(1);
            state.next_class_seq.insert(s.class, next_class_seq);
            s.next_send_at += s.interval;
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
