use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use turm_core::protocol::{Request, Response};

pub fn send_command(
    socket_path: &str,
    method: &str,
    params: serde_json::Value,
) -> Result<Response, Box<dyn std::error::Error>> {
    let stream = UnixStream::connect(socket_path)?;
    // Read timeout sits just above the supervisor's action timeout
    // (30s default) so a slow service-plugin call can return a
    // structured `action_timeout` response over the socket instead of
    // failing at the transport layer. Bump if the supervisor's
    // `DEFAULT_ACTION_TIMEOUT` ever changes.
    stream.set_read_timeout(Some(Duration::from_secs(35)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;

    let request = Request {
        id: uuid::Uuid::new_v4().to_string(),
        method: method.to_string(),
        params,
    };

    let mut writer = stream.try_clone()?;
    let line = serde_json::to_string(&request)?;
    writeln!(writer, "{line}")?;
    writer.flush()?;

    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let line = line?;
        if line.is_empty() {
            continue;
        }
        let response: Response = serde_json::from_str(&line)?;
        if response.id == request.id {
            return Ok(response);
        }
    }

    Err("No response received".into())
}

pub fn subscribe(socket_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let stream = UnixStream::connect(socket_path)?;
    stream.set_read_timeout(None)?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;

    let request = Request {
        id: uuid::Uuid::new_v4().to_string(),
        method: "event.subscribe".to_string(),
        params: serde_json::json!({}),
    };

    let mut writer = stream.try_clone()?;
    let line = serde_json::to_string(&request)?;
    writeln!(writer, "{line}")?;
    writer.flush()?;

    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let line = line?;
        if line.is_empty() {
            continue;
        }
        println!("{line}");
    }

    Ok(())
}
