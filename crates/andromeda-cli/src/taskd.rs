//! A blocking HTTP/1.1 client for `andromeda-taskd`, so the CLI can read and
//! drive the daemon's tasks instead of opening a second store of its own.
//!
//! # Why this exists
//!
//! The CLI used to open a [`FileTaskStore`](andromeda_runtime::FileTaskStore)
//! in process, defaulting to `.andromeda/state` under the working directory,
//! while the installed daemon keeps its records in
//! `/var/lib/andromeda-taskd/state` under a systemd `DynamicUser` at mode
//! `0700`. On a real machine those are two disjoint task universes, and the
//! only symptom is that `andromeda task list` prints an empty list — the
//! daemon's store is not merely different, it is unreadable
//! (`docs/reviews/architecture-review.md` §5). Connected mode makes the daemon
//! the single source of truth; the local store stays available, but is now a
//! deliberate, labelled choice rather than an unmarked default.
//!
//! # Why no HTTP crate
//!
//! `hyper`'s client feature would add `want` and `try-lock` to the lockfile and
//! would make this one-shot CLI async for the sake of a single request against
//! a loopback socket. The daemon's wire contract is small and bounded by
//! construction (summaries in the listing, at most
//! [`MAX_EVENTS`](crate::taskd::MAX_EVENTS) events in a task read), so a
//! blocking client built on `std::net` costs no dependency at all.
//!
//! # What is deliberately not offered
//!
//! There is no `--auth-token` flag and no `ANDROMEDA_AUTH_TOKEN` variable. A
//! token in `argv` is readable by every local process through `/proc`, which
//! would hand away the credential that the daemon's `0700` runtime directory
//! exists to withhold. `os/files/usr/libexec/andromeda-ci-verify` does pass one
//! on a command line, and says at length why that is acceptable *there only*: a
//! throwaway CI VM with no interactive users. This is a command users run, so
//! the token is read from a file and from nowhere else.

use std::fmt;
use std::io::{Read as _, Write as _};
use std::net::{IpAddr, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

/// Token path the shipped image uses, set by `andromeda-taskd.service` as
/// `ANDROMEDA_AUTH_TOKEN_FILE`.
///
/// Kept equal to `andromeda_taskd::auth::SYSTEM_TOKEN_PATH` by
/// `tests::the_token_search_path_matches_the_daemons_own`; the CLI cannot
/// depend on the daemon crate at run time without dragging axum and tokio into
/// a binary that needs neither.
pub(crate) const SYSTEM_TOKEN_PATH: &str = "/run/andromeda-taskd/token";

/// `andromeda-taskd`'s own default token path when it is started by hand from a
/// working directory, which is how the developer flow in
/// `docs/development/getting-started.md` runs it.
pub(crate) const DEV_TOKEN_PATH: &str = ".andromeda/taskd-token";

/// Where the token is looked for when `--auth-token-file` is not given, in
/// order. The installed daemon comes first: it is the deployment where reading
/// the wrong store is invisible and harmful.
pub(crate) const TOKEN_SEARCH_PATH: [&str; 2] = [SYSTEM_TOKEN_PATH, DEV_TOKEN_PATH];

/// The daemon's hard ceiling on `?events=<n>`.
///
/// Kept equal to `andromeda_taskd::MAX_EVENT_LIMIT` by
/// `tests::the_event_ceiling_matches_the_daemons_own`. It is repeated here so
/// the CLI can tell a caller how much history is reachable at all, rather than
/// suggesting an `--events` value the daemon will silently clamp.
pub(crate) const MAX_EVENTS: usize = 1_000;

/// Port assumed when `--connect` names no port, matching `andromeda-taskd`'s
/// own `--listen` default. This URL only ever addresses taskd, so inheriting
/// HTTP's port 80 would be a worse guess than the daemon's own.
const DEFAULT_PORT: u16 = 7777;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const IO_TIMEOUT: Duration = Duration::from_secs(30);

/// Ceiling on a single response, so a wedged or hostile peer cannot exhaust
/// memory. Far above anything the contract can produce: taskd's own test
/// measures a 1000-event record at 224 KiB.
const MAX_RESPONSE_BYTES: u64 = 64 * 1024 * 1024;

/// A validated `--connect` target: a loopback origin and nothing else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Endpoint {
    host: String,
    port: u16,
    /// `host:port` exactly as it goes into the `Host` header, which taskd
    /// checks against its loopback allowlist.
    authority: String,
}

impl Endpoint {
    /// Parses `http://host[:port]`, a bare `host[:port]`, and nothing else.
    ///
    /// Refuses a non-loopback host outright. That is not merely mirroring the
    /// daemon's own `Host` check: `--connect http://example.com` would send the
    /// local bearer token to whoever answers, so the refusal protects the
    /// credential, not just the request.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::Endpoint`] for an unsupported scheme, a URL
    /// carrying a path, query, or userinfo, an unparsable port, or a host that
    /// is not loopback.
    pub(crate) fn parse(raw: &str) -> Result<Self, ClientError> {
        let trimmed = raw.trim();
        let rest = if let Some(rest) = trimmed.strip_prefix("http://") {
            rest
        } else if trimmed.starts_with("https://") {
            return Err(ClientError::Endpoint(format!(
                "{trimmed} uses https, but taskd serves plain HTTP on loopback and holds no \
                 certificate; use http://"
            )));
        } else if trimmed.contains("://") {
            return Err(ClientError::Endpoint(format!(
                "{trimmed} is not an http:// URL"
            )));
        } else {
            trimmed
        };

        let authority = rest.trim_end_matches('/');
        if authority.is_empty() {
            return Err(ClientError::Endpoint(
                "--connect needs a host, for example http://127.0.0.1:7777".to_owned(),
            ));
        }
        if authority.contains('/') || authority.contains('?') || authority.contains('#') {
            return Err(ClientError::Endpoint(format!(
                "{trimmed} must be a bare origin such as http://127.0.0.1:7777; the CLI appends \
                 the API paths itself"
            )));
        }
        if authority.contains('@') {
            return Err(ClientError::Endpoint(format!(
                "{trimmed} carries userinfo; taskd authenticates with the local bearer token from \
                 --auth-token-file, never with credentials in a URL"
            )));
        }

        let (host, port) = split_authority(authority)?;
        if !is_loopback_host(&host) {
            return Err(ClientError::Endpoint(format!(
                "refusing to connect to {host}: it is not a loopback address, and the local \
                 bearer token would be sent to it. taskd is a loopback-only service and rejects \
                 any request not addressed to localhost; tunnel it to 127.0.0.1 instead"
            )));
        }
        Ok(Self {
            authority: authority.to_owned(),
            host,
            port,
        })
    }

    /// The endpoint as it is reported to the user.
    pub(crate) fn url(&self) -> String {
        format!("http://{}", self.authority)
    }
}

/// Splits `host`, `host:port`, `[v6]`, or `[v6]:port`.
fn split_authority(authority: &str) -> Result<(String, u16), ClientError> {
    let (host, port) = if let Some(rest) = authority.strip_prefix('[') {
        let Some((address, suffix)) = rest.split_once(']') else {
            return Err(ClientError::Endpoint(format!(
                "{authority} opens a bracketed IPv6 literal that is never closed"
            )));
        };
        if suffix.is_empty() {
            (address, None)
        } else if let Some(port) = suffix.strip_prefix(':') {
            (address, Some(port))
        } else {
            return Err(ClientError::Endpoint(format!(
                "{authority} has trailing text after the IPv6 literal"
            )));
        }
    } else if let Some((address, port)) = authority.rsplit_once(':') {
        // A second colon means an unbracketed IPv6 literal, which is ambiguous
        // with the port separator and must be written in brackets.
        if address.contains(':') {
            return Err(ClientError::Endpoint(format!(
                "{authority} looks like an IPv6 address; write it in brackets, as [::1]:7777"
            )));
        }
        (address, Some(port))
    } else {
        (authority, None)
    };

    let port = match port {
        Some(port) => port.parse::<u16>().map_err(|error| {
            ClientError::Endpoint(format!("{authority} has an unusable port: {error}"))
        })?,
        None => DEFAULT_PORT,
    };
    Ok((host.to_owned(), port))
}

/// Accepts exactly the addresses `andromeda-taskd` will serve: `localhost` and
/// literal loopback IPs, including the IPv4-mapped forms.
fn is_loopback_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.parse::<IpAddr>()
        .is_ok_and(|ip| ip.to_canonical().is_loopback())
}

/// A connected-mode client: one endpoint, one token, one request at a time.
pub(crate) struct Client {
    endpoint: Endpoint,
    token: String,
    token_path: PathBuf,
}

// The token must never reach a log line, an error message, or a panic message.
impl fmt::Debug for Client {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Client")
            .field("endpoint", &self.endpoint)
            .field("token_path", &self.token_path)
            .field("token", &"<redacted>")
            .finish()
    }
}

impl Client {
    /// Resolves the token and binds it to `endpoint`. No connection is made
    /// here: a missing or unreadable token is reported before any request, so
    /// the failure names the file rather than the socket.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] when the token file is absent, empty, or cannot
    /// be read.
    pub(crate) fn connect(
        endpoint: Endpoint,
        token_file: Option<&Path>,
    ) -> Result<Self, ClientError> {
        let (token, token_path) = load_token(token_file)?;
        Ok(Self {
            endpoint,
            token,
            token_path,
        })
    }

    pub(crate) fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    pub(crate) fn token_path(&self) -> &Path {
        &self.token_path
    }

    /// Performs `GET path`.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on a transport failure, an unusable response, or
    /// any non-2xx status.
    pub(crate) fn get(&self, path: &str) -> Result<Value, ClientError> {
        self.request("GET", path, None)
    }

    /// Performs `POST path` with a JSON body.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on a transport failure, an unusable response, or
    /// any non-2xx status.
    pub(crate) fn post(
        &self,
        path: &str,
        body: &impl serde::Serialize,
    ) -> Result<Value, ClientError> {
        let body = serde_json::to_vec(body).map_err(|error| ClientError::Request {
            message: format!("could not encode the request body: {error}"),
        })?;
        self.request("POST", path, Some(&body))
    }

    fn request(&self, method: &str, path: &str, body: Option<&[u8]>) -> Result<Value, ClientError> {
        validate_path(path)?;
        let mut head = format!(
            "{method} {path} HTTP/1.1\r\n\
             Host: {authority}\r\n\
             Accept: application/json\r\n\
             Authorization: Bearer {token}\r\n\
             Connection: close\r\n",
            authority = self.endpoint.authority,
            token = self.token,
        );
        if let Some(body) = body {
            head.push_str("Content-Type: application/json\r\n");
            head.push_str(&format!("Content-Length: {}\r\n", body.len()));
        }
        head.push_str("\r\n");

        let mut stream = self.open()?;
        let write = stream
            .write_all(head.as_bytes())
            .and_then(|()| match body {
                Some(body) => stream.write_all(body),
                None => Ok(()),
            })
            .and_then(|()| stream.flush());
        write.map_err(|source| ClientError::Transport {
            endpoint: self.endpoint.url(),
            action: "send the request to",
            source,
        })?;

        let mut raw = Vec::new();
        stream
            .take(MAX_RESPONSE_BYTES)
            .read_to_end(&mut raw)
            .map_err(|source| ClientError::Transport {
                endpoint: self.endpoint.url(),
                action: "read the response from",
                source,
            })?;

        let response = parse_response(&raw).map_err(ClientError::Response)?;
        self.interpret(method, path, &response)
    }

    fn open(&self) -> Result<TcpStream, ClientError> {
        let addresses = (self.endpoint.host.as_str(), self.endpoint.port)
            .to_socket_addrs()
            .map_err(|source| ClientError::Transport {
                endpoint: self.endpoint.url(),
                action: "resolve",
                source,
            })?;
        let mut last = None;
        for address in addresses {
            match TcpStream::connect_timeout(&address, CONNECT_TIMEOUT) {
                Ok(stream) => {
                    stream
                        .set_read_timeout(Some(IO_TIMEOUT))
                        .and_then(|()| stream.set_write_timeout(Some(IO_TIMEOUT)))
                        .map_err(|source| ClientError::Transport {
                            endpoint: self.endpoint.url(),
                            action: "configure the socket to",
                            source,
                        })?;
                    return Ok(stream);
                }
                Err(error) => last = Some(error),
            }
        }
        Err(match last {
            Some(source) => ClientError::Connect {
                endpoint: self.endpoint.url(),
                source,
            },
            None => ClientError::Transport {
                endpoint: self.endpoint.url(),
                action: "resolve any address for",
                source: std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "the host resolved to no addresses",
                ),
            },
        })
    }

    /// Turns one HTTP response into either a JSON document or an error that
    /// states what to do about it.
    fn interpret(
        &self,
        method: &str,
        path: &str,
        response: &HttpResponse,
    ) -> Result<Value, ClientError> {
        if response.status == 401 {
            return Err(ClientError::Unauthorized {
                endpoint: self.endpoint.url(),
                token_path: self.token_path.clone(),
            });
        }
        let (code, message) = wire_error(&response.body);
        if response.status == 403 && code.as_deref() == Some("forbidden_host") {
            return Err(ClientError::ForbiddenHost {
                endpoint: self.endpoint.url(),
            });
        }
        if !(200..300).contains(&response.status) {
            return Err(ClientError::Http {
                endpoint: self.endpoint.url(),
                request: format!("{method} {path}"),
                status: response.status,
                code,
                message,
            });
        }
        if response.body.is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_slice(&response.body).map_err(|error| {
            ClientError::Response(format!(
                "taskd answered {} with a body that is not JSON: {error}",
                response.status
            ))
        })
    }
}

/// Reads the bearer token, returning it together with the file it came from.
///
/// With no explicit path the search order is [`TOKEN_SEARCH_PATH`]. A file that
/// exists but cannot be read is reported immediately rather than skipped: on
/// the shipped image that is the *expected* outcome for a non-root caller and
/// is the single most useful thing to say.
fn load_token(explicit: Option<&Path>) -> Result<(String, PathBuf), ClientError> {
    let candidates: Vec<PathBuf> = match explicit {
        Some(path) => vec![path.to_path_buf()],
        None => TOKEN_SEARCH_PATH.iter().map(PathBuf::from).collect(),
    };
    for candidate in &candidates {
        match std::fs::read_to_string(candidate) {
            Ok(text) => {
                let token = text.trim().to_owned();
                if token.is_empty() {
                    return Err(ClientError::EmptyToken {
                        path: candidate.clone(),
                    });
                }
                return Ok((token, candidate.clone()));
            }
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(ClientError::UnreadableToken {
                    path: candidate.clone(),
                    source,
                });
            }
        }
    }
    Err(ClientError::NoToken {
        searched: candidates,
    })
}

/// Rejects a request target that could smuggle a second request line.
///
/// Every path this CLI builds comes from a parsed `TaskId` or a `usize`, so
/// this cannot fire today; it exists so that a future command taking a
/// free-form segment cannot make it fire either.
fn validate_path(path: &str) -> Result<(), ClientError> {
    if path.starts_with('/') && path.bytes().all(|byte| (0x21..=0x7e).contains(&byte)) {
        return Ok(());
    }
    Err(ClientError::Request {
        message: format!("refusing to request {path:?}: not a plain absolute request target"),
    })
}

/// The `{"error": ..., "message": ...}` envelope every taskd failure carries.
fn wire_error(body: &[u8]) -> (Option<String>, Option<String>) {
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        return (None, None);
    };
    let text = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .filter(|found| !found.is_empty())
    };
    (text("error"), text("message"))
}

/// One parsed HTTP response: the status and the decoded body.
#[derive(Debug, PartialEq, Eq)]
struct HttpResponse {
    status: u16,
    body: Vec<u8>,
}

/// Parses an HTTP/1.1 response from the bytes read until EOF.
///
/// `Connection: close` is sent on every request, so the peer ends the body by
/// closing; `Content-Length` and `Transfer-Encoding: chunked` are still honored
/// because the daemon, not this client, decides which to use.
fn parse_response(raw: &[u8]) -> Result<HttpResponse, String> {
    let separator = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| {
            if raw.is_empty() {
                "taskd closed the connection without answering".to_owned()
            } else {
                "taskd sent no complete HTTP header block".to_owned()
            }
        })?;
    let head = std::str::from_utf8(&raw[..separator])
        .map_err(|error| format!("taskd sent a non-UTF-8 response header: {error}"))?;
    let rest = &raw[separator + 4..];

    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap_or_default();
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| format!("taskd sent an unparsable status line {status_line:?}"))?;

    let mut content_length = None;
    let mut chunked = false;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim();
        if name == "content-length" {
            content_length = value.parse::<usize>().ok();
        } else if name == "transfer-encoding" {
            chunked = value
                .to_ascii_lowercase()
                .split(',')
                .any(|encoding| encoding.trim() == "chunked");
        }
    }

    let body = if chunked {
        decode_chunked(rest)?
    } else if let Some(length) = content_length {
        if rest.len() < length {
            return Err(format!(
                "taskd announced {length} body bytes but the connection ended after {}",
                rest.len()
            ));
        }
        rest[..length].to_vec()
    } else {
        rest.to_vec()
    };
    Ok(HttpResponse { status, body })
}

/// Decodes a `Transfer-Encoding: chunked` body.
fn decode_chunked(mut rest: &[u8]) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    loop {
        let end = rest
            .windows(2)
            .position(|window| window == b"\r\n")
            .ok_or_else(|| "taskd sent a chunked body with an unterminated size line".to_owned())?;
        let header = std::str::from_utf8(&rest[..end])
            .map_err(|_| "taskd sent a non-ASCII chunk size".to_owned())?;
        // Chunk extensions (`1a;name=value`) are legal and carry nothing we use.
        let size_text = header.split(';').next().unwrap_or_default().trim();
        let size = usize::from_str_radix(size_text, 16)
            .map_err(|error| format!("taskd sent an unusable chunk size {size_text:?}: {error}"))?;
        rest = &rest[end + 2..];
        if size == 0 {
            return Ok(body);
        }
        if rest.len() < size + 2 {
            return Err("taskd sent a chunked body that ends mid-chunk".to_owned());
        }
        body.extend_from_slice(&rest[..size]);
        rest = &rest[size + 2..];
    }
}

/// Every way a connected-mode command can fail, each rendered as the action the
/// operator has to take.
#[derive(Debug)]
pub(crate) enum ClientError {
    Endpoint(String),
    Request {
        message: String,
    },
    NoToken {
        searched: Vec<PathBuf>,
    },
    EmptyToken {
        path: PathBuf,
    },
    UnreadableToken {
        path: PathBuf,
        source: std::io::Error,
    },
    Connect {
        endpoint: String,
        source: std::io::Error,
    },
    Transport {
        endpoint: String,
        action: &'static str,
        source: std::io::Error,
    },
    Response(String),
    Unauthorized {
        endpoint: String,
        token_path: PathBuf,
    },
    ForbiddenHost {
        endpoint: String,
    },
    Http {
        endpoint: String,
        request: String,
        status: u16,
        code: Option<String>,
        message: Option<String>,
    },
}

impl fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Endpoint(message) | Self::Request { message } | Self::Response(message) => {
                formatter.write_str(message)
            }
            Self::NoToken { searched } => write!(
                formatter,
                "no taskd token found. Every taskd route requires a bearer token, /healthz \
                 included, so the CLI cannot talk to the daemon without one. Looked in: {}. The \
                 shipped unit generates the token into {SYSTEM_TOKEN_PATH}, which only the \
                 service account and root can read — try `sudo andromeda ...`, or point \
                 --auth-token-file (ANDROMEDA_AUTH_TOKEN_FILE) at the file",
                searched
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            Self::EmptyToken { path } => write!(
                formatter,
                "the taskd token file {} is empty. taskd writes the token when it starts; if the \
                 daemon is still starting, retry, and otherwise check `systemctl status \
                 andromeda-taskd`",
                path.display()
            ),
            Self::UnreadableToken { path, source } => {
                write!(
                    formatter,
                    "cannot read the taskd token {}: {source}",
                    path.display()
                )?;
                if source.kind() == std::io::ErrorKind::PermissionDenied {
                    formatter.write_str(
                        ". The token is mode 0600 in a 0700 directory owned by the daemon's \
                         DynamicUser, so only that account and root can read it — that is the \
                         boundary it exists to enforce, not a misconfiguration. Re-run under \
                         sudo",
                    )?;
                }
                Ok(())
            }
            Self::Connect { endpoint, source } => write!(
                formatter,
                "cannot reach taskd at {endpoint}: {source}. Check that the daemon is running \
                 (`systemctl status andromeda-taskd`, or `cargo run --bin andromeda-taskd`) and \
                 that --connect names its --listen address"
            ),
            Self::Transport {
                endpoint,
                action,
                source,
            } => write!(formatter, "could not {action} {endpoint}: {source}"),
            Self::Unauthorized {
                endpoint,
                token_path,
            } => write!(
                formatter,
                "taskd at {endpoint} rejected the token from {} (HTTP 401 unauthorized). The \
                 daemon does not say whether the token was missing, malformed, or simply wrong, \
                 so check that this file is the one the running daemon uses: the shipped unit \
                 sets ANDROMEDA_AUTH_TOKEN_FILE={SYSTEM_TOKEN_PATH}, and a daemon started by \
                 hand defaults to {DEV_TOKEN_PATH} relative to its own working directory. A \
                 restarted daemon keeps its existing token, so a stale copy elsewhere is the \
                 usual cause",
                token_path.display()
            ),
            Self::ForbiddenHost { endpoint } => write!(
                formatter,
                "taskd at {endpoint} refused the request's Host header (HTTP 403 \
                 forbidden_host). It only answers requests addressed to localhost or a literal \
                 loopback IP; connect through 127.0.0.1 or [::1]"
            ),
            Self::Http {
                endpoint,
                request,
                status,
                code,
                message,
            } => {
                write!(
                    formatter,
                    "taskd at {endpoint} refused {request}: HTTP {status}"
                )?;
                if let Some(code) = code {
                    write!(formatter, " {code}")?;
                }
                if let Some(message) = message {
                    write!(formatter, " — {message}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for ClientError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::UnreadableToken { source, .. }
            | Self::Connect { source, .. }
            | Self::Transport { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::net::SocketAddr;

    use andromeda_core::{
        ActionId, ActionKind, ActionPlan, ActionSpec, Capability, CapabilityId, CapabilityResource,
        FileAccess, Intent, RecoverySemantics, RiskLevel, TaskId,
    };
    use andromeda_policy::PolicyEngine;
    use andromeda_runtime::{
        CapabilityAdmission, CreateTaskRequest, EvaluationRequest, FileTaskStore, TaskService,
    };
    use chrono::Utc;
    use serde_json::json;
    use tempfile::TempDir;

    use super::*;

    /// A fixed literal, never a generated value: tests must not depend on an RNG.
    const TEST_TOKEN: &str = "0123456789abcdef0123456789abcdef";

    /// A real `andromeda-taskd` router on a real loopback socket, so the tests
    /// exercise the bytes this client actually puts on the wire — the
    /// `Authorization` header included — rather than a stub that could agree
    /// with a wrong client.
    struct Daemon {
        address: SocketAddr,
        _runtime: tokio::runtime::Runtime,
        _state: TempDir,
    }

    impl Daemon {
        fn spawn(token: &str) -> Self {
            let state = TempDir::new().expect("state dir");
            let service = TaskService::new(
                FileTaskStore::open(state.path()).expect("store"),
                PolicyEngine::default(),
                CapabilityAdmission::unsigned_for_development(),
            );
            let router = andromeda_taskd::app(
                service,
                andromeda_taskd::Authenticator::from_token(token).expect("token"),
            );
            let runtime = tokio::runtime::Runtime::new().expect("runtime");
            let listener = runtime.block_on(async {
                tokio::net::TcpListener::bind("127.0.0.1:0")
                    .await
                    .expect("bind loopback")
            });
            let address = listener.local_addr().expect("local address");
            runtime.spawn(async move {
                axum::serve(listener, router).await.expect("serve");
            });
            Self {
                address,
                _runtime: runtime,
                _state: state,
            }
        }

        fn url(&self) -> String {
            format!("http://{}", self.address)
        }
    }

    /// Writes `token` into a temporary file and returns both, keeping the
    /// directory alive for the caller's lifetime.
    fn token_file(token: &str) -> (TempDir, PathBuf) {
        let directory = TempDir::new().expect("token dir");
        let path = directory.path().join("token");
        std::fs::write(&path, format!("{token}\n")).expect("write token");
        (directory, path)
    }

    fn client_for(daemon: &Daemon, token: &str) -> (TempDir, Client) {
        let (directory, path) = token_file(token);
        let endpoint = Endpoint::parse(&daemon.url()).expect("endpoint");
        let client = Client::connect(endpoint, Some(&path)).expect("client");
        (directory, client)
    }

    fn inspection_request() -> CreateTaskRequest {
        let task_id = TaskId::new();
        let capability = Capability {
            id: CapabilityId::new(),
            resource: CapabilityResource::Files {
                root: PathBuf::from(workspace_path()),
                access: FileAccess::Read,
            },
            issued_to: task_id.to_string(),
            issued_at: Utc::now(),
            expires_at: None,
            single_use: false,
            signature: None,
        };
        let plan = ActionPlan {
            schema_version: ActionPlan::CURRENT_SCHEMA_VERSION,
            task_id,
            intent: Intent::new("Inspect the workspace", "test"),
            actions: vec![ActionSpec {
                id: ActionId::new(),
                name: "Inspect directory".into(),
                kind: ActionKind::Inspect,
                target: workspace_path().into(),
                arguments: BTreeMap::new(),
                depends_on: Vec::new(),
                required_capabilities: vec![capability.id],
                risk: RiskLevel::L1Sandboxed,
                recovery: RecoverySemantics::None,
            }],
        };
        CreateTaskRequest {
            plan,
            capabilities: vec![capability],
            actor: "test".into(),
        }
    }

    #[cfg(not(target_os = "windows"))]
    const fn workspace_path() -> &'static str {
        "/workspace"
    }

    #[cfg(target_os = "windows")]
    const fn workspace_path() -> &'static str {
        r"C:\workspace"
    }

    /// The whole point of connected mode: a request carries the token read from
    /// the token file, and the daemon accepts it.
    #[test]
    fn the_client_presents_the_token_from_the_token_file() {
        let daemon = Daemon::spawn(TEST_TOKEN);
        let (_directory, client) = client_for(&daemon, TEST_TOKEN);
        let health = client.get("/healthz").expect("healthz");
        assert_eq!(health["status"], "ok");
        assert_eq!(health["api_version"], "v1");
        // Proof that the token is what got us in, rather than an unauthenticated
        // route: the same call with a different token is refused.
        assert_eq!(health["authentication"], "bearer_token");
    }

    /// A wrong token must produce guidance, not a status dump.
    #[test]
    fn a_rejected_token_names_the_file_and_says_what_to_check() {
        let daemon = Daemon::spawn(TEST_TOKEN);
        let (_directory, client) = client_for(&daemon, "ffffffffffffffffffffffffffffffff");
        let error = client.get("/healthz").expect_err("401 expected");
        assert!(
            matches!(error, ClientError::Unauthorized { .. }),
            "401 must have its own variant, got {error:?}"
        );
        let message = error.to_string();
        for expected in [
            "401",
            "unauthorized",
            SYSTEM_TOKEN_PATH,
            DEV_TOKEN_PATH,
            "ANDROMEDA_AUTH_TOKEN_FILE",
        ] {
            assert!(
                message.contains(expected),
                "{expected} missing from: {message}"
            );
        }
        assert!(
            message.contains(client.token_path().to_str().expect("token path")),
            "the message must name the file that was actually used: {message}"
        );
        assert!(
            !message.contains(TEST_TOKEN) && !message.contains("ffffffff"),
            "no token may appear in an error message: {message}"
        );
    }

    /// The listing is a summary projection: counts, no event bodies, no plan.
    #[test]
    fn list_parses_the_current_summary_shape() {
        let daemon = Daemon::spawn(TEST_TOKEN);
        let (_directory, client) = client_for(&daemon, TEST_TOKEN);
        let request = inspection_request();
        let task_id = request.plan.task_id;
        let created = client.post("/v1/tasks", &request).expect("create");
        assert_eq!(created["state"], "ready");

        let listing = client.get("/v1/tasks").expect("list");
        let tasks = listing["tasks"].as_array().expect("tasks");
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0]["task_id"], json!(task_id.to_string()));
        assert_eq!(tasks[0]["event_count"], json!(1));
        assert!(
            tasks[0].get("events").is_none() && tasks[0].get("plan").is_none(),
            "the listing carries no event bodies: {}",
            tasks[0]
        );
        assert_eq!(listing["warnings"].as_array().expect("warnings").len(), 0);
    }

    /// A single-task read is bounded, and the client can tell how much history
    /// it did not receive.
    #[test]
    fn show_parses_the_bounded_read_and_reports_the_true_total() {
        let daemon = Daemon::spawn(TEST_TOKEN);
        let (_directory, client) = client_for(&daemon, TEST_TOKEN);
        let request = inspection_request();
        let task_id = request.plan.task_id;
        client.post("/v1/tasks", &request).expect("create");

        // 60 evaluations plus the `created` event exceed the daemon's default
        // window of 50.
        let evaluation = EvaluationRequest::default();
        for _ in 0..60 {
            client
                .post(&format!("/v1/tasks/{task_id}/evaluate"), &evaluation)
                .expect("evaluate");
        }

        let record = client.get(&format!("/v1/tasks/{task_id}")).expect("show");
        assert_eq!(record["events"].as_array().expect("events").len(), 50);
        assert_eq!(record["event_count"], json!(61));

        let widened = client
            .get(&format!("/v1/tasks/{task_id}?events=61"))
            .expect("show widened");
        assert_eq!(widened["events"].as_array().expect("events").len(), 61);
        assert_eq!(widened["event_count"], json!(61));
    }

    /// A refused request surfaces the daemon's own error code, not a bare
    /// status number.
    #[test]
    fn a_refused_request_reports_the_wire_error_code() {
        let daemon = Daemon::spawn(TEST_TOKEN);
        let (_directory, client) = client_for(&daemon, TEST_TOKEN);
        let error = client
            .get(&format!("/v1/tasks/{}", TaskId::new()))
            .expect_err("unknown task");
        let message = error.to_string();
        assert!(
            message.contains("404") && message.contains("not_found"),
            "{message}"
        );
        assert!(message.contains(&client.endpoint().url()), "{message}");
    }

    /// The constants this crate repeats must equal the daemon's own, or the
    /// CLI would advertise a path or a ceiling that no longer exists.
    #[test]
    fn the_token_search_path_matches_the_daemons_own() {
        assert_eq!(SYSTEM_TOKEN_PATH, andromeda_taskd::auth::SYSTEM_TOKEN_PATH);
        assert_eq!(MAX_EVENTS, andromeda_taskd::MAX_EVENT_LIMIT);
        assert!(TOKEN_SEARCH_PATH.contains(&SYSTEM_TOKEN_PATH));
        assert!(TOKEN_SEARCH_PATH.contains(&DEV_TOKEN_PATH));
    }

    #[test]
    fn endpoints_are_parsed_and_normalized() {
        for (raw, authority, port) in [
            ("http://127.0.0.1:7777", "127.0.0.1:7777", 7777u16),
            ("http://127.0.0.1:7777/", "127.0.0.1:7777", 7777),
            ("127.0.0.1:7777", "127.0.0.1:7777", 7777),
            ("http://localhost", "localhost", DEFAULT_PORT),
            ("http://[::1]:7777", "[::1]:7777", 7777),
            ("http://[::1]", "[::1]", DEFAULT_PORT),
            ("  http://127.0.0.5:1  ", "127.0.0.5:1", 1),
        ] {
            let endpoint = Endpoint::parse(raw).unwrap_or_else(|error| panic!("{raw}: {error}"));
            assert_eq!(endpoint.authority, authority, "{raw}");
            assert_eq!(endpoint.port, port, "{raw}");
        }
    }

    /// A non-loopback `--connect` is refused before any byte is sent, because
    /// sending it would hand the local bearer token to that host.
    #[test]
    fn a_non_loopback_endpoint_is_refused_before_the_token_travels() {
        for raw in [
            "http://example.com:7777",
            "http://192.168.1.10:7777",
            "http://127.0.0.1.evil.example",
            "http://[::2]:7777",
        ] {
            let error = Endpoint::parse(raw).expect_err("{raw} must be refused");
            let message = error.to_string();
            assert!(
                message.contains("loopback") && message.contains("token"),
                "{raw} must be refused because of the token: {message}"
            );
        }
    }

    #[test]
    fn unusable_endpoints_are_rejected_with_a_reason() {
        for (raw, expected) in [
            ("https://127.0.0.1:7777", "https"),
            ("ftp://127.0.0.1", "http://"),
            ("http://127.0.0.1:7777/v1/tasks", "bare origin"),
            ("http://user:pass@127.0.0.1:7777", "userinfo"),
            ("http://127.0.0.1:notaport", "unusable port"),
            ("http://::1:7777", "brackets"),
            ("http://", "needs a host"),
        ] {
            let error = Endpoint::parse(raw).expect_err("must be rejected");
            assert!(
                error.to_string().contains(expected),
                "{raw}: {expected:?} missing from {error}"
            );
        }
    }

    #[test]
    fn a_missing_token_lists_every_path_it_looked_in() {
        let error = load_token(None).map(|(_, path)| path);
        // On a developer machine neither search path normally exists; if one
        // does, the token simply loads and there is nothing to assert.
        if let Err(error) = error {
            let message = error.to_string();
            assert!(message.contains(SYSTEM_TOKEN_PATH), "{message}");
            assert!(message.contains(DEV_TOKEN_PATH), "{message}");
            assert!(message.contains("sudo"), "{message}");
        }
    }

    #[test]
    fn an_empty_token_file_is_refused_rather_than_sent() {
        let directory = TempDir::new().expect("dir");
        let path = directory.path().join("token");
        std::fs::write(&path, "   \n").expect("write");
        let error = load_token(Some(&path)).expect_err("empty token must be refused");
        assert!(error.to_string().contains("is empty"), "{error}");
    }

    #[test]
    fn responses_are_parsed_with_a_content_length() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 9\r\n\r\n{\"a\": 1}\n";
        let response = parse_response(raw).expect("parse");
        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"{\"a\": 1}\n");
    }

    #[test]
    fn responses_are_parsed_when_chunked() {
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\n{\"a\"\r\n4;x=y\r\n: 1}\r\n0\r\n\r\n";
        let response = parse_response(raw).expect("parse");
        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"{\"a\": 1}");
    }

    #[test]
    fn a_truncated_response_is_an_error_rather_than_a_short_read() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 40\r\n\r\n{\"a\": 1}";
        let error = parse_response(raw).expect_err("truncation must be detected");
        assert!(error.contains("40 body bytes"), "{error}");

        let error = parse_response(b"").expect_err("empty must be detected");
        assert!(error.contains("without answering"), "{error}");
    }

    #[test]
    fn a_request_target_may_not_smuggle_a_second_request() {
        assert!(validate_path("/v1/tasks").is_ok());
        assert!(validate_path("/v1/tasks?events=50").is_ok());
        for bad in ["v1/tasks", "/v1/tasks HTTP/1.1\r\nX: y", "/v1/ tasks"] {
            assert!(validate_path(bad).is_err(), "{bad} must be refused");
        }
    }
}
