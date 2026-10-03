//! TCP control-channel client. Connects to the WSL1 compositor over `127.0.0.1`, forwards
//! input/resize messages from the window, and delivers server messages to the winit event
//! loop via an [`EventLoopProxy`].

use std::io::BufReader;
use std::net::TcpStream;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use bridge_protocol::{ClientMessage, ServerMessage};
use winit::event_loop::EventLoopProxy;

use crate::UserEvent;

/// Spawn the control-channel client. Returns a sender for outgoing [`ClientMessage`]s.
///
/// `candidates` is the ordered list of `ip:port` addresses to try; the client connects to
/// the first that answers. It keeps retrying until the compositor is reachable, and
/// transparently reconnects (re-scanning the list) if the link drops. Incoming
/// [`ServerMessage`]s are wrapped in [`UserEvent`] and pushed to the event loop.
pub fn spawn(candidates: Vec<String>, proxy: EventLoopProxy<UserEvent>) -> Sender<ClientMessage> {
    let (tx, rx) = std::sync::mpsc::channel::<ClientMessage>();

    // The write half of the current connection, shared with the writer thread. `None` while
    // disconnected.
    let writer: Arc<Mutex<Option<TcpStream>>> = Arc::new(Mutex::new(None));

    // Writer thread: drains outgoing messages and writes to whatever stream is live.
    {
        let writer = writer.clone();
        thread::spawn(move || {
            while let Ok(msg) = rx.recv() {
                let mut guard = writer.lock().unwrap();
                if let Some(stream) = guard.as_mut() {
                    if msg.write(stream).is_err() {
                        *guard = None; // drop broken connection; manager will reconnect
                    }
                }
            }
        });
    }

    // Manager thread: (re)connects and reads server messages.
    thread::spawn(move || loop {
        let (stream, addr) = connect_any(&candidates);
        log::info!("connected to compositor at {addr}");
        let _ = stream.set_nodelay(true);

        match stream.try_clone() {
            Ok(w) => *writer.lock().unwrap() = Some(w),
            Err(e) => {
                log::error!("try_clone failed: {e}");
                thread::sleep(Duration::from_millis(500));
                continue;
            }
        }

        let mut reader = BufReader::new(stream);
        loop {
            match ServerMessage::read(&mut reader) {
                Ok(msg) => {
                    if proxy.send_event(UserEvent::Server(msg)).is_err() {
                        return; // event loop gone
                    }
                }
                Err(e) => {
                    log::warn!("control channel closed: {e}");
                    break;
                }
            }
        }

        *writer.lock().unwrap() = None;
        thread::sleep(Duration::from_millis(500));
    });

    tx
}

/// Block until one of `candidates` accepts a connection, scanning the list in order and
/// pausing briefly between full passes. Returns the live stream and the address it reached.
fn connect_any(candidates: &[String]) -> (TcpStream, String) {
    loop {
        for addr in candidates {
            if let Ok(s) = TcpStream::connect(addr) {
                return (s, addr.clone());
            }
        }
        thread::sleep(Duration::from_millis(500));
    }
}

