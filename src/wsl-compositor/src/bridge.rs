//! Control-channel TCP server. Accepts the Windows host, sends it the shared-framebuffer
//! configuration, forwards incoming input messages into the event loop via a calloop
//! channel, and lets the compositor push frame notifications back to the host.

use std::io::BufReader;
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

use bridge_protocol::{ClientMessage, FrameLayout, ServerMessage};
use smithay::reexports::calloop::channel::Sender;

/// Handle used by the compositor to send [`ServerMessage`]s to the connected host.
pub struct Bridge {
    writer: Arc<Mutex<Option<TcpStream>>>,
}

impl Bridge {
    /// Spawn the TCP server. Binds the first address in `addrs` that is free, then serves on
    /// it. Incoming [`ClientMessage`]s are forwarded through `ctrl_tx`. Returns the handle
    /// and the bound address (`None` if every candidate failed to bind).
    pub fn spawn(
        addrs: Vec<String>,
        host_path: String,
        layout: FrameLayout,
        ctrl_tx: Sender<ClientMessage>,
    ) -> (Bridge, Option<String>) {
        let writer: Arc<Mutex<Option<TcpStream>>> = Arc::new(Mutex::new(None));
        let shared = writer.clone();

        // Bind on the calling thread so we can report the chosen address to the caller.
        let listener = bind_first(&addrs);
        let bound_addr = listener
            .as_ref()
            .and_then(|l| l.local_addr().ok())
            .map(|a| a.to_string());
        match &bound_addr {
            Some(addr) => tracing::info!("control channel listening on {addr}"),
            None => tracing::error!("failed to bind control channel on any of {addrs:?}"),
        }

        if let Some(listener) = listener {
            thread::spawn(move || {
                for incoming in listener.incoming() {
                    let stream = match incoming {
                        Ok(s) => s,
                        Err(e) => {
                            tracing::warn!("accept failed: {e}");
                            continue;
                        }
                    };
                    let _ = stream.set_nodelay(true);
                    tracing::info!("host connected from {:?}", stream.peer_addr());

                    // Send the framebuffer configuration first.
                    let config = ServerMessage::FrameConfig {
                        width: layout.width,
                        height: layout.height,
                        shm_size: layout.total_size(),
                        host_path: host_path.clone(),
                    };
                    let write_half = match stream.try_clone() {
                        Ok(mut w) => {
                            if config.write(&mut w).is_err() {
                                continue;
                            }
                            w
                        }
                        Err(e) => {
                            tracing::warn!("try_clone failed: {e}");
                            continue;
                        }
                    };
                    *shared.lock().unwrap() = Some(write_half);

                    // Read incoming client messages until the connection drops.
                    let mut reader = BufReader::new(stream);
                    loop {
                        match ClientMessage::read(&mut reader) {
                            Ok(msg) => {
                                if ctrl_tx.send(msg).is_err() {
                                    return; // event loop gone
                                }
                            }
                            Err(_) => break,
                        }
                    }

                    *shared.lock().unwrap() = None;
                    tracing::info!("host disconnected");
                }
            });
        }

        (Bridge { writer }, bound_addr)
    }

    /// Send a message to the connected host, if any. Drops the connection on write error.
    pub fn send(&self, msg: &ServerMessage) {
        let mut guard = self.writer.lock().unwrap();
        if let Some(stream) = guard.as_mut() {
            if msg.write(stream).is_err() {
                *guard = None;
            }
        }
    }
}

/// Bind the first address in `addrs` that is free, scanning in order.
fn bind_first(addrs: &[String]) -> Option<TcpListener> {
    for addr in addrs {
        match TcpListener::bind(addr) {
            Ok(l) => return Some(l),
            Err(_) => continue,
        }
    }
    None
}
