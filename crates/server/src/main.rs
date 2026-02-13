use std::{io::Result, net::UdpSocket};

const MAGIC: u32 = 0x4A4E4554;

fn main() -> Result<()>{
    let socket = UdpSocket::bind("127.0.0.1:8080").expect("Couldn't bind to socket");
    println!("Server listening on 8080...");

    let mut buf = [0u8; 1024];

    loop {
        let (amt, src) = socket.recv_from(&mut buf)?;

        println!("Received {} bytes from {}", amt, src);

        if let Some(packet) = parse_packet(&buf[..amt]) {
            if packet.magic != MAGIC {
                println!("Invalid packet: {:08x}", packet.magic);
                continue;
            }

            println!(
                "seq={} class={} ts={} from {}",
                packet.seq, packet.class, packet.timestamp_us, src
            );
        }


    }
}

struct ReceivedPacket {
    magic: u32,
    version: u8,
    msg_type: u8,
    class: u8,
    flags: u8,
    seq: u32,
    timestamp_us: u64,
    declared_bytes: u32,
}

fn parse_packet(buf: &[u8]) -> Option<ReceivedPacket> {
    if buf.len() < 24 {
        return None;  // Too small
    }

    Some(ReceivedPacket {
        magic: u32::from_le_bytes(buf[0..4].try_into().ok()?),
        version: buf[4],
        msg_type: buf[5],
        class: buf[6],
        flags: buf[7],
        seq: u32::from_le_bytes(buf[8..12].try_into().ok()?),
        timestamp_us: u64::from_le_bytes(buf[12..20].try_into().ok()?),
        declared_bytes: u32::from_le_bytes(buf[20..24].try_into().ok()?),
    })
}
