//! Command-line argument parsing for the Windows host.

use bridge_protocol::Backend;

/// Transport used to reach the compositor. Only TCP exists today; the flag is here so other
/// transports (e.g. shared-memory signalling) can be added without changing the CLI shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionType {
    Tcp,
}

/// Parsed command-line options.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Args {
    /// Explicit `ip:port` of the compositor control channel, or `None` to auto-discover by
    /// scanning upward from [`bridge_protocol::DEFAULT_PORT`].
    pub host: Option<String>,
    /// Selected transport.
    pub connection_type: ConnectionType,
    /// Render backend to use. Defaults to [`Backend::Webgpu`]. If the compositor's handshake
    /// advertises a different backend, the host-side value is overridden to match it so a
    /// single `--backend` on either side is consistent.
    pub backend: Backend,
    /// Whether to print usage and exit (`-h`/`--help`, or an invalid flag value).
    pub help: bool,
}

impl Default for Args {
    fn default() -> Self {
        Args {
            host: None,
            connection_type: ConnectionType::Tcp,
            backend: Backend::Webgpu,
            help: false,
        }
    }
}

/// Parse arguments from an iterator that does **not** include the program name.
pub fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Args {
    let mut host: Option<String> = None;
    let mut connection_type = ConnectionType::Tcp;
    let mut backend = Backend::Webgpu;
    let mut help = false;

    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--host" => {
                if let Some(v) = it.next() {
                    host = Some(v);
                }
            }
            "--connection-type" => match it.next().as_deref() {
                Some("tcp") => connection_type = ConnectionType::Tcp,
                // Any other (or missing) value is unsupported for now: show help and exit.
                _ => help = true,
            },
            "--backend" => match it.next().as_deref().and_then(Backend::parse) {
                Some(b) => backend = b,
                None => help = true,
            },
            "-h" | "--help" => help = true,
            _ => {}
        }
    }

    Args {
        host,
        connection_type,
        backend,
        help,
    }
}

/// Resolve the ordered list of addresses to try: the explicit `--host` if given, otherwise
/// the default upward port scan shared with the compositor.
pub fn candidate_addrs(host: &Option<String>) -> Vec<String> {
    match host {
        Some(h) => vec![h.clone()],
        None => bridge_protocol::default_scan_addrs(),
    }
}

/// Usage string shown for `--help`.
pub const USAGE: &str =
    "win-host [--host <ip:port>] [--connection-type tcp] [--backend webgpu|vulkan]\n\
     \n\
     --host             compositor control channel address. If omitted, scan 127.0.0.1\n\
     \u{20}                  starting at port 8335 and connect to the first that answers.\n\
     --connection-type  transport to use. Only 'tcp' is supported (the default).\n\
     --backend          host renderer: 'webgpu' (default) or 'vulkan' (ash + gpu-allocator).\n\
     \u{20}                  The compositor's handshake backend overrides this if they differ.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_empty() {
        let a = parse_args(Vec::<String>::new());
        assert_eq!(a.host, None);
        assert_eq!(a.connection_type, ConnectionType::Tcp);
        assert_eq!(a.backend, Backend::Webgpu);
        assert!(!a.help);
    }

    #[test]
    fn backend_defaults_to_webgpu() {
        assert_eq!(parse_args(Vec::<String>::new()).backend, Backend::Webgpu);
        assert_eq!(Args::default().backend, Backend::Webgpu);
    }

    #[test]
    fn parses_backend_vulkan() {
        let a = parse_args(["--backend".to_string(), "vulkan".to_string()]);
        assert_eq!(a.backend, Backend::Vulkan);
        assert!(!a.help);
    }

    #[test]
    fn parses_backend_webgpu() {
        let a = parse_args(["--backend".to_string(), "webgpu".to_string()]);
        assert_eq!(a.backend, Backend::Webgpu);
        assert!(!a.help);
    }

    #[test]
    fn backend_invalid_requests_help() {
        let a = parse_args(["--backend".to_string(), "metal".to_string()]);
        assert!(a.help);
    }

    #[test]
    fn parses_host() {
        let a = parse_args(["--host".to_string(), "10.0.0.5:9999".to_string()]);
        assert_eq!(a.host, Some("10.0.0.5:9999".to_string()));
        assert!(!a.help);
    }

    #[test]
    fn host_without_value_keeps_default() {
        let a = parse_args(["--host".to_string()]);
        assert_eq!(a.host, None);
    }

    #[test]
    fn connection_type_tcp_ok() {
        let a = parse_args(["--connection-type".to_string(), "tcp".to_string()]);
        assert_eq!(a.connection_type, ConnectionType::Tcp);
        assert!(!a.help);
    }

    #[test]
    fn connection_type_invalid_requests_help() {
        let a = parse_args(["--connection-type".to_string(), "shm".to_string()]);
        assert!(a.help);
    }

    #[test]
    fn connection_type_missing_value_requests_help() {
        let a = parse_args(["--connection-type".to_string()]);
        assert!(a.help);
    }

    #[test]
    fn help_flag() {
        assert!(parse_args(["-h".to_string()]).help);
        assert!(parse_args(["--help".to_string()]).help);
    }

    #[test]
    fn ignores_unknown_args() {
        let a = parse_args(["--nope".to_string(), "--host".to_string(), "x:1".to_string()]);
        assert_eq!(a.host, Some("x:1".to_string()));
        assert!(!a.help);
    }

    #[test]
    fn candidates_explicit_host_is_single() {
        let c = candidate_addrs(&Some("1.2.3.4:5".to_string()));
        assert_eq!(c, vec!["1.2.3.4:5".to_string()]);
    }

    #[test]
    fn candidates_default_is_port_scan() {
        let c = candidate_addrs(&None);
        assert_eq!(c, bridge_protocol::default_scan_addrs());
        assert_eq!(c[0], "127.0.0.1:8335");
    }

    #[test]
    fn default_is_tcp_autodiscover() {
        let a = Args::default();
        assert_eq!(a.host, None);
        assert_eq!(a.connection_type, ConnectionType::Tcp);
        assert!(!a.help);
    }
}
