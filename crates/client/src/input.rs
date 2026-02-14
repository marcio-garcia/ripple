use std::{io::Result, net::UdpSocket, time::{Duration, Instant}};
use common::{TYPE_REQUEST_ANALYTICS, TrafficClass};
use crossterm::event::KeyCode;
use crate::transmission::{ClientState, SendMode, schedule_burst};

pub enum InputCommand {
    SendSingle,
    SendBurst,
    SetBurstCount(u32),
    StartContinuous { class: TrafficClass, rate: u32 },
    StopContinuous,
    RequestAnalytics,
}

pub fn handle_input(key: KeyCode) -> Option<InputCommand> {
    match key {
        KeyCode::Char(c) => {
            match c {
                ' ' => Some(InputCommand::SendSingle),
                'b' => Some(InputCommand::SendBurst),
                '1'..='9' => {
                    let count = c.to_digit(10).unwrap_or(1);
                    Some(InputCommand::SetBurstCount(count))
                },
                'a' => Some(InputCommand::StartContinuous {
                    class: TrafficClass::Api,
                    rate: 100,
                }),
                'h' => Some(InputCommand::StartContinuous {
                    class: TrafficClass::HeavyCompute,
                    rate: 10,
                }),
                'g' => Some(InputCommand::StartContinuous {
                    class: TrafficClass::Background,
                    rate: 1000,
                }),
                's' => Some(InputCommand::StopContinuous),
                'r' => Some(InputCommand::RequestAnalytics),
                _ => None,
            }
        },
        _ => None
    }
}

pub fn execute_command(
    command: InputCommand,
    state: &mut ClientState,
    socket: &UdpSocket,
    server_addr: &str,
) -> Result<()> {
    match command {
        InputCommand::SendSingle => {
            schedule_burst(
                &mut state.queue,
                Instant::now(),
                1,
                10,
                TrafficClass::HealthCheck,
                1200
            );
            Ok(())
        },
        InputCommand::SendBurst => {
            schedule_burst(
                &mut state.queue,
                Instant::now(),
                state.burst_count,
                10,
                TrafficClass::Background,
                1200
            );
            Ok(())
        },
        InputCommand::SetBurstCount(n) => {
            state.burst_count = n * 100;
            print!("Burst count now: {}", state.burst_count);
            Ok(())
        },
        InputCommand::StartContinuous { class, rate } => {
            let interval = Duration::from_secs(1) / rate;
            state.send_mode = SendMode::Continuous {
                class,
                packets_per_second: rate,
                last_send: Instant::now(),
                interval,
            };
            state.queue.clear();  // Stop burst mode
            print!("Continuous mode: {} at {} pps", class, rate);
            Ok(())
        },
        InputCommand::StopContinuous => {
            state.send_mode = SendMode::Idle;
            print!("Continuous mode stopped");
            Ok(())
        },
        InputCommand::RequestAnalytics => {
            let pkt = common::pack_data_packet(
                0,
                TYPE_REQUEST_ANALYTICS,
                common::TrafficClass::HealthCheck,
                state.client_start,
                24  // Header only
            );
            socket.send_to(&pkt, server_addr)?;
            print!("Requesting analytics...");
            Ok(())
        },
    }
}
