use crate::{TYPE_ACK, pack_header, parse_header};

/// ACK payload sent back to client
#[derive(Debug, Clone, Copy)]
pub struct AckPayload {
    /// Sequence number we're acknowledging
    pub original_seq: u32,
    /// When server received the packet (microseconds since server start)
    pub server_timestamp_us: u64,
    /// How long server took to process (typically microseconds)
    pub server_processing_us: u64,
}

pub fn parse_ack_packet(buf: &[u8]) -> Option<AckPayload> {
    if let Some(header) = parse_header(buf) {
        if header.msg_type != TYPE_ACK {
            return None;
        }

        let original_seq = u32::from_le_bytes(buf[24..28].try_into().ok()?);
        let server_timestamp_us = u64::from_le_bytes(buf[28..36].try_into().ok()?);
        let server_processing_us = u64::from_le_bytes(buf[36..40].try_into().ok()?);

        return Some(AckPayload {
            original_seq,
            server_timestamp_us,
            server_processing_us
        });
    }
    None
}

pub fn pack_ack_packet(original_seq: u32, server_timestamp_us: u64, server_processing_us: u64) -> [u8; 40] {
    let mut buf = [0u8; 40];
    let header = pack_header(
        server_timestamp_us,
        40,
        TYPE_ACK,
        0, // Not really used for ACKs
        0 // ACKs don't need their own sequence
    );
    buf[0..24].copy_from_slice(&header);
    buf[24..28].copy_from_slice(&original_seq.to_le_bytes());
    buf[28..36].copy_from_slice(&server_timestamp_us.to_le_bytes());
    buf[36..40].copy_from_slice(&server_processing_us.to_le_bytes());
    buf
}
