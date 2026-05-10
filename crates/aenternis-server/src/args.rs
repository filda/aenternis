//! CLI / env arg parsing for `aenternis-server`.
//!
//! Hand-rolled (no `clap` dependency). Two flags, two env vars,
//! preference order: CLI > env > built-in default. Pure: reads no
//! globals, performs no I/O. The driver in `main.rs` injects
//! `std::env::args_os()` and a closure over `std::env::var()`.

use std::ffi::OsString;
use std::net::IpAddr;
use std::str::FromStr;

/// Parsed server arguments, ready to bind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Args {
    /// Host IP to bind. Defaults to `127.0.0.1` (loopback only).
    pub(crate) host: IpAddr,
    /// Port to bind. Defaults to `8765`.
    pub(crate) port: u16,
}

impl Args {
    /// Default host: loopback only, so a misconfigured dev box never
    /// accidentally exposes the simulation to the network.
    pub(crate) const DEFAULT_HOST: &'static str = "127.0.0.1";
    /// Default port: 8765, picked from the IANA user-port range with
    /// no known conflicts on common dev environments.
    pub(crate) const DEFAULT_PORT: u16 = 8765;

    /// Whether the bound host is loopback. Used to decide whether to
    /// emit the "no auth, LAN-only" warning at startup.
    #[must_use]
    pub(crate) const fn is_loopback(&self) -> bool {
        self.host.is_loopback()
    }
}

/// Argument-parsing failure. Distinct from `--help`, which is a
/// non-error path signalled via [`ParseOutcome::Help`].
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ArgError {
    /// Unknown flag passed on the command line.
    UnknownFlag(String),
    /// A flag expected a value but the iterator was exhausted.
    MissingValue(&'static str),
    /// A value was provided but couldn't be parsed in the expected
    /// format (IP for `--host`, `u16` for `--port`).
    InvalidValue {
        /// Which flag the bad value belonged to.
        flag: &'static str,
        /// The offending value, captured so the error message is useful.
        value: String,
    },
}

impl std::fmt::Display for ArgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownFlag(flag) => write!(f, "unknown flag: {flag}"),
            Self::MissingValue(flag) => write!(f, "missing value for flag: {flag}"),
            Self::InvalidValue { flag, value } => {
                write!(f, "invalid value for {flag}: {value:?}")
            }
        }
    }
}

impl std::error::Error for ArgError {}

/// Outcome of [`parse`]: either the parsed [`Args`] or a request to
/// print [`USAGE`] and exit. Kept as an enum (rather than collapsing
/// `Help` into `ArgError`) so `--help` exits with status 0.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ParseOutcome {
    /// Parsed args, ready to bind.
    Args(Args),
    /// User asked for `--help` / `-h`.
    Help,
}

/// Usage banner. Printed on `--help` (stdout) and prepended to the
/// error message on a parse failure (stderr).
pub(crate) const USAGE: &str = "aenternis-server \u{2014} native dev backend for Aenternis

USAGE:
    aenternis-server [--host HOST] [--port PORT]

OPTIONS:
    --host HOST    Bind address. Default 127.0.0.1.
                   Pass 0.0.0.0 to expose on LAN. Env: AENTERNIS_HOST.
    --port PORT    TCP port. Default 8765. Env: AENTERNIS_PORT.
    -h, --help     Show this message.";

/// Parse CLI args (excluding `argv[0]`) plus an env getter into an
/// [`Args`] (or `Help`). CLI flags override env vars; env vars
/// override defaults.
pub(crate) fn parse<I, E>(argv: I, env: E) -> Result<ParseOutcome, ArgError>
where
    I: IntoIterator<Item = OsString>,
    E: Fn(&str) -> Option<String>,
{
    let mut iter = argv.into_iter();
    let mut host_arg: Option<String> = None;
    let mut port_arg: Option<String> = None;

    while let Some(raw) = iter.next() {
        let s = raw.to_string_lossy().into_owned();
        match s.as_str() {
            "-h" | "--help" => return Ok(ParseOutcome::Help),
            "--host" => {
                let v = iter
                    .next()
                    .ok_or(ArgError::MissingValue("--host"))?
                    .to_string_lossy()
                    .into_owned();
                host_arg = Some(v);
            }
            "--port" => {
                let v = iter
                    .next()
                    .ok_or(ArgError::MissingValue("--port"))?
                    .to_string_lossy()
                    .into_owned();
                port_arg = Some(v);
            }
            // Long-form `--host=VALUE` / `--port=VALUE` for callers who
            // prefer one shell token per flag.
            other if other.starts_with("--host=") => {
                host_arg = Some(other.trim_start_matches("--host=").to_owned());
            }
            other if other.starts_with("--port=") => {
                port_arg = Some(other.trim_start_matches("--port=").to_owned());
            }
            other => return Err(ArgError::UnknownFlag(other.to_owned())),
        }
    }

    let host_str = host_arg
        .or_else(|| env("AENTERNIS_HOST"))
        .unwrap_or_else(|| Args::DEFAULT_HOST.to_owned());
    let port_str = port_arg
        .or_else(|| env("AENTERNIS_PORT"))
        .unwrap_or_else(|| Args::DEFAULT_PORT.to_string());

    let host = IpAddr::from_str(&host_str).map_err(|_| ArgError::InvalidValue {
        flag: "--host",
        value: host_str,
    })?;
    let port = port_str
        .parse::<u16>()
        .map_err(|_| ArgError::InvalidValue {
            flag: "--port",
            value: port_str,
        })?;

    Ok(ParseOutcome::Args(Args { host, port }))
}

#[cfg(test)]
mod tests {
    use super::{parse, ArgError, Args, ParseOutcome};
    use std::ffi::OsString;

    fn argv(items: &[&str]) -> Vec<OsString> {
        items.iter().map(|s| OsString::from(*s)).collect()
    }

    fn no_env(_: &str) -> Option<String> {
        None
    }

    fn unwrap_args(outcome: ParseOutcome) -> Args {
        match outcome {
            ParseOutcome::Args(a) => a,
            ParseOutcome::Help => panic!("expected Args, got Help"),
        }
    }

    #[test]
    fn defaults_when_no_args_no_env() {
        let parsed = unwrap_args(parse(argv(&[]), no_env).unwrap());
        assert_eq!(parsed.host.to_string(), "127.0.0.1");
        assert_eq!(parsed.port, 8765);
        assert!(parsed.is_loopback());
    }

    #[test]
    fn host_flag_overrides_default() {
        let parsed = unwrap_args(parse(argv(&["--host", "0.0.0.0"]), no_env).unwrap());
        assert_eq!(parsed.host.to_string(), "0.0.0.0");
        assert!(!parsed.is_loopback());
    }

    #[test]
    fn port_flag_overrides_default() {
        let parsed = unwrap_args(parse(argv(&["--port", "9000"]), no_env).unwrap());
        assert_eq!(parsed.port, 9000);
    }

    #[test]
    fn equals_form_works_for_both() {
        let parsed = unwrap_args(parse(argv(&["--host=10.0.0.1", "--port=1234"]), no_env).unwrap());
        assert_eq!(parsed.host.to_string(), "10.0.0.1");
        assert_eq!(parsed.port, 1234);
    }

    #[test]
    fn cli_overrides_env() {
        let env = |k: &str| (k == "AENTERNIS_HOST").then(|| "0.0.0.0".to_owned());
        let parsed = unwrap_args(parse(argv(&["--host", "127.0.0.1"]), env).unwrap());
        assert_eq!(parsed.host.to_string(), "127.0.0.1");
    }

    #[test]
    fn env_used_when_no_cli() {
        let env = |k: &str| match k {
            "AENTERNIS_HOST" => Some("192.168.1.10".to_owned()),
            "AENTERNIS_PORT" => Some("9999".to_owned()),
            _ => None,
        };
        let parsed = unwrap_args(parse(argv(&[]), env).unwrap());
        assert_eq!(parsed.host.to_string(), "192.168.1.10");
        assert_eq!(parsed.port, 9999);
    }

    #[test]
    fn ipv6_loopback_is_loopback() {
        let parsed = unwrap_args(parse(argv(&["--host", "::1"]), no_env).unwrap());
        assert!(parsed.is_loopback());
    }

    #[test]
    fn ipv6_non_loopback_is_not_loopback() {
        let parsed = unwrap_args(parse(argv(&["--host", "::"]), no_env).unwrap());
        assert!(!parsed.is_loopback());
    }

    #[test]
    fn help_short_form() {
        assert_eq!(parse(argv(&["-h"]), no_env), Ok(ParseOutcome::Help));
    }

    #[test]
    fn help_long_form() {
        assert_eq!(parse(argv(&["--help"]), no_env), Ok(ParseOutcome::Help));
    }

    #[test]
    fn unknown_flag_is_an_error() {
        let err = parse(argv(&["--bogus"]), no_env).unwrap_err();
        assert!(matches!(err, ArgError::UnknownFlag(ref f) if f == "--bogus"));
    }

    #[test]
    fn missing_host_value() {
        let err = parse(argv(&["--host"]), no_env).unwrap_err();
        assert_eq!(err, ArgError::MissingValue("--host"));
    }

    #[test]
    fn missing_port_value() {
        let err = parse(argv(&["--port"]), no_env).unwrap_err();
        assert_eq!(err, ArgError::MissingValue("--port"));
    }

    #[test]
    fn invalid_host_rejected() {
        let err = parse(argv(&["--host", "not-an-ip"]), no_env).unwrap_err();
        assert!(matches!(err, ArgError::InvalidValue { flag: "--host", .. }));
    }

    #[test]
    fn invalid_port_rejected() {
        let err = parse(argv(&["--port", "999999"]), no_env).unwrap_err();
        assert!(matches!(err, ArgError::InvalidValue { flag: "--port", .. }));
    }

    #[test]
    fn empty_env_value_falls_through_when_present() {
        // Env returning Some("") is used as-is — caller can distinguish
        // "unset" from "set to empty" by returning None vs Some(""). An
        // empty string fails IP parsing, surfacing a clear error.
        let env = |k: &str| (k == "AENTERNIS_HOST").then(String::new);
        let err = parse(argv(&[]), env).unwrap_err();
        assert!(matches!(err, ArgError::InvalidValue { flag: "--host", .. }));
    }

    #[test]
    fn arg_error_display_messages() {
        assert_eq!(
            ArgError::UnknownFlag("--bogus".to_owned()).to_string(),
            "unknown flag: --bogus"
        );
        assert_eq!(
            ArgError::MissingValue("--host").to_string(),
            "missing value for flag: --host"
        );
        assert_eq!(
            ArgError::InvalidValue {
                flag: "--port",
                value: "abc".to_owned()
            }
            .to_string(),
            "invalid value for --port: \"abc\""
        );
    }
}
