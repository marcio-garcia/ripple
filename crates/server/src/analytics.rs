use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use common::{Packet, TrafficClass};

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
    max_clients: usize,     // Prevent unbounded growth (e.g., 1000)
}

impl AnalyticsManager {
    pub fn new(window_secs: u32, max_clients: usize) -> Self {
        todo!("Initialize manager")
    }

    pub fn on_packet_received(
        &mut self,
        src: SocketAddr,
        packet: &Packet,
        now: Instant
    ) -> common::ack::AckPayload {
        todo!("Process packet, update stats, return ACK")
    }

    pub fn export_snapshot(&self) -> common::analytics::AnalyticsSnapshot {
        todo!("Create analytics snapshot")
    }

    pub fn cleanup_stale_clients(&mut self, timeout: Duration) {
        todo!("Remove inactive clients")
    }
}

/// State for a single client
pub struct ClientState {
    /// Client's address
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
        todo!("Initialize all fields")
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
        todo!("Detect gaps, out-of-order, duplicates")
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
        todo!("Initialize with min_rtt_us = u64::MAX")
    }

    pub fn add_rtt_sample(&mut self, rtt_us: u64) {
        todo!("Add RTT, calculate jitter")
    }

    pub fn mean_rtt_us(&self) -> f64 {
        todo!("Return average RTT")
    }

    pub fn mean_jitter_us(&self) -> f64 {
        todo!("Return average jitter")
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
