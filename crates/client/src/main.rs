use std::env;
use std::io::{Result, stdout};
use std::net::UdpSocket;
use std::time::{Duration, Instant};
use crossterm::{
    ExecutableCommand,
    cursor,
    cursor::{MoveToColumn, MoveToNextLine},
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use crate::input::{execute_command, handle_input};
use crate::transmission::{ClientState, receive_acks, send_continuous_packets, send_scheduled_packets};

mod input;
mod transmission;

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let _ = execute!(stdout(), LeaveAlternateScreen, cursor::Show);
    }
}

fn main() -> Result<()> {
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

    terminal::enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen, cursor::Hide)?;

    let _guard = TerminalGuard;

    print!("Network Traffic simulator");
    stdout.execute(MoveToNextLine(1))?;
    print!("Send to: {}", &server_addr);
    stdout.execute(MoveToNextLine(1))?;
    print!("Commands: Space=send | B=burst | 1-9=count | Q=quit");
    stdout.execute(MoveToNextLine(1))?;
    print!("Stats: [waiting for ACKs...]");  // ← Add this line

    let socket = open_socket().expect("Couldn't open socket");
    socket.set_nonblocking(true).expect("error on non blocking");
    let result = run_app(socket, &server_addr);


    result
}

fn run_app(socket: UdpSocket, server_addr: &str) -> Result<()> {
    let mut state = ClientState::new();

    loop {
        // Handle keyboard input
        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => break,
                        _ => {
                            if let Some(command) = handle_input(key.code) {
                                execute_command(
                                    command,
                                    &mut state,
                                    &socket,
                                    server_addr
                                )?;
                            }
                        }
                    }
                }
            }
        }
        stdout().execute(MoveToColumn(0))?;

        // Send scheduled packets
        send_scheduled_packets(&mut state, &socket, server_addr, Instant::now())?;

        // Send continuous packets (if in continuous mode)
        send_continuous_packets(&mut state, &socket, server_addr)?;

        // Receive ACKs and analytics
        receive_acks(&mut state, &socket)?;

        std::thread::sleep(Duration::from_millis(1));
    }

    Ok(())
}

fn open_socket() -> Result<UdpSocket>{
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    Ok(socket)
}
