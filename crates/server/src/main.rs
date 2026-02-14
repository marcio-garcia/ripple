use std::{env, io::Result, net::UdpSocket, time::Instant};
use std::io::Error;
use common::WireMessage;
use crate::analytics::AnalyticsManager;

pub mod analytics;

fn encode_wire_message(message: &WireMessage) -> Result<Vec<u8>> {
    common::encode_message(message).map_err(Error::other)
}

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

        if let Ok(message) = common::decode_message(&buf[..amt]) {
            match message {
                WireMessage::Data(packet) => {
                    let ack = analytics.on_packet_received(src, &packet, Instant::now());
                    let ack_bytes = encode_wire_message(&WireMessage::Ack(ack))?;
                    socket.send_to(&ack_bytes, src)?;
                    println!(
                        "seq={} class={} class_seq={} → ACK sent",
                        packet.global_seq, packet.class, packet.class_seq
                    );
                }
                WireMessage::RequestAnalytics => {
                    let snapshot = analytics.export_snapshot();
                    let analytics_bytes =
                        encode_wire_message(&WireMessage::Analytics(snapshot))?;
                    socket.send_to(&analytics_bytes, src)?;
                    println!("Analytics snapshot sent to {} ({} bytes)", src, analytics_bytes.len());
                }
                WireMessage::Ack(_) | WireMessage::Analytics(_) => {
                    println!("Ignoring unexpected server-side message from {}", src);
                }
            }
        } else {
            println!("Failed to decode packet from {}", src);
        }

        packet_count += 1;
        if packet_count % 1000 == 0 {
            use std::time::Duration;
            analytics.cleanup_stale_clients(Duration::from_secs(60));
            println!("Cleaned up stale clients");
        }
    }
}
