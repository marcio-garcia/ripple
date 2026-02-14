use std::{env, io::Result, net::UdpSocket, time::Instant};
use common::{MAGIC, TYPE_DATA, TYPE_REQUEST_ANALYTICS, parse_packet};
use crate::analytics::AnalyticsManager;

pub mod analytics;

fn main() -> Result<()>{
    let mut server_addr = String::from("127.0.0.1");
    let mut port = String::from("8080");
    let args: Vec<String> = env::args().collect();
    println!("Program path: {}", args[0]);

    for (idx, arg) in args.iter().enumerate() {
        if idx >= args.len() { continue; }
        match arg.as_str() {
            "-s" => { server_addr = args[idx + 1].clone(); }
            "-p" => { port = args[idx + 1].clone(); }
            _ => {}
        }
    }

    server_addr = format!("{}:{}", server_addr, port);

    let socket = UdpSocket::bind(server_addr).expect("Couldn't bind to socket");
    println!("Server listening on {}...", port);

    let mut analytics = AnalyticsManager::new(5, 1000);  // 5-sec window, max 1000 clients
    let mut buf = [0u8; 1024];
    let mut packet_count = 0;

    loop {
        let (amt, src) = socket.recv_from(&mut buf)?;

        println!("Received {} bytes from {}", amt, src);

        if let Some(packet) = parse_packet(&buf[..amt]) {
            if packet.magic != MAGIC {
                println!("Invalid packet: {:08x}", packet.magic);
                continue;
            }

            match packet.msg_type {
                TYPE_DATA => {
                    // Existing analytics processing
                    let ack_payload = analytics.on_packet_received(src, &packet, Instant::now());
                    let ack_packet = common::ack::pack_ack_packet(
                        ack_payload.original_seq,
                        ack_payload.server_timestamp_us,
                        ack_payload.server_processing_us,
                    );
                    socket.send_to(&ack_packet, src)?;
                    println!("seq={} class={} → ACK sent", packet.seq, packet.class);
                }

                TYPE_REQUEST_ANALYTICS => {
                    // Export and send analytics
                    let snapshot = analytics.export_snapshot();
                    let analytics_bytes = postcard::to_stdvec(&snapshot)
                        .expect("Failed to serialize analytics");

                    // Send analytics packet (header + payload)
                    socket.send_to(&analytics_bytes, src)?;
                    println!("Analytics snapshot sent to {} ({} bytes)", src, analytics_bytes.len());
                }

                _ => {
                    println!("Unknown packet type: {}", packet.msg_type);
                }
            }
        }

        packet_count += 1;
        if packet_count % 1000 == 0 {
            use std::time::Duration;
            analytics.cleanup_stale_clients(Duration::from_secs(60));
            println!("Cleaned up stale clients");
        }
    }
}
