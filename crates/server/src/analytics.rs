use common::{AckPacket, ClientId, DataPacket};
use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::time::{Duration, Instant, SystemTime};

/// Main analytics engine tracking all clients and stats
pub struct AnalyticsManager {
    /// When the server started (for relative timestamps)
    start_time: Instant,

    /// All connected clients, keyed by stable node id
    clients: HashMap<ClientId, Client>,

    /// Global counters
    total_packets: u64,
    total_bytes: u64,
    packets_by_class: [u64; 4],
    bytes_by_class: [u64; 4],

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
        self.total_packets += 1;
        self.total_bytes += packet.declared_bytes as u64;
        self.packets_by_class[class_idx] += 1;
        self.bytes_by_class[class_idx] += packet.declared_bytes as u64;

        client.packets_by_class[class_idx] += 1;
        client.bytes_by_class[class_idx] += packet.declared_bytes as u64;

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

/// State for a single client
pub struct Client {
    node_id: ClientId,
    desc: [u8; 16],
    addr: SocketAddr,
    first_seen: Instant,
    last_seen: Instant,
    seq_trackers: [SequenceTracker; 4],
    packets_by_class: [u64; 4],
    bytes_by_class: [u64; 4],
    latency_stats: LatencyStats,
    rate_calculators: [RateCalculator; 4],
}

impl Client {
    pub fn new(
        node_id: ClientId,
        desc: [u8; 16],
        addr: SocketAddr,
        now: Instant,
        window_secs: u32,
    ) -> Self {
        Client {
            node_id,
            desc,
            addr,
            first_seen: now,
            last_seen: now,
            seq_trackers: Default::default(),
            packets_by_class: [0; 4],
            bytes_by_class: [0; 4],
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

#[derive(Clone, Default)]
/// Tracks sequence numbers for one traffic class
pub struct SequenceTracker {
    /// Last sequence number we saw
    last_seq: Option<u32>,

    /// Ranges of missing sequences
    missing_sequences: Vec<MissingSeqRange>,

    /// Count of out-of-order packets
    out_of_order_count: u64,

    /// Count of duplicate packets
    duplicate_count: u64,
}

impl SequenceTracker {
    pub fn process_sequence(&mut self, seq: u32, now: Instant) -> LossEvent {
        match self.last_seq {
            None => {
                self.last_seq = Some(seq);
                LossEvent::None
            }
            Some(last) => {
                let expected = last.wrapping_add(1);

                if seq == expected {
                    self.last_seq = Some(seq);
                    LossEvent::None
                } else if seq > expected {
                    let missing = MissingSeqRange {
                        start: expected,
                        end: seq - 1,
                        detected_at: now,
                    };
                    let count = (seq - expected) as u64;
                    self.missing_sequences.push(missing);
                    self.last_seq = Some(seq);
                    LossEvent::Loss { count }
                } else if seq == last {
                    self.duplicate_count += 1;
                    LossEvent::Duplicate
                } else {
                    self.out_of_order_count += 1;
                    LossEvent::OutOfOrder
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct MissingSeqRange {
    pub start: u32,
    pub end: u32,
    pub detected_at: Instant,
}

pub enum LossEvent {
    None,
    Loss { count: u64 },
    OutOfOrder,
    Duplicate,
}

#[derive(Clone, Default)]
pub struct LatencyStats {
    recent_rtts: VecDeque<u64>,
    max_samples: usize,
    min_rtt_us: u64,
    max_rtt_us: u64,
    sum_rtt_us: u64,
    count: u64,
    sum_jitter_us: u64,
    jitter_count: u64,
}

impl LatencyStats {
    pub fn new() -> Self {
        Self {
            recent_rtts: VecDeque::new(),
            max_samples: 100,
            min_rtt_us: u64::MAX,
            max_rtt_us: 0,
            sum_rtt_us: 0,
            count: 0,
            sum_jitter_us: 0,
            jitter_count: 0,
        }
    }

    pub fn add_rtt_sample(&mut self, rtt_us: u64) {
        if let Some(prev_rtt) = self.recent_rtts.back() {
            let jitter = rtt_us.abs_diff(*prev_rtt);
            self.sum_jitter_us += jitter;
            self.jitter_count += 1;
        }

        self.recent_rtts.push_back(rtt_us);
        if self.recent_rtts.len() > self.max_samples {
            self.recent_rtts.pop_front();
        }

        self.min_rtt_us = self.min_rtt_us.min(rtt_us);
        self.max_rtt_us = self.max_rtt_us.max(rtt_us);
        self.sum_rtt_us += rtt_us;
        self.count += 1;
    }

    pub fn mean_rtt_us(&self) -> f64 {
        if self.count == 0 {
            0.0
        } else {
            self.sum_rtt_us as f64 / self.count as f64
        }
    }

    pub fn mean_jitter_us(&self) -> f64 {
        if self.jitter_count == 0 {
            0.0
        } else {
            self.sum_jitter_us as f64 / self.jitter_count as f64
        }
    }
}

#[derive(Clone, Default)]
pub struct RateCalculator {
    window_duration: Duration,
    buckets: VecDeque<RateBucket>,
}

impl RateCalculator {
    pub fn new(window_secs: u32) -> Self {
        RateCalculator {
            window_duration: Duration::from_secs(window_secs as u64),
            buckets: VecDeque::new(),
        }
    }

    pub fn record_packet(&mut self, now: Instant, bytes: u32) {
        while let Some(front) = self.buckets.front() {
            if now - front.timestamp >= self.window_duration {
                self.buckets.pop_front();
            } else {
                break;
            }
        }

        if let Some(last) = self.buckets.back_mut() {
            if now.duration_since(last.timestamp) < Duration::from_secs(1) {
                last.packets += 1;
                last.bytes += bytes as u64;
                return;
            }
        }

        self.buckets.push_back(RateBucket {
            timestamp: now,
            packets: 1,
            bytes: bytes as u64,
        });
    }

    pub fn calculate_rate(&self, now: Instant) -> (f64, f64) {
        let mut packets = 0;
        let mut bytes = 0;
        for bucket in self.buckets.iter() {
            if now - bucket.timestamp >= self.window_duration {
                continue;
            }
            packets += bucket.packets;
            bytes += bucket.bytes;
        }

        let window_secs = self.window_duration.as_secs_f64();
        (packets as f64 / window_secs, bytes as f64 / window_secs)
    }
}

#[derive(Debug, Clone)]
pub struct RateBucket {
    timestamp: Instant,
    packets: u64,
    bytes: u64,
}
