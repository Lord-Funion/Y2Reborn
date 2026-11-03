//! The platform's radio daemon (`y2-radiod`): its socket and one request with its answer. Bluetooth
//! (`bluetooth.rs`) keeps a subscription open; Wi-Fi's switch and Diagnostics ask once.
#![forbid(unsafe_code)]
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    time::Duration,
};
use y2p_contract::radiod::{Message, Request};

/// Where the radio daemon listens.
pub(crate) const SOCKET: &str = "/run/y2/radiod.sock";

/// One request as a line.
pub(crate) fn send(writer: &mut UnixStream, request: &Request) -> std::io::Result<()> {
    let mut line = serde_json::to_string(request).map_err(std::io::Error::other)?;
    line.push('\n');
    writer.write_all(line.as_bytes())
}

/// One request and its answer; Err("radio daemon unavailable") when it does not run.
pub(crate) fn ask(request: &Request) -> Result<Message, String> {
    let mut stream =
        UnixStream::connect(SOCKET).map_err(|_| "radio daemon unavailable".to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| e.to_string())?;
    send(&mut stream, request).map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    serde_json::from_str(&line).map_err(|_| "unreadable answer from the radio daemon".into())
}
