use std::{
    collections::{HashMap, VecDeque},
    io::{Result, stdout},
    net::UdpSocket,
    time::{Duration, Instant}
};
use common::{
    TYPE_DATA,
    TrafficClass,
    analytics::AnalyticsSnapshot,
    pack_data_packet
};
use crossterm::{ExecutableCommand, cursor, terminal};

#[derive(Clone, Copy)]
pub struct ScheduledSend {
    pub at: Instant,
    pub class: TrafficClass,
    pub declared_bytes: u32,
}

pub enum SendMode {
    Idle,
    Continuous {
        class: TrafficClass,
        #[allow(dead_code)]
        packets_per_second: u32,  // Reserved for displaying current rate
        last_send: Instant,
        interval: Duration,
    },
}

pub struct ClientState {
    pub burst_count: u32,
    pub client_start: Instant,
    pub seq: u32,
    pub queue: VecDeque<ScheduledSend>,
    pub pending_acks: HashMap<u32, Instant>,
    pub total_acks: u64,
    pub min_rtt: Duration,
    pub max_rtt: Duration,
    pub sum_rtt: Duration,
    pub send_mode: SendMode,
}

impl ClientState {
    pub fn new() -> Self {
        Self {
            burst_count: 200,
            client_start: Instant::now(),
            seq: 0,
            queue: VecDeque::new(),
            pending_acks: HashMap::new(),
            total_acks: 0,
            min_rtt: Duration::MAX,
            max_rtt: Duration::ZERO,
            sum_rtt: Duration::ZERO,
            send_mode: SendMode::Idle,
        }
    }
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

        let pkt = pack_data_packet(
            state.seq,
            TYPE_DATA,
            front.class,
            state.client_start,
            front.declared_bytes
        );
        let send_time = Instant::now();
        socket.send_to(&pkt, server_addr)?;

        state.pending_acks.insert(state.seq, send_time);
        state.seq = state.seq.wrapping_add(1);
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

pub fn receive_acks(
    state: &mut ClientState,
    socket: &UdpSocket,
) -> Result<()> {
    let mut buf = [0u8; 8192];  // Large enough for both ACKs and analytics

    loop {
        match socket.recv_from(&mut buf) {
            Ok((amt, _src)) => {
                // Check packet size to distinguish ACKs from analytics
                if amt == 40 {
                    // ACK packet
                    if let Some(ack) = common::ack::parse_ack_packet(&buf[..amt]) {
                        if let Some(send_time) = state.pending_acks.remove(&ack.original_seq) {
                            let rtt = Instant::now() - send_time;

                            state.total_acks += 1;
                            state.min_rtt = state.min_rtt.min(rtt);
                            state.max_rtt = state.max_rtt.max(rtt);
                            state.sum_rtt += rtt;

                            // Display RTT on fixed line
                            let mut out = stdout();
                            out.execute(cursor::SavePosition)?;
                            out.execute(cursor::MoveTo(0, 2))?;  // Move to stats line
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
                } else if amt > 100 {
                    // Analytics packet (postcard serialized, typically hundreds/thousands of bytes)
                    if let Ok(snapshot) = postcard::from_bytes::<common::analytics::AnalyticsSnapshot>(&buf[..amt]) {
                        display_analytics(&snapshot);
                    }
                }
                // Ignore other packet sizes
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
    if let SendMode::Continuous { class, packets_per_second: _, last_send, interval } = &mut state.send_mode {
        let now = Instant::now();

        if now.duration_since(*last_send) >= *interval {
            // Send packet
            let pkt = pack_data_packet(
                state.seq,
                TYPE_DATA,
                *class,
                state.client_start,
                1200  // Default bytes
            );
            let send_time = Instant::now();
            socket.send_to(&pkt, server_addr)?;

            state.pending_acks.insert(state.seq, send_time);
            state.seq = state.seq.wrapping_add(1);

            *last_send = now;
        }
    }

    Ok(())
}

fn display_analytics(snapshot: &AnalyticsSnapshot) {
    use crossterm::{ExecutableCommand, cursor, terminal};
    use std::io::{stdout, Write};

    let mut out = stdout();
    out.execute(cursor::SavePosition).ok();
    out.execute(cursor::MoveTo(0, 20)).ok();
    out.execute(terminal::Clear(terminal::ClearType::FromCursorDown)).ok();

    // Build the entire output string first (use \r\n for raw mode)
    let mut output = String::new();
    output.push_str("=== Analytics Snapshot ===\r\n");
    output.push_str(&format!("Server uptime: {:.2}s\r\n", snapshot.server_uptime_us as f64 / 1_000_000.0));
    output.push_str(&format!("Total packets: {}\r\n", snapshot.global_stats.total_packets));
    output.push_str(&format!("Total bytes: {}\r\n", snapshot.global_stats.total_bytes));
    output.push_str(&format!("Unique clients: {}\r\n", snapshot.global_stats.unique_clients));

    output.push_str("\r\nPer-class breakdown:\r\n");
    let classes = ["Api", "HeavyCompute", "Background", "HealthCheck"];
    for (i, name) in classes.iter().enumerate() {
        let pkts = snapshot.global_stats.packets_by_class[i];
        let bytes = snapshot.global_stats.bytes_by_class[i];
        if pkts > 0 {
            output.push_str(&format!("  {}: {} packets, {} bytes\r\n", name, pkts, bytes));
        }
    }

    if let Some(client) = snapshot.per_client_stats.first() {
        // Only show latency if we have actual samples (min_rtt != u64::MAX sentinel)
        if client.latency.samples > 0 && client.latency.min_rtt_us != u64::MAX {
            output.push_str(&format!("\r\nLatency: min={}µs max={}µs avg={:.0}µs\r\n",
                client.latency.min_rtt_us,
                client.latency.max_rtt_us,
                client.latency.mean_rtt_us
            ));
        } else {
            output.push_str("\r\nLatency: (no RTT data collected by server)\r\n");
        }

        output.push_str(&format!("Loss: {} missing, {} out-of-order, {} duplicates\r\n",
            client.loss.missing_sequences,
            client.loss.out_of_order,
            client.loss.duplicates
        ));
    }

    output.push_str("========================\r\n");

    // Print all at once and flush
    print!("{}", output);
    out.flush().ok();
    out.execute(cursor::RestorePosition).ok();
}
