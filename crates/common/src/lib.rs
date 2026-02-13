use std::{fmt::Display, time::Instant};

pub mod ack;
pub mod analytics;

pub const MAGIC: u32 = 0x4A4E4554; // 'JNET'
pub const TYPE_DATA: u8 = 1;
pub const TYPE_ACK: u8 = 2; // server acknowledgments
const VERSION: u8 = 1;
const TYPE_ANALYTICS: u8 = 3; // analytics snapshots
const TYPE_REQUEST_ANALYTICS: u8 = 4; // client requests analytics

#[repr(u8)]
#[derive(Clone, Copy, Debug)]
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

#[derive(Debug)]
pub struct Packet {
    pub magic: u32,
    pub declared_bytes: u32,
    pub version: u8,
    pub msg_type: u8,
    pub class: TrafficClass,
    pub flags: u8,
    pub seq: u32,
    pub timestamp_us: u64,
}

fn pack_header(
    timestamp: u64, // as micros
    declared_bytes: u32,
    data_type: u8,
    class: u8,
    seq: u32,
) -> [u8; 24] {
    let mut buf = [0u8; 24];
    buf[0..4].copy_from_slice(&MAGIC.to_le_bytes());
    buf[4..8].copy_from_slice(&declared_bytes.to_le_bytes());
    buf[8] = VERSION;
    buf[9] = data_type;
    buf[10] = class;
    buf[11] = 0;
    buf[12..16].copy_from_slice(&seq.to_le_bytes());
    buf[16..24].copy_from_slice(&timestamp.to_le_bytes());
    buf
}

pub fn parse_header(buf: &[u8]) -> Option<Packet> {
    if buf.len() < 24 { return None; }

    let class_u8 = buf[10];
    let class = match class_u8 {
        0 => TrafficClass::Api,
        1 => TrafficClass::HeavyCompute,
        2 => TrafficClass::Background,
        3 => TrafficClass::HealthCheck,
        _ => return None,
    };

    Some(Packet {
        magic: u32::from_le_bytes(buf[0..4].try_into().ok()?),
        declared_bytes: u32::from_le_bytes(buf[4..8].try_into().ok()?),
        version: buf[8],
        msg_type: buf[9],
        class,
        flags: buf[11],
        seq: u32::from_le_bytes(buf[12..16].try_into().ok()?),
        timestamp_us: u64::from_le_bytes(buf[16..24].try_into().ok()?),
    })
}

pub fn pack_data_packet(
    seq: u32,
    data_type: u8,
    class: TrafficClass,
    client_start: Instant,
    declared_bytes: u32,
) -> [u8; 24] {
    let ts_us: u64 = client_start.elapsed().as_micros() as u64;
    pack_header(ts_us, declared_bytes, data_type, class as u8, seq)
}

pub fn parse_packet(buf: &[u8]) -> Option<Packet> {
    if buf.len() < 24 { return None; }
    parse_header(buf)
}
