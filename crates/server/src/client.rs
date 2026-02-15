use std::{
    collections::VecDeque,
    net::SocketAddr,
    time::{Duration, Instant},
};

use common::NodeId;

/// State for a single client
pub struct Client {
    pub node_id: NodeId,
    pub desc: [u8; 16],
    pub addr: SocketAddr,
    pub first_seen: Instant,
    pub last_seen: Instant,
    pub seq_trackers: [SequenceTracker; 4],
    pub packets_by_class: [u64; 4],
    pub bytes_by_class: [u64; 4],
    pub route_packets: [u64; 4],
    pub route_bytes: [u64; 4],
    pub latency_stats: LatencyStats,
    pub rate_calculators: [RateCalculator; 4],
}

impl Client {
    pub fn new(
        node_id: NodeId,
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

#[derive(Clone, Default)]
/// Tracks sequence numbers for one traffic class
pub struct SequenceTracker {
    /// Last sequence number we saw
    last_seq: Option<u32>,

    /// Ranges of missing sequences
    pub missing_sequences: Vec<MissingSeqRange>,

    /// Count of out-of-order packets
    pub out_of_order_count: u64,

    /// Count of duplicate packets
    pub duplicate_count: u64,
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

#[derive(Clone, Default)]
pub struct LatencyStats {
    recent_rtts: VecDeque<u64>,
    max_samples: usize,
    pub min_rtt_us: u64,
    pub max_rtt_us: u64,
    sum_rtt_us: u64,
    pub count: u64,
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
