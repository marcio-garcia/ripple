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
use crate::transmission::{ClientState, receive_acks, send_scheduled_packets};

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
    terminal::enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen, cursor::Hide)?;

    let _guard = TerminalGuard;

    println!("Network Traffic simulator");
    stdout.execute(MoveToNextLine(1))?;

    let socket = open_socket().expect("Couldn't open socket");
    socket.set_nonblocking(true).expect("error on non blocking");
    let server_addr = "127.0.0.1:8080";
    let result = run_app(socket, server_addr);


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
                                execute_command(command, &mut state);
                            }
                        }
                    }
                }
            }
        }
        stdout().execute(MoveToColumn(0))?;

        // Send scheduled packets
        send_scheduled_packets(&mut state, &socket, server_addr, Instant::now())?;

        // Receive ACKs
        receive_acks(&mut state, &socket)?;

        std::thread::sleep(Duration::from_millis(1));
    }

    Ok(())
}

fn open_socket() -> Result<UdpSocket>{
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    Ok(socket)
}
