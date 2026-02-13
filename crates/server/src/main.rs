use std::{io::Result, net::UdpSocket};
use common::{MAGIC, parse_packet};

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
