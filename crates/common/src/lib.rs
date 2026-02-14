use std::{fmt::Display, time::SystemTime};

use serde::{Deserialize, Serialize};

pub mod analytics;

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct DataPacket {
    pub global_seq: u32,
    pub class_seq: u32,
    pub class: TrafficClass,
    pub timestamp_us: u64,
    pub declared_bytes: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct AckPacket {
    pub original_seq: u32,
    pub server_timestamp_us: u64,
    pub server_processing_us: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WireMessage {
    Data(DataPacket),
    Ack(AckPacket),
    RequestAnalytics,
    Analytics(analytics::AnalyticsSnapshot),
}

pub fn now_timestamp_us() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("System time before UNIX epoch")
        .as_micros() as u64
}

pub fn make_data_packet(
    global_seq: u32,
    class_seq: u32,
    class: TrafficClass,
    declared_bytes: u32,
) -> DataPacket {
    DataPacket {
        global_seq,
        class_seq,
        class,
        timestamp_us: now_timestamp_us(),
        declared_bytes,
    }
}

pub fn encode_message(message: &WireMessage) -> postcard::Result<Vec<u8>> {
    postcard::to_stdvec(message)
}

pub fn decode_message(bytes: &[u8]) -> postcard::Result<WireMessage> {
    postcard::from_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_data_message() {
        let msg = WireMessage::Data(make_data_packet(10, 5, TrafficClass::Background, 1200));
        let bytes = encode_message(&msg).expect("should encode");
        let decoded = decode_message(&bytes).expect("should decode");
        match decoded {
            WireMessage::Data(packet) => {
                assert_eq!(packet.global_seq, 10);
                assert_eq!(packet.class_seq, 5);
                assert_eq!(packet.class, TrafficClass::Background);
                assert_eq!(packet.declared_bytes, 1200);
            }
            _ => panic!("expected data message"),
        }
    }
}
