use crate::cli::parse_server_addr_args;
use crate::input::{execute_command, handle_input};
use crate::transmission::{
    ClientState, receive_acks, send_continuous_packets, send_scheduled_packets,
};
use crossterm::{
    ExecutableCommand, cursor,
    cursor::{MoveToColumn, MoveToNextLine},
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::env;
use std::io::{Result, stdout};
use std::net::UdpSocket;
use std::time::{Duration, Instant};

mod cli;
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
    let args: Vec<String> = env::args().collect();
    println!("Program path: {}", args[0]);
    let server_addr = parse_server_addr_args()?;

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
    print!("Stats: [waiting for ACKs...]"); // ← Add this line

    let socket = open_socket().expect("Couldn't open socket");
    socket.set_nonblocking(true).expect("error on non blocking");
    let result = run_app(socket, &server_addr);

    result
}

fn run_app(socket: UdpSocket, server_addr: &str) -> Result<()> {
    let mut state = ClientState::new();

    loop {
        // poll input only until the next send deadline instead of a fixed 50ms block.
        let timeout = compute_input_timeout(&state, Instant::now());
        if let Ok(Some(key)) = get_input(timeout) {
            match key {
                KeyCode::Char('q') | KeyCode::Esc => break,
                _ => {
                    if let Some(command) = handle_input(key) {
                        execute_command(command, &mut state, &socket, server_addr)?;
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
        // no fixed sleep; dynamic input polling now paces the loop.
    }

    Ok(())
}

fn open_socket() -> Result<UdpSocket> {
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    Ok(socket)
}

// cap idle polling so we stay responsive even when nothing is scheduled.
const MAX_INPUT_POLL_TIMEOUT: Duration = Duration::from_millis(50);

// derive the next wake-up from burst and continuous schedules.
fn compute_input_timeout(state: &ClientState, now: Instant) -> Duration {
    let next_burst_deadline = state.queue.front().map(|scheduled| scheduled.at);
    let next_continuous_deadline = state
        .continuous_state
        .as_ref()
        .map(|continuous| continuous.next_send_at);

    let next_deadline = [next_burst_deadline, next_continuous_deadline]
        .into_iter()
        .flatten()
        .min();

    match next_deadline {
        Some(deadline) => deadline
            .saturating_duration_since(now)
            .min(MAX_INPUT_POLL_TIMEOUT),
        None => MAX_INPUT_POLL_TIMEOUT,
    }
}

fn get_input(timeout: Duration) -> Result<Option<KeyCode>> {
    if event::poll(timeout)? {
        if let Event::Key(key) = event::read()? {
            if key.kind == KeyEventKind::Press {
                return Ok(Some(key.code));
            }
        }
    }
    Ok(None)
}
