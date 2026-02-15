use std::env;
use std::io::{Error, ErrorKind, Result};

pub fn parse_server_addr_args() -> Result<String> {
    let mut server = String::from("127.0.0.1");
    let mut port: u16 = 8080;
    let mut args = env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-s" | "--server" => {
                let value = args.next().ok_or_else(|| {
                    Error::new(ErrorKind::InvalidInput, "missing value for -s/--server")
                })?;
                server = value;
            }
            "-p" | "--port" => {
                let value = args.next().ok_or_else(|| {
                    Error::new(ErrorKind::InvalidInput, "missing value for -p/--port")
                })?;
                port = value.parse::<u16>().map_err(|_| {
                    Error::new(ErrorKind::InvalidInput, format!("invalid port: {value}"))
                })?;
            }
            "-h" | "--help" => {
                println!("Usage: client [-s|--server <host>] [-p|--port <port>]");
                std::process::exit(0);
            }
            _ => {
                return Err(Error::new(
                    ErrorKind::InvalidInput,
                    format!("unknown argument: {arg}"),
                ));
            }
        }
    }

    Ok(format!("{server}:{port}"))
}
