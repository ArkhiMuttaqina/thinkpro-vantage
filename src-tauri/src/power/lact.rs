//! Talking to LACT's daemon.
//!
//! LACT (`lactd`) manages the same amdgpu nodes this app writes. Where it runs,
//! it re-applies the performance level from `/etc/lact/config.yaml` whenever it
//! applies settings, so a clock ceiling written straight to sysfs only holds
//! until its next tick. On the machine this was written for, that config says
//! `low`, which pins the iGPU to its lowest DPM level.
//!
//! The only way a level written here survives is for lactd to stop managing the
//! device, which its own API calls `detach_gpu`; `reattach_gpu` gives it back.
//! Both go over `/run/lactd.sock`, which is not HTTP: one JSON line out, one
//! JSON line in, internally tagged `{"command": ..., "args": {...}}`, with
//! failures reported in-band as `{"status": "error", "data": ...}` rather than
//! as a transport error.
//!
//! A hand-rolled client is the whole protocol: one `UnixStream`, one encode and
//! one decode. The published `lact-client` crate would pull lactd's own
//! dependency tree into this binary for the same three calls.
//!
//! Nothing here panics, and nothing here breaks the app when lactd is missing:
//! [`Client::detect`] answers whether the socket is there, and every failure is
//! a value the caller decides what to do with.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Where `lactd` listens. The socket's presence is what "LACT is running" means
/// here; the daemon and its version are never looked up separately.
pub const SOCKET_PATH: &str = "/run/lactd.sock";

/// How long one request may take before the daemon counts as unresponsive.
///
/// Each of these is a local socket round trip, so anything slower is a daemon in
/// trouble, and waiting longer would only stall the caller.
const TIMEOUT: Duration = Duration::from_secs(5);

/// A request lactd understands.
///
/// The serde attributes are lactd's framing, not a choice: the variant name
/// becomes `command` and the fields become `args`.
#[derive(Debug, Serialize)]
#[serde(tag = "command", content = "args", rename_all = "snake_case")]
pub enum Request {
    /// The devices lactd currently manages. A detached GPU is absent from it.
    ListDevices,
    /// Stop managing a device, so its knobs can be written elsewhere.
    DetachGpu { id: String },
    /// Manage a device again.
    ReattachGpu { id: String },
}

/// Why a request did not come back with the answer the caller asked for.
#[derive(Debug)]
pub enum Error {
    /// Nothing answered: no socket, nothing listening behind it, or the daemon
    /// hung up mid-request. lactd is not writing anything, so a caller may carry
    /// on without it — deliberately not the same answer as a refusal.
    Unreachable(String),
    /// lactd answered, and the answer was no.
    Refused(String),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Unreachable(reason) => write!(formatter, "lactd did not answer ({reason})"),
            Error::Refused(reason) => write!(formatter, "lactd refused: {reason}"),
        }
    }
}

/// A `lactd` that is running, addressed by its socket.
#[derive(Debug, Clone)]
pub struct Client {
    socket: PathBuf,
}

/// One JSON line from lactd. `data` is whatever the command returned, including
/// the payload of an in-band error.
#[derive(Debug, Deserialize)]
struct Response {
    status: String,
    #[serde(default)]
    data: serde_json::Value,
}

/// The part of a device entry this client needs. lactd sends more (name, device
/// type); it is ignored rather than modelled, because nothing here decides
/// anything on it.
#[derive(Debug, Deserialize)]
struct Device {
    id: String,
}

impl Client {
    /// lactd, when its socket is there.
    ///
    /// `None` is the ordinary "LACT is not installed, or not running" case, and
    /// every caller treats it as "no competing writer" rather than as an error.
    pub fn detect() -> Option<Self> {
        Self::at(SOCKET_PATH)
    }

    /// lactd at a specific socket path.
    ///
    /// Only tests pass anything but the default: the daemon's own path is not a
    /// setting, and inventing one would let the app point at a socket that is
    /// not the daemon it claims to be.
    pub fn at(path: impl AsRef<Path>) -> Option<Self> {
        let path = path.as_ref();
        path.exists().then(|| Self {
            socket: path.to_path_buf(),
        })
    }

    /// The ids of the devices lactd is managing, in its own order.
    pub fn gpu_ids(&self) -> Result<Vec<String>, Error> {
        let data = self.call(&Request::ListDevices)?;
        let devices: Vec<Device> = serde_json::from_value(data).map_err(|error| {
            Error::Refused(format!("could not read the device list it sent ({error})"))
        })?;
        Ok(devices.into_iter().map(|device| device.id).collect())
    }

    /// Stop lactd managing `id` until [`Client::reattach`] is called.
    pub fn detach(&self, id: &str) -> Result<(), Error> {
        self.call(&Request::DetachGpu { id: id.to_string() })
            .map(|_| ())
    }

    /// Manage `id` again.
    pub fn reattach(&self, id: &str) -> Result<(), Error> {
        self.call(&Request::ReattachGpu { id: id.to_string() })
            .map(|_| ())
    }

    /// One request, one answer.
    fn call(&self, request: &Request) -> Result<serde_json::Value, Error> {
        let request = serde_json::to_string(request)
            .map_err(|error| Error::Refused(format!("could not encode the request ({error})")))?;

        let stream = UnixStream::connect(&self.socket)
            .map_err(|error| Error::Unreachable(format!("{}: {error}", self.socket.display())))?;
        // A daemon that accepts the connection and then goes quiet must not hang
        // the caller. The write side gets the same guard: a full socket buffer
        // would otherwise block forever.
        let _ = stream.set_read_timeout(Some(TIMEOUT));
        let _ = stream.set_write_timeout(Some(TIMEOUT));

        let mut writer = &stream;
        writer
            .write_all(request.as_bytes())
            .and_then(|()| writer.write_all(b"\n"))
            .and_then(|()| writer.flush())
            .map_err(|error| Error::Unreachable(format!("could not send the request: {error}")))?;

        let mut answer = String::new();
        BufReader::new(&stream)
            .read_line(&mut answer)
            .map_err(|error| Error::Unreachable(format!("could not read the answer: {error}")))?;

        let answer = answer.trim_end();
        if answer.is_empty() {
            return Err(Error::Unreachable(
                "lactd closed the connection without answering".to_string(),
            ));
        }

        let response: Response = serde_json::from_str(answer).map_err(|error| {
            Error::Refused(format!("could not read its answer ({error}): {answer}"))
        })?;

        if response.status == "ok" {
            return Ok(response.data);
        }
        Err(Error::Refused(reason(&response.data)))
    }
}

/// What an in-band error said, in one line.
fn reason(data: &serde_json::Value) -> String {
    match data {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Null => "no reason given".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;
    use std::thread::JoinHandle;

    /// A test socket that deletes its file when the test ends, so the next run
    /// does not find a stale one and read the wrong verdict from it.
    struct Socket(PathBuf);

    impl Socket {
        fn bind(name: &str) -> (Socket, UnixListener) {
            let path = std::env::temp_dir().join(format!(
                "thinkpro-vantage_lact_{}_{}.sock",
                std::process::id(),
                name
            ));
            let _ = std::fs::remove_file(&path);
            let listener = UnixListener::bind(&path).expect("bind the test socket");
            (Socket(path), listener)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Socket {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    /// A lactd stand-in: accepts one connection per canned reply, answers it,
    /// and hands back every request line it received so a test can check the
    /// framing as well as the result.
    fn fake_lactd(replies: &[&'static str]) -> (Socket, JoinHandle<Vec<String>>) {
        let (socket, listener) = Socket::bind(&format!(
            "fake_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|age| age.subsec_nanos())
                .unwrap_or(0)
        ));

        let replies = replies.to_vec();
        let server = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for reply in replies {
                let Ok((stream, _)) = listener.accept() else {
                    break;
                };
                let mut line = String::new();
                let _ = BufReader::new(&stream).read_line(&mut line);
                seen.push(line.trim_end().to_string());
                let _ = (&stream).write_all(reply.as_bytes());
                let _ = (&stream).write_all(b"\n");
            }
            seen
        });

        (socket, server)
    }

    #[test]
    fn the_device_list_is_read_from_lactds_own_answer() {
        let (socket, server) = fake_lactd(&[
            r#"{"status":"ok","data":[{"id":"1002:1638-17AA:5095-0000:05:00.0","name":"Cezanne","device_type":"Integrated"}]}"#,
        ]);
        let client = Client::at(socket.path()).expect("the socket is there");

        assert_eq!(
            client.gpu_ids().expect("the list parses"),
            vec!["1002:1638-17AA:5095-0000:05:00.0"]
        );
        assert_eq!(
            server.join().unwrap(),
            vec![r#"{"command":"list_devices"}"#]
        );
    }

    #[test]
    fn a_detached_gpu_is_simply_absent_from_the_list() {
        let (socket, server) = fake_lactd(&[r#"{"status":"ok","data":[]}"#]);
        let client = Client::at(socket.path()).expect("the socket is there");

        assert!(client.gpu_ids().expect("an empty list parses").is_empty());
        server.join().unwrap();
    }

    #[test]
    fn detach_sends_the_id_in_lactds_own_framing() {
        let (socket, server) = fake_lactd(&[r#"{"status":"ok","data":null}"#]);
        let client = Client::at(socket.path()).expect("the socket is there");

        client
            .detach("1002:1638-17AA:5095-0000:05:00.0")
            .expect("detach is accepted");

        assert_eq!(
            server.join().unwrap(),
            vec![r#"{"command":"detach_gpu","args":{"id":"1002:1638-17AA:5095-0000:05:00.0"}}"#]
        );
    }

    #[test]
    fn reattach_is_the_same_shape_as_detach() {
        let (socket, server) = fake_lactd(&[r#"{"status":"ok","data":null}"#]);
        let client = Client::at(socket.path()).expect("the socket is there");

        client
            .reattach("1002:1638-17AA:5095-0000:05:00.0")
            .expect("reattach is accepted");

        assert_eq!(
            server.join().unwrap(),
            vec![r#"{"command":"reattach_gpu","args":{"id":"1002:1638-17AA:5095-0000:05:00.0"}}"#]
        );
    }

    /// The distinction the caller depends on: a daemon that says no is not the
    /// same as a daemon that is not there, and only one of them stops a write.
    #[test]
    fn an_in_band_error_is_a_refusal_not_a_success() {
        let (socket, server) =
            fake_lactd(&[r#"{"status":"error","data":"device is currently detached"}"#]);
        let client = Client::at(socket.path()).expect("the socket is there");

        match client.reattach("1002:1638-17AA:5095-0000:05:00.0") {
            Err(Error::Refused(reason)) => assert_eq!(reason, "device is currently detached"),
            other => panic!("expected a refusal, got {other:?}"),
        }
        server.join().unwrap();
    }

    #[test]
    fn a_missing_socket_is_the_lact_is_not_here_case() {
        assert!(Client::at("/run/thinkpro-vantage_no-such-lactd.sock").is_none());
    }

    /// A socket file outlives a daemon that crashed. Connecting to it fails, and
    /// that must read as "nothing is listening", not as "LACT said no" — the
    /// caller writes the cap in the first case and only in the first case.
    #[test]
    fn a_stale_socket_is_unreachable_rather_than_a_refusal() {
        let (socket, listener) = Socket::bind("stale");
        drop(listener);
        assert!(socket.path().exists(), "the file survives the daemon");

        let client = Client::at(socket.path()).expect("the file is there");
        match client.gpu_ids() {
            Err(Error::Unreachable(_)) => {}
            other => panic!("expected an unreachable daemon, got {other:?}"),
        }
    }

    /// A daemon that accepts and then hangs up has answered nothing. Treating
    /// the empty read as an error keeps the caller from acting on a reply that
    /// was never sent.
    #[test]
    fn a_daemon_that_hangs_up_is_unreachable() {
        let (socket, server) = fake_lactd(&[""]);
        let client = Client::at(socket.path()).expect("the socket is there");

        match client.gpu_ids() {
            Err(Error::Unreachable(_)) => {}
            other => panic!("expected an unreachable daemon, got {other:?}"),
        }
        server.join().unwrap();
    }

    #[test]
    fn a_reply_that_is_not_json_is_a_refusal_that_says_so() {
        let (socket, server) = fake_lactd(&["not json at all"]);
        let client = Client::at(socket.path()).expect("the socket is there");

        match client.gpu_ids() {
            Err(Error::Refused(reason)) => assert!(reason.contains("not json at all")),
            other => panic!("expected a refusal, got {other:?}"),
        }
        server.join().unwrap();
    }
}
