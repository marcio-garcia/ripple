use std::collections::VecDeque;
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
use common::{TrafficClass, pack_data_packet};

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

enum InputCommand {
    SendSingle,
    SendBurst,
    SetBurstCount(u32),
    // Could add more: ChangeBurstSpeed, ChangeTrafficClass, etc.
}

#[derive(Clone, Copy)]
struct ScheduledSend {
    at: Instant,
    class: TrafficClass,
    declared_bytes: u32,
}

fn run_app(socket: UdpSocket, server_addr: &str) -> Result<()> {
    let mut burst_count: u32 = 200;
    let client_start = Instant::now();
    let mut seq: u32 = 0;
    let mut queue: VecDeque<ScheduledSend> = VecDeque::new();

    loop {
        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => break,
                        _ => {
                            if let Some(command) = handle_input(key.code) {
                                match command {
                                    InputCommand::SendSingle => {
                                        schedule_burst(
                                            &mut queue,
                                            Instant::now(),
                                            1,
                                            10,
                                            TrafficClass::HealthCheck,
                                            1200
                                        );
                                    },
                                    InputCommand::SendBurst => {
                                        schedule_burst(
                                            &mut queue,
                                            Instant::now(),
                                            burst_count,
                                            10,
                                            TrafficClass::Background,
                                            1200
                                        );
                                    },
                                    InputCommand::SetBurstCount(n) => {
                                        burst_count = n * 100;
                                        print!("Burst count now: {}", burst_count);
                                    },
                                }
                            }
                        }
                    }
                }
            }
        }
        stdout().execute(MoveToColumn(0))?;

        let now = Instant::now();
        loop {
            // Peek and copy the front element
            let front = match queue.front() {
                Some(f) if f.at <= now => *f,
                _ => break,
            };

            // Now safe to pop
            queue.pop_front();

            let pkt = pack_data_packet(seq, front.class, client_start, front.declared_bytes);
            let _ = socket.send_to(&pkt, server_addr); // ignore WouldBlock
            seq = seq.wrapping_add(1);
        }
        std::thread::sleep(Duration::from_millis(1));
    }

    Ok(())
}

fn handle_input(key: KeyCode) -> Option<InputCommand> {
    match key {
        KeyCode::Char(c) => {
            match c {
                ' ' => {
                    print!("Pressed: Space");
                    Some(InputCommand::SendSingle)  // Return command
                },
                'b' => {
                    print!("Pressed: Burst");
                    Some(InputCommand::SendBurst)  // Return command
                },
                '1'..='9' => {
                    let count = c.to_digit(10).unwrap_or(1);
                    Some(InputCommand::SetBurstCount(count))  // Return command
                },
                _ => None,
            }
        },
        _ => None
    }
}

fn open_socket() -> Result<UdpSocket>{
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    Ok(socket)
}

fn schedule_burst(
    q: &mut VecDeque<ScheduledSend>,
    now: Instant,
    count: u32,
    interval_ms: u32,
    class: TrafficClass,
    declared_bytes: u32,
) {
    let interval = Duration::from_millis(interval_ms as u64);
    for i in 0..count {
        q.push_back(ScheduledSend {
            at: now + interval * i,
            class,
            declared_bytes,
        });
    }
}
