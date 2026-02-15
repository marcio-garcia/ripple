use crate::cli::parse_server_addr_args;
use crate::input::{execute_command, handle_input};
use crate::transmission::{
    ClientState, receive_acks, send_continuous_packets, send_scheduled_packets,
};
use common::EndpointDomain;
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
use std::path::Path;
use std::time::{Duration, Instant};
use uuid::Uuid;

mod cli;
mod input;
mod transmission;

const DEFAULT_DESC: [u8; 16] = *b"simd-client-----";
const MAX_INPUT_POLL_TIMEOUT: Duration = Duration::from_millis(50);

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
    print!("Commands: Space=send | B=burst | 1-9=count | I/E=src | K/L=dst | Q=quit");
    stdout.execute(MoveToNextLine(1))?;
    print!("Mode: src=external dst=internal");
    stdout.execute(MoveToNextLine(1))?;
    print!("Stats: [waiting for ACKs...]");

    let socket = open_socket().expect("Couldn't open socket");
    socket.set_nonblocking(true).expect("error on non blocking");
    let result = run_app(socket, &server_addr);

    result
}

fn run_app(socket: UdpSocket, server_addr: &str) -> Result<()> {
    let node_id = load_or_create_client_id(Path::new("client_id.txt"))?;
    let mut state = ClientState::new(node_id, DEFAULT_DESC);

    loop {
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

        render_domain_status(&state)?;
        stdout().execute(MoveToColumn(0))?;

        send_scheduled_packets(&mut state, &socket, server_addr, Instant::now())?;
        send_continuous_packets(&mut state, &socket, server_addr)?;
        receive_acks(&mut state, &socket)?;
    }

    Ok(())
}

fn open_socket() -> Result<UdpSocket> {
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    Ok(socket)
}

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

fn render_domain_status(state: &ClientState) -> Result<()> {
    let mut out = stdout();
    out.execute(cursor::SavePosition)?;
    out.execute(cursor::MoveTo(0, 3))?;
    out.execute(terminal::Clear(terminal::ClearType::CurrentLine))?;
    print!(
        "Mode: src={} dst={}",
        format_domain(state.src_domain),
        format_domain(state.dst_domain)
    );
    out.execute(cursor::RestorePosition)?;
    Ok(())
}

fn format_domain(domain: EndpointDomain) -> &'static str {
    match domain {
        EndpointDomain::Internal => "internal",
        EndpointDomain::External => "external",
    }
}

fn load_or_create_client_id(path: &Path) -> std::io::Result<common::ClientId> {
    if path.exists() {
        let existing = std::fs::read_to_string(path)?;
        if let Ok(parsed) = Uuid::parse_str(existing.trim()) {
            return Ok(*parsed.as_bytes());
        }
    }

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let id = Uuid::new_v4();
    std::fs::write(path, format!("{id}\n"))?;
    Ok(*id.as_bytes())
}
