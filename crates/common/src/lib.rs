use std::{fmt::Display, time::Instant};

pub const MAGIC: u32 = 0x4A4E4554; // 'JNET'
const VERSION: u8 = 1;
const TYPE_DATA: u8 = 1;

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
    pub version: u8,
    pub msg_type: u8,
    pub class: TrafficClass,
    pub flags: u8,
    pub seq: u32,
    pub timestamp_us: u64,
    pub declared_bytes: u32,
}

pub fn pack_data_packet(
    seq: u32,
    class: TrafficClass,
    client_start: Instant,
    declared_bytes: u32
) -> [u8; 24] {
    let ts_us: u64 = client_start.elapsed().as_micros() as u64;
    let mut buf = [0u8; 24];
    buf[0..4].copy_from_slice(&MAGIC.to_le_bytes());
    buf[4] = VERSION;
    buf[5] = TYPE_DATA;
    buf[6] = class as u8;
    buf[7] = 0;
    buf[8..12].copy_from_slice(&seq.to_le_bytes());
    buf[12..20].copy_from_slice(&ts_us.to_le_bytes());
    buf[20..24].copy_from_slice(&declared_bytes.to_le_bytes());
    buf
}

pub fn parse_packet(buf: &[u8]) -> Option<Packet> {
    if buf.len() < 24 { return None; }

    let class_u8 = buf[6];
    let class = match class_u8 {
        0 => TrafficClass::Api,
        1 => TrafficClass::HeavyCompute,
        2 => TrafficClass::Background,
        3 => TrafficClass::HealthCheck,
        _ => return None,
    };

    Some(Packet {
        magic: u32::from_le_bytes(buf[0..4].try_into().ok()?),
        version: buf[4],
        msg_type: buf[5],
        class,
        flags: buf[7],
        seq: u32::from_le_bytes(buf[8..12].try_into().ok()?),
        timestamp_us: u64::from_le_bytes(buf[12..20].try_into().ok()?),
        declared_bytes: u32::from_le_bytes(buf[20..24].try_into().ok()?),
    })
}
