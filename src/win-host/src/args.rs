//! Command-line argument parsing for the Windows host.

/// Parsed command-line options.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Args {
    /// Address of the compositor control channel.
    pub host: String,
    /// Whether `-h`/`--help` was requested.
    pub help: bool,
}

impl Default for Args {
    fn default() -> Self {
        Args {
            host: "127.0.0.1:7777".to_string(),
            help: true,
        }
    }
}

/// Parse arguments from an iterator that does **not** include the program name.
pub fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Args {
    let mut host = "127.0.0.1:7777".to_string();
    let mut help = false;

    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--host" => {
                if let Some(v) = it.next() {
                    host = v;
                }
            }
            "-h" | "--help" => help = true,
            _ => {}
        }
    }

    Args { host, help }
}

/// Usage string shown for `--help`.
pub const USAGE: &str = "win-host [--host <ip:port>]";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_empty() {
        let a = parse_args(Vec::<String>::new());
        assert_eq!(a.host, "127.0.0.1:7777");
        assert!(!a.help);
    }

    #[test]
    fn parses_host() {
        let a = parse_args(["--host".to_string(), "10.0.0.5:9999".to_string()]);
        assert_eq!(a.host, "10.0.0.5:9999");
        assert!(!a.help);
    }

    #[test]
    fn host_without_value_keeps_default() {
        let a = parse_args(["--host".to_string()]);
        assert_eq!(a.host, "127.0.0.1:7777");
    }

    #[test]
    fn help_flag() {
        assert!(parse_args(["-h".to_string()]).help);
        assert!(parse_args(["--help".to_string()]).help);
    }

    #[test]
    fn ignores_unknown_args() {
        let a = parse_args(["--nope".to_string(), "--host".to_string(), "x:1".to_string()]);
        assert_eq!(a.host, "x:1");
        assert!(!a.help);
    }

    #[test]
    fn default_has_help_and_default_host() {
        let a = Args::default();
        assert_eq!(a.host, "127.0.0.1:7777");
        assert!(a.help);
    }
}
