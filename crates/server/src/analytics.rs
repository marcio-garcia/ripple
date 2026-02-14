use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::time::{Duration, Instant, SystemTime};
use common::{AckPacket, DataPacket};

/// Main analytics engine tracking all clients and stats
pub struct AnalyticsManager {
    /// When the server started (for relative timestamps)
    start_time: Instant,

    /// All connected clients, keyed by address
    clients: HashMap<SocketAddr, ClientState>,

    /// Global counters
    total_packets: u64,
    total_bytes: u64,
    packets_by_class: [u64; 4],
    bytes_by_class: [u64; 4],

    /// Configuration
    rate_window_secs: u32,  // 5 seconds
    #[allow(dead_code)]
    max_clients: usize,     // Prevent unbounded growth (e.g., 1000) - reserved for future use
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
        now: Instant
    ) -> AckPacket {
        // Get or create client state
        let client = self.clients
            .entry(src)
            .or_insert_with(|| ClientState::new(src, now, self.rate_window_secs));

        // Update client timestamp
        client.last_seen = now;

        // Update global counters
        let class_idx = packet.class as usize;
        self.total_packets += 1;
        self.total_bytes += packet.declared_bytes as u64;
        self.packets_by_class[class_idx] += 1;
        self.bytes_by_class[class_idx] += packet.declared_bytes as u64;

        // Update per client counters
        client.packets_by_class[class_idx] += 1;
        client.bytes_by_class[class_idx] += packet.declared_bytes as u64;

        // Process sequence - detect packet loss
        let loss_event = client.seq_trackers[class_idx]
            .process_sequence(packet.class_seq, now);

        // Log loss events
        match loss_event {
            LossEvent::Loss { count } => {
                println!("Loss detected: {} packets missing", count);
            }
            _ => {}
        }

        // 6. Record packet in rate calculator
        client.rate_calculators[class_idx]
            .record_packet(now, packet.declared_bytes);

        // 7. Calculate one-way latency (client timestamp → server receive time)
        // Use absolute wall-clock time (UNIX epoch) for accurate latency
        let server_timestamp_us = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("System time before UNIX epoch")
            .as_micros() as u64;

        let client_timestamp_us = packet.timestamp_us;

        // One-way latency (client and server use same time base: UNIX epoch)
        if server_timestamp_us >= client_timestamp_us {
            let latency_us = server_timestamp_us - client_timestamp_us;
            client.latency_stats.add_rtt_sample(latency_us);
        }

        // 8. Create and return ACK payload
        AckPacket {
            original_seq: packet.global_seq,
            server_timestamp_us,
            server_processing_us: 0,  // Could measure actual processing time if needed
        }
    }

    pub fn cleanup_stale_clients(&mut self, timeout: Duration) {
        let now = Instant::now();

        // Remove clients where last_seen is older than timeout
        self.clients.retain(|_addr, client| {
            now.duration_since(client.last_seen) < timeout
        });
    }

    pub fn export_snapshot(&self) -> common::analytics::AnalyticsSnapshot {
        let now = Instant::now();
        let uptime = now.duration_since(self.start_time).as_micros() as u64;

        // Build global stats
        let global_stats = common::analytics::GlobalStats {
            total_packets: self.total_packets,
            total_bytes: self.total_bytes,
            packets_by_class: self.packets_by_class,
            bytes_by_class: self.bytes_by_class,
            unique_clients: self.clients.len(),
        };

        // Build per-client stats
        let per_client_stats: Vec<_> = self.clients.iter().map(|(addr, client)| {
            // For each client, build ClassStats array
            let class_stats: [common::analytics::ClassStats; 4] =
                std::array::from_fn(|i| {
                    let (pps, bps) = client.rate_calculators[i].calculate_rate(now);
                    common::analytics::ClassStats {
                        packets: client.packets_by_class[i],
                        bytes: client.bytes_by_class[i],
                        packets_per_second: pps,
                        bytes_per_second: bps,
                    }
                });

            // Build latency metrics
            let latency = common::analytics::LatencyMetrics {
                min_rtt_us: client.latency_stats.min_rtt_us,
                max_rtt_us: client.latency_stats.max_rtt_us,
                mean_rtt_us: client.latency_stats.mean_rtt_us(),
                mean_jitter_us: client.latency_stats.mean_jitter_us(),
                samples: client.latency_stats.count,
            };

            // Build loss metrics (aggregate across all classes)
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

            // Build ClientStats
            common::analytics::ClientStats {
                addr: addr.to_string(),
                first_seen_us: client.first_seen
                    .duration_since(self.start_time)
                    .as_micros() as u64,
                last_seen_us: client.last_seen
                    .duration_since(self.start_time)
                    .as_micros() as u64,
                session_duration_us: client.last_seen
                    .duration_since(client.first_seen)
                    .as_micros() as u64,
                class_stats,
                latency,
                loss,
            }
        }).collect();

        // Build final snapshot
        common::analytics::AnalyticsSnapshot {
            snapshot_timestamp_us: uptime,
            server_uptime_us: uptime,
            global_stats,
            per_client_stats,
        }
    }

}

/// State for a single client
pub struct ClientState {
    /// Client's address (reserved for future debugging)
    #[allow(dead_code)]
    addr: SocketAddr,
    /// When we first saw this client
    first_seen: Instant,
    /// When we last saw this client (for timeout detection)
    last_seen: Instant,
    /// Sequence tracking per traffic class
    seq_trackers: [SequenceTracker; 4],
    /// Packet counters per class
    packets_by_class: [u64; 4],
    bytes_by_class: [u64; 4],
    /// RTT and jitter tracking
    latency_stats: LatencyStats,
    /// Rate calculation per class
    rate_calculators: [RateCalculator; 4],
}

impl ClientState {
    pub fn new(addr: SocketAddr, now: Instant, window_secs: u32) -> Self {
        ClientState {
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
                // First packet ever from this client/server
                self.last_seq = Some(seq);
                LossEvent::None
            }
            Some(last) => {
                let expected = last.wrapping_add(1);

                if seq == expected {
                    self.last_seq = Some(seq);
                    LossEvent::None
                } else if seq > expected {
                    // Missing packets
                    let missing = MissingSeqRange {
                        start: expected,
                        end: seq - 1,
                        detected_at: now,
                    };
                    let count = (seq - expected) as u64;
                    self.missing_sequences.push(missing);
                    self.last_seq = Some(seq);
                    LossEvent::Loss { count }
                } else {
                    // either duplicate or out of order
                    if seq == last {
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
}


#[derive(Debug, Clone)]
pub struct MissingSeqRange {
    /// First missing sequence number
    pub start: u32,

    /// Last missing sequence number (inclusive)
    pub end: u32,

    /// When we detected this gap
    pub detected_at: Instant,
}

/// Packet loss event types
pub enum LossEvent {
    None,
    Loss { count: u64 },
    OutOfOrder,
    Duplicate,
}

#[derive(Clone, Default)]
/// Latency statistics tracker
pub struct LatencyStats {
    /// Recent RTT samples (ring buffer, max 100)
    recent_rtts: VecDeque<u64>,
    max_samples: usize,  // 100

    /// Min RTT ever seen
    min_rtt_us: u64, // Use u64::MAX as 'no sample yet' - check if it's still MAX

    /// Max RTT ever seen
    max_rtt_us: u64,

    /// Sum of all RTTs (for mean calculation)
    sum_rtt_us: u64,

    /// Number of RTT samples
    count: u64,

    /// Sum of jitter values
    sum_jitter_us: u64,

    /// Number of jitter samples
    jitter_count: u64,
}

impl LatencyStats {
    pub fn new() -> Self {
        Self {
            recent_rtts: VecDeque::new(),
            max_samples: 100,
            min_rtt_us: u64::MAX,  // Sentinel value
            max_rtt_us: 0,
            sum_rtt_us: 0,
            count: 0,
            sum_jitter_us: 0,
            jitter_count: 0,
        }
    }

    pub fn add_rtt_sample(&mut self, rtt_us: u64) {
        // Calculate jitter (if we have a previous RTT)
        if let Some(prev_rtt) = self.recent_rtts.back() {
            let jitter = rtt_us.abs_diff(*prev_rtt);
            self.sum_jitter_us += jitter;
            self.jitter_count += 1;
        }

        // Add to ring buffer (max 100)
        self.recent_rtts.push_back(rtt_us);
        if self.recent_rtts.len() > self.max_samples {
            self.recent_rtts.pop_front();
        }

        // Update min/max/sum/count
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
/// Calculates packets/second over a sliding window
pub struct RateCalculator {
    /// Size of the sliding window
    window_duration: Duration,  // 5 seconds
    /// 1-second buckets
    buckets: VecDeque<RateBucket>,
}

impl RateCalculator {
    pub fn new(window_secs: u32) -> Self {
        RateCalculator {
            window_duration: Duration::from_secs(window_secs as u64),
            buckets: VecDeque::new()
        }
    }

    pub fn record_packet(&mut self, now: Instant, bytes: u32) {
        // Clean up old buckets
        while let Some(front) = self.buckets.front() {
            if now - front.timestamp >= self.window_duration {
                self.buckets.pop_front();  // Remove buckets older than 5s
            } else {
                break;
            }
        }

        // Get or create bucket for current second
        if let Some(last) = self.buckets.back_mut() {
            if now.duration_since(last.timestamp) < Duration::from_secs(1) {
                // Still in the same 1-second window - add to existing bucket
                last.packets += 1;
                last.bytes += bytes as u64;
                return;
            }
        }

        // New second - create new bucket
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
    /// When this bucket started
    timestamp: Instant,

    /// Packets in this 1-second bucket
    packets: u64,

    /// Bytes in this 1-second bucket
    bytes: u64,
}
