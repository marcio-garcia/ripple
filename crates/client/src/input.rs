use std::time::Instant;
use common::TrafficClass;
use crossterm::event::KeyCode;

use crate::transmission::{ClientState, schedule_burst};

pub enum InputCommand {
    SendSingle,
    SendBurst,
    SetBurstCount(u32),
    // Could add more: ChangeBurstSpeed, ChangeTrafficClass, etc.
}

pub fn handle_input(key: KeyCode) -> Option<InputCommand> {
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

pub fn execute_command(
    command: InputCommand,
    state: &mut ClientState,
) {
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
        },
        InputCommand::SetBurstCount(n) => {
            state.burst_count = n * 100;
            print!("Burst count now: {}", state.burst_count);
        },
    }
}
