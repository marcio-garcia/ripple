use crate::transmission::{
    ClientState, ContinuousState, active_peer_node_id, add_peer, clear_profile, next_peer_node_id,
    register_self, remove_peer, request_topology, run_topology_mixed_classes_test,
    run_topology_removal_test, run_topology_smoke_test, schedule_burst,
    select_or_add_peer_for_domain, select_peer, set_profile_burst, set_profile_oscillation,
    set_profile_ramp, set_profile_steady, unregister_self, update_source_domain,
};
use common::{EndpointDomain, NodeDomain, NodeId, TrafficClass, WireMessage};
use crossterm::event::KeyCode;
use std::io::Error;
use std::{
    io::Result,
    net::UdpSocket,
    time::{Duration, Instant},
};

pub enum InputCommand {
    SendSingle,
    SendBurst,
    SetBurstCount(u32),
    StartContinuous { class: TrafficClass, rate: u32 },
    StopContinuous,
    RegisterSelf,
    UnregisterSelf,
    RequestAnalytics,
    RequestTopology,
    AddPeer { domain: NodeDomain },
    RemovePeer { node_id: NodeId },
    SelectPeer { node_id: NodeId },
    SetProfileSteady,
    SetProfileBurst,
    SetProfileRamp,
    SetProfileOscillation,
    RunTopologySmokeTest,
    RunTopologyRemovalTest,
    RunTopologyMixedClassesTest,
    SetSourceDomain(EndpointDomain),
    SetDestinationDomain(EndpointDomain),
}

pub fn handle_input(key: KeyCode, state: &ClientState) -> Option<InputCommand> {
    match key {
        KeyCode::Char(c) => match c.to_ascii_lowercase() {
            ' ' => Some(InputCommand::SendSingle),
            'b' => Some(InputCommand::SendBurst),
            '1'..='9' => {
                let count = c.to_digit(10).unwrap_or(1);
                Some(InputCommand::SetBurstCount(count))
            }
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
            'v' => Some(InputCommand::RegisterSelf),
            'x' => Some(InputCommand::UnregisterSelf),
            'r' => Some(InputCommand::RequestAnalytics),
            'p' => Some(InputCommand::RequestTopology),
            'f' => Some(InputCommand::SetProfileSteady),
            'z' => Some(InputCommand::SetProfileBurst),
            'w' => Some(InputCommand::SetProfileRamp),
            'o' => Some(InputCommand::SetProfileOscillation),
            't' => Some(InputCommand::RunTopologySmokeTest),
            'y' => Some(InputCommand::RunTopologyRemovalTest),
            'u' => Some(InputCommand::RunTopologyMixedClassesTest),
            'n' => Some(InputCommand::AddPeer {
                domain: NodeDomain::Internal,
            }),
            'j' => Some(InputCommand::AddPeer {
                domain: NodeDomain::External,
            }),
            'c' => next_peer_node_id(state).map(|node_id| InputCommand::SelectPeer { node_id }),
            'm' => active_peer_node_id(state).map(|node_id| InputCommand::RemovePeer { node_id }),
            'i' => Some(InputCommand::SetSourceDomain(EndpointDomain::Internal)),
            'e' => Some(InputCommand::SetSourceDomain(EndpointDomain::External)),
            'k' => Some(InputCommand::SetDestinationDomain(EndpointDomain::Internal)),
            'l' => Some(InputCommand::SetDestinationDomain(EndpointDomain::External)),
            _ => None,
        },
        _ => None,
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
                1200,
            );
            Ok(())
        }
        InputCommand::SendBurst => {
            schedule_burst(
                &mut state.queue,
                Instant::now(),
                state.burst_count,
                10,
                TrafficClass::Background,
                1200,
            );
            Ok(())
        }
        InputCommand::SetBurstCount(n) => {
            state.burst_count = n * 100;
            print!("Burst count now: {}", state.burst_count);
            Ok(())
        }
        InputCommand::StartContinuous { class, rate } => {
            let interval = Duration::from_secs(1) / rate;
            // store the next scheduled send deadline instead of last-send time.
            state.continuous_state = Some(ContinuousState {
                class,
                next_send_at: Instant::now() + interval,
                interval,
            });
            clear_profile(state)?;
            state.queue.clear(); // Stop burst mode
            print!("Continuous mode: {} at {} pps", class, rate);
            Ok(())
        }
        InputCommand::StopContinuous => {
            state.continuous_state = None;
            clear_profile(state)?;
            print!("Continuous mode stopped");
            Ok(())
        }
        InputCommand::RegisterSelf => {
            register_self(state, socket, server_addr)?;
            print!("Self node registered");
            Ok(())
        }
        InputCommand::UnregisterSelf => {
            unregister_self(state, socket, server_addr)?;
            print!("Self node unregistered");
            Ok(())
        }
        InputCommand::RequestAnalytics => {
            let pkt =
                common::encode_message(&WireMessage::RequestAnalytics).map_err(Error::other)?;
            socket.send_to(&pkt, server_addr)?;
            print!("Requesting analytics...");
            Ok(())
        }
        InputCommand::RequestTopology => {
            request_topology(socket, server_addr)?;
            print!("Requesting topology...");
            Ok(())
        }
        InputCommand::AddPeer { domain } => {
            add_peer(state, domain, socket, server_addr)?;
            print!("Added {} peer", format_node_domain(domain));
            Ok(())
        }
        InputCommand::RemovePeer { node_id } => {
            remove_peer(state, node_id, socket, server_addr)?;
            print!("Removed peer {}", short_node_id(&node_id));
            Ok(())
        }
        InputCommand::SelectPeer { node_id } => {
            select_peer(state, node_id)?;
            print!("Selected peer {}", short_node_id(&node_id));
            Ok(())
        }
        InputCommand::SetProfileSteady => {
            set_profile_steady(state)?;
            Ok(())
        }
        InputCommand::SetProfileBurst => {
            set_profile_burst(state)?;
            Ok(())
        }
        InputCommand::SetProfileRamp => {
            set_profile_ramp(state)?;
            Ok(())
        }
        InputCommand::SetProfileOscillation => {
            set_profile_oscillation(state)?;
            Ok(())
        }
        InputCommand::RunTopologySmokeTest => {
            run_topology_smoke_test(state, socket, server_addr)?;
            Ok(())
        }
        InputCommand::RunTopologyRemovalTest => {
            run_topology_removal_test(state, socket, server_addr)?;
            Ok(())
        }
        InputCommand::RunTopologyMixedClassesTest => {
            run_topology_mixed_classes_test(state, socket, server_addr)?;
            Ok(())
        }
        InputCommand::SetSourceDomain(domain) => {
            update_source_domain(state, domain, socket, server_addr)?;
            print!("Source domain now: {}", format_domain(state.src_domain));
            Ok(())
        }
        InputCommand::SetDestinationDomain(domain) => {
            select_or_add_peer_for_domain(state, domain, socket, server_addr)?;
            print!(
                "Destination domain now: {}",
                format_domain(state.dst_domain)
            );
            Ok(())
        }
    }
}

fn format_domain(domain: EndpointDomain) -> &'static str {
    match domain {
        EndpointDomain::Internal => "internal",
        EndpointDomain::External => "external",
    }
}

fn format_node_domain(domain: NodeDomain) -> &'static str {
    match domain {
        NodeDomain::Internal => "internal",
        NodeDomain::External => "external",
    }
}

fn short_node_id(node_id: &NodeId) -> String {
    format!(
        "{:02x}{:02x}{:02x}{:02x}",
        node_id[0], node_id[1], node_id[2], node_id[3]
    )
}
