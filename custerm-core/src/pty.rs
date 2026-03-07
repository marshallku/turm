use std::io::{Read, Write};
use std::sync::mpsc;

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::error::{CustermError, Result};

pub struct PtySession {
    pub master: Box<dyn MasterPty + Send>,
    pub child: Box<dyn Child + Send + Sync>,
    pub input_tx: mpsc::Sender<Vec<u8>>,
}

pub type OutputCallback = Box<dyn Fn(&str, &str) + Send + 'static>;
pub type ExitCallback = Box<dyn Fn(&str) + Send + 'static>;

/// Spawn a new PTY session. Returns `(session_id, PtySession)`.
///
/// `on_output(session_id, data)` is called from the reader thread.
/// `on_exit(session_id)` is called when the process exits.
pub fn spawn_session(
    shell: &str,
    cols: u16,
    rows: u16,
    env_vars: &[(&str, &str)],
    on_output: OutputCallback,
    on_exit: ExitCallback,
) -> Result<(String, PtySession)> {
    let pty_system = native_pty_system();

    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| CustermError::Pty(e.to_string()))?;

    let mut cmd = CommandBuilder::new(shell);
    cmd.env("TERM", "xterm-256color");
    for (k, v) in env_vars {
        cmd.env(k, v);
    }

    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| CustermError::Pty(e.to_string()))?;
    drop(pair.slave);

    let mut writer = pair
        .master
        .take_writer()
        .map_err(|e| CustermError::Pty(e.to_string()))?;

    // Input channel -> dedicated writer thread (no Mutex on hot path)
    let (input_tx, input_rx) = mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        while let Ok(data) = input_rx.recv() {
            if writer.write_all(&data).is_err() {
                break;
            }
        }
    });

    // Reader thread
    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| CustermError::Pty(e.to_string()))?;

    let session_id = uuid::Uuid::new_v4().to_string();
    let id_for_thread = session_id.clone();

    std::thread::spawn(move || {
        let mut buf = [0u8; 65536];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => {
                    on_exit(&id_for_thread);
                    break;
                }
                Ok(n) => {
                    let data = String::from_utf8_lossy(&buf[..n]);
                    on_output(&id_for_thread, &data);
                }
                Err(_) => {
                    on_exit(&id_for_thread);
                    break;
                }
            }
        }
    });

    Ok((
        session_id,
        PtySession {
            master: pair.master,
            child,
            input_tx,
        },
    ))
}
