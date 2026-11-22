//! Protocol v3 isolated VST3 sidecar host controller (`dawsu_vst3_host`).
//!
//! Mirrors `native/vst3-host/host.cpp` (Magic `"DVH3"`, Protocol 3).

use serde::{Deserialize, Serialize};
use std::fmt;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

/// Shared-memory magic `"DVH3"` (little-endian).
pub const PROTOCOL_MAGIC: u32 = 0x3348_5644;
/// Sidecar protocol version 3 (instruments + Fx, stereo audio input bus).
pub const PROTOCOL_VERSION: u32 = 3;
/// Maximum MIDI events per shared-memory audio block.
pub const MAX_EVENTS: usize = 1024;
/// Maximum stereo frames per shared-memory audio block.
pub const MAX_FRAMES: usize = 4096;

/// Single MIDI-style event inside the shared-memory audio block.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ShmEvent {
    pub offset: i32,
    pub status: u8,
    pub data1: u8,
    pub data2: u8,
    pub pad: u8,
}

/// Shared-memory audio exchange layout (must match `Shm` in `native/vst3-host/host.cpp`).
#[repr(C)]
pub struct ShmBlock {
    pub magic: u32,
    pub version: u32,
    pub frames: i32,
    pub event_count: i32,
    pub request_seq: u32,
    pub done_seq: u32,
    /// `0` when rendered, `1` when not rendered (silence).
    pub status: i32,
    /// `1` when `in_left`/`in_right` carry stereo audio input (for Fx), `0` otherwise.
    pub has_audio_in: i32,
    pub sample_rate: f64,
    pub events: [ShmEvent; MAX_EVENTS],
    pub in_left: [f32; MAX_FRAMES],
    pub in_right: [f32; MAX_FRAMES],
    pub left: [f32; MAX_FRAMES],
    pub right: [f32; MAX_FRAMES],
}

/// Launch parameters for `dawsu_vst3_host --host ...`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vst3HostConfig {
    pub host_executable: PathBuf,
    pub shm_name: String,
    pub parent_pid: u32,
    pub plugin_path: PathBuf,
    pub plugin_uid: Option<String>,
    pub offline: bool,
}

impl Vst3HostConfig {
    /// Builds the CLI argument list passed to `dawsu_vst3_host`.
    pub fn args(&self) -> Vec<String> {
        let mut args = vec![
            "--host".to_string(),
            self.shm_name.clone(),
            self.parent_pid.to_string(),
            self.plugin_path.to_string_lossy().into_owned(),
        ];
        if let Some(uid) = &self.plugin_uid {
            if !uid.is_empty() {
                args.push(uid.clone());
            }
        }
        if self.offline {
            args.push("--offline".to_string());
        }
        args
    }
}

/// A timed MIDI message for offline `RENDER` commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MidiEvent {
    pub frame: u64,
    pub status: u8,
    pub data1: u8,
    pub data2: u8,
}

/// Parameters for an offline `RENDER` command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderRequest {
    pub out_path: PathBuf,
    pub max_frames: u64,
    pub quiet_frames: u64,
    pub threshold: f64,
    pub events: Vec<MidiEvent>,
}

/// Result returned by a successful offline `RENDER` command.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RenderResult {
    pub frames: u64,
    pub truncated: bool,
    pub peak: f32,
}

/// Control commands sent over stdin to the VST3 sidecar host.
#[derive(Debug, Clone, PartialEq)]
pub enum HostCommand {
    Show,
    Hide,
    GetState,
    SetState(String),
    NoteNames,
    Render(RenderRequest),
    Quit,
}

impl HostCommand {
    /// Formats a tagged protocol line (`"<id>\t<command>"`, without trailing newline).
    pub fn format_tagged(&self, id: u64) -> String {
        match self {
            Self::Show => format!("{id}\tSHOW"),
            Self::Hide => format!("{id}\tHIDE"),
            Self::GetState => format!("{id}\tGETSTATE"),
            Self::SetState(b64) => format!("{id}\tSETSTATE\t{}", sanitize_field(b64)),
            Self::NoteNames => format!("{id}\tNOTENAMES"),
            Self::Render(req) => {
                let events: Vec<String> = req
                    .events
                    .iter()
                    .map(|e| format!("{}:{}:{}:{}", e.frame, e.status, e.data1, e.data2))
                    .collect();
                format!(
                    "{id}\tRENDER\t{}\t{}\t{}\t{}\t{}",
                    sanitize_field(&req.out_path.to_string_lossy()),
                    req.max_frames,
                    req.quiet_frames,
                    req.threshold,
                    events.join(";")
                )
            }
            Self::Quit => format!("{id}\tQUIT"),
        }
    }
}

fn sanitize_field(value: &str) -> String {
    value
        .chars()
        .map(|c| match c {
            '\t' | '\n' | '\r' => ' ',
            other => other,
        })
        .collect()
}

/// Parsed reply from the VST3 sidecar host.
#[derive(Debug, Clone, PartialEq)]
pub enum HostReply {
    Ok,
    State(String),
    NoteNames(Vec<(u8, String)>),
    Rendered(RenderResult),
    Error(String),
}

/// Errors from spawning, talking to, or monitoring the sidecar host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostError {
    SpawnFailed(String),
    Io(String),
    ProtocolMismatch { expected: u32, actual: u32 },
    PluginError(String),
    Crashed(String),
    InvalidReply(String),
    Closed,
}

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SpawnFailed(msg) => write!(f, "failed to start VST3 host: {msg}"),
            Self::Io(msg) => write!(f, "VST3 host I/O error: {msg}"),
            Self::ProtocolMismatch { expected, actual } => {
                write!(f, "VST3 host protocol mismatch: expected {expected}, got {actual}")
            }
            Self::PluginError(msg) => write!(f, "VST3 plug-in error: {msg}"),
            Self::Crashed(msg) => write!(f, "VST3 host crashed: {msg}"),
            Self::InvalidReply(msg) => write!(f, "invalid reply from VST3 host: {msg}"),
            Self::Closed => write!(f, "VST3 host session is already closed"),
        }
    }
}

impl std::error::Error for HostError {}

/// Line-oriented transport abstraction so `HostSession` can be unit-tested without a real `.exe`.
pub trait HostTransport {
    fn write_line(&mut self, line: &str) -> Result<(), HostError>;
    /// Reads the next line (without trailing `\r`/`\n`), or `Ok(None)` on EOF.
    fn read_line(&mut self) -> Result<Option<String>, HostError>;
    fn is_alive(&mut self) -> bool;
    fn kill(&mut self);
}

/// Real child-process transport backed by `dawsu_vst3_host`'s stdin/stdout pipes.
pub struct ProcessTransport {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl ProcessTransport {
    pub fn spawn(config: &Vst3HostConfig) -> Result<Self, HostError> {
        let mut child = Command::new(&config.host_executable)
            .args(config.args())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|err| HostError::SpawnFailed(err.to_string()))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| HostError::SpawnFailed("child stdin pipe unavailable".to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| HostError::SpawnFailed("child stdout pipe unavailable".to_string()))?;

        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
        })
    }
}

impl HostTransport for ProcessTransport {
    fn write_line(&mut self, line: &str) -> Result<(), HostError> {
        self.stdin
            .write_all(line.as_bytes())
            .and_then(|()| self.stdin.write_all(b"\n"))
            .and_then(|()| self.stdin.flush())
            .map_err(|err| HostError::Io(err.to_string()))
    }

    fn read_line(&mut self) -> Result<Option<String>, HostError> {
        let mut buf = String::new();
        let bytes = self
            .stdout
            .read_line(&mut buf)
            .map_err(|err| HostError::Io(err.to_string()))?;
        if bytes == 0 {
            return Ok(None);
        }
        Ok(Some(buf.trim_end_matches(['\r', '\n']).to_string()))
    }

    fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Active session talking to one isolated VST3 sidecar host process.
pub struct HostSession<T: HostTransport = ProcessTransport> {
    transport: T,
    plugin_name: String,
    protocol_version: u32,
    next_id: u64,
    crashed: bool,
    closed: bool,
}

impl HostSession<ProcessTransport> {
    /// Spawns `dawsu_vst3_host` and completes the `READY` handshake.
    pub fn spawn(config: &Vst3HostConfig) -> Result<Self, HostError> {
        let transport = ProcessTransport::spawn(config)?;
        Self::from_transport(transport)
    }
}

impl<T: HostTransport> HostSession<T> {
    /// Attaches to an already-created transport and performs the `READY` handshake.
    pub fn from_transport(mut transport: T) -> Result<Self, HostError> {
        let Some(banner) = transport.read_line()? else {
            return Err(HostError::Crashed(
                "host exited before sending READY".to_string(),
            ));
        };
        let (plugin_name, protocol_version) = parse_ready_banner(&banner)?;
        if protocol_version != PROTOCOL_VERSION {
            transport.kill();
            return Err(HostError::ProtocolMismatch {
                expected: PROTOCOL_VERSION,
                actual: protocol_version,
            });
        }
        Ok(Self {
            transport,
            plugin_name,
            protocol_version,
            next_id: 1,
            crashed: false,
            closed: false,
        })
    }

    pub fn plugin_name(&self) -> &str {
        &self.plugin_name
    }

    pub fn protocol_version(&self) -> u32 {
        self.protocol_version
    }

    pub fn is_crashed(&mut self) -> bool {
        if self.crashed {
            return true;
        }
        if !self.closed && !self.transport.is_alive() {
            self.crashed = true;
        }
        self.crashed
    }

    /// Sends a tagged command and waits for its matching tagged reply.
    pub fn send(&mut self, command: &HostCommand) -> Result<HostReply, HostError> {
        if self.crashed {
            return Err(HostError::Crashed("host previously crashed".to_string()));
        }
        if self.closed {
            return Err(HostError::Closed);
        }
        if !self.transport.is_alive() {
            self.crashed = true;
            return Err(HostError::Crashed(
                "host process exited unexpectedly".to_string(),
            ));
        }

        let id = self.next_id;
        self.next_id += 1;
        let wire = command.format_tagged(id);

        if let Err(err) = self.transport.write_line(&wire) {
            if !self.transport.is_alive() {
                self.crashed = true;
                return Err(HostError::Crashed(format!("write failed after exit: {err}")));
            }
            return Err(err);
        }

        if matches!(command, HostCommand::Quit) {
            self.closed = true;
            return Ok(HostReply::Ok);
        }

        let line = match self.transport.read_line() {
            Ok(Some(line)) => line,
            Ok(None) => {
                self.crashed = true;
                return Err(HostError::Crashed(
                    "host closed stdout before replying".to_string(),
                ));
            }
            Err(err) => {
                if !self.transport.is_alive() {
                    self.crashed = true;
                    return Err(HostError::Crashed(format!("read failed after exit: {err}")));
                }
                return Err(err);
            }
        };

        parse_tagged_reply(&line, id)
    }

    pub fn show(&mut self) -> Result<(), HostError> {
        expect_ok(self.send(&HostCommand::Show)?)
    }

    pub fn hide(&mut self) -> Result<(), HostError> {
        expect_ok(self.send(&HostCommand::Hide)?)
    }

    pub fn get_state(&mut self) -> Result<String, HostError> {
        match self.send(&HostCommand::GetState)? {
            HostReply::State(b64) => Ok(b64),
            HostReply::Error(msg) => Err(HostError::PluginError(msg)),
            other => Err(HostError::InvalidReply(format!("expected STATE, got {other:?}"))),
        }
    }

    pub fn set_state(&mut self, base64_state: &str) -> Result<(), HostError> {
        expect_ok(self.send(&HostCommand::SetState(base64_state.to_string()))?)
    }

    pub fn note_names(&mut self) -> Result<Vec<(u8, String)>, HostError> {
        match self.send(&HostCommand::NoteNames)? {
            HostReply::NoteNames(names) => Ok(names),
            HostReply::Error(msg) => Err(HostError::PluginError(msg)),
            other => Err(HostError::InvalidReply(format!("expected NAMES, got {other:?}"))),
        }
    }

    pub fn render(&mut self, request: &RenderRequest) -> Result<RenderResult, HostError> {
        match self.send(&HostCommand::Render(request.clone()))? {
            HostReply::Rendered(res) => Ok(res),
            HostReply::Error(msg) => Err(HostError::PluginError(msg)),
            other => Err(HostError::InvalidReply(format!("expected RENDERED, got {other:?}"))),
        }
    }

    pub fn quit(&mut self) -> Result<(), HostError> {
        expect_ok(self.send(&HostCommand::Quit)?)
    }

    /// Consumes the session and returns the underlying transport (useful in tests).
    pub fn into_transport(mut self) -> T {
        self.closed = true;
        // Safety: we mark `closed = true` so `Drop` won't send QUIT; use ManuallyDrop via ptr::read.
        let this = std::mem::ManuallyDrop::new(self);
        // SAFETY: `this` will not be dropped, so moving `transport` out is sound.
        unsafe { std::ptr::read(&this.transport) }
    }
}

impl<T: HostTransport> Drop for HostSession<T> {
    fn drop(&mut self) {
        if !self.closed && !self.crashed && self.transport.is_alive() {
            let id = self.next_id;
            let _ = self.transport.write_line(&HostCommand::Quit.format_tagged(id));
        }
        self.closed = true;
    }
}

fn expect_ok(reply: HostReply) -> Result<(), HostError> {
    match reply {
        HostReply::Ok => Ok(()),
        HostReply::Error(msg) => Err(HostError::PluginError(msg)),
        other => Err(HostError::InvalidReply(format!("expected OK, got {other:?}"))),
    }
}

fn parse_ready_banner(line: &str) -> Result<(String, u32), HostError> {
    if let Some(msg) = line.strip_prefix("ERROR\t") {
        return Err(HostError::PluginError(msg.to_string()));
    }
    let Some(rest) = line.strip_prefix("READY\t") else {
        return Err(HostError::InvalidReply(line.to_string()));
    };
    let mut parts = rest.split('\t');
    let name = parts
        .next()
        .ok_or_else(|| HostError::InvalidReply(line.to_string()))?;
    let proto_str = parts
        .next()
        .ok_or_else(|| HostError::InvalidReply(line.to_string()))?;
    let protocol = proto_str
        .parse::<u32>()
        .map_err(|_| HostError::InvalidReply(line.to_string()))?;
    Ok((name.to_string(), protocol))
}

fn parse_tagged_reply(line: &str, expected_id: u64) -> Result<HostReply, HostError> {
    let Some((id_str, payload)) = line.split_once('\t') else {
        return Err(HostError::InvalidReply(line.to_string()));
    };
    let id = id_str
        .parse::<u64>()
        .map_err(|_| HostError::InvalidReply(line.to_string()))?;
    if id != expected_id {
        return Err(HostError::InvalidReply(format!(
            "reply id mismatch: expected {expected_id}, got {id}"
        )));
    }

    if payload == "OK" {
        return Ok(HostReply::Ok);
    }
    if let Some(msg) = payload.strip_prefix("ERROR\t") {
        return Ok(HostReply::Error(msg.to_string()));
    }
    if let Some(b64) = payload.strip_prefix("STATE\t") {
        return Ok(HostReply::State(b64.to_string()));
    }
    if payload == "STATE" {
        return Ok(HostReply::State(String::new()));
    }
    if payload == "NAMES" {
        return Ok(HostReply::NoteNames(Vec::new()));
    }
    if let Some(rest) = payload.strip_prefix("NAMES\t") {
        let mut names = Vec::new();
        for entry in rest.split('\t') {
            if entry.is_empty() {
                continue;
            }
            let Some((pitch_str, label)) = entry.split_once('=') else {
                return Err(HostError::InvalidReply(line.to_string()));
            };
            let pitch = pitch_str
                .parse::<u8>()
                .map_err(|_| HostError::InvalidReply(line.to_string()))?;
            names.push((pitch, label.to_string()));
        }
        return Ok(HostReply::NoteNames(names));
    }
    if let Some(rest) = payload.strip_prefix("RENDERED\t") {
        let parts: Vec<&str> = rest.split('\t').collect();
        if parts.len() != 3 {
            return Err(HostError::InvalidReply(line.to_string()));
        }
        let frames = parts[0]
            .parse::<u64>()
            .map_err(|_| HostError::InvalidReply(line.to_string()))?;
        let truncated = match parts[1] {
            "0" => false,
            "1" => true,
            _ => return Err(HostError::InvalidReply(line.to_string())),
        };
        let peak = parts[2]
            .parse::<f32>()
            .map_err(|_| HostError::InvalidReply(line.to_string()))?;
        return Ok(HostReply::Rendered(RenderResult {
            frames,
            truncated,
            peak,
        }));
    }

    Err(HostError::InvalidReply(line.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::mem::{offset_of, size_of};

    struct MockTransport {
        sent: Vec<String>,
        replies: VecDeque<Option<String>>,
        alive: bool,
        killed: bool,
    }

    impl MockTransport {
        fn new(replies: impl IntoIterator<Item = Option<&'static str>>) -> Self {
            Self {
                sent: Vec::new(),
                replies: replies
                    .into_iter()
                    .map(|r| r.map(ToString::to_string))
                    .collect(),
                alive: true,
                killed: false,
            }
        }
    }

    impl HostTransport for MockTransport {
        fn write_line(&mut self, line: &str) -> Result<(), HostError> {
            self.sent.push(line.to_string());
            Ok(())
        }

        fn read_line(&mut self) -> Result<Option<String>, HostError> {
            Ok(self.replies.pop_front().flatten())
        }

        fn is_alive(&mut self) -> bool {
            self.alive
        }

        fn kill(&mut self) {
            self.alive = false;
            self.killed = true;
        }
    }

    #[test]
    fn shm_layout_matches_cpp_host() {
        assert_eq!(PROTOCOL_MAGIC, u32::from_le_bytes(*b"DVH3"));
        assert_eq!(size_of::<ShmEvent>(), 8);
        assert_eq!(offset_of!(ShmBlock, events), 40);
        assert_eq!(offset_of!(ShmBlock, in_left), 40 + MAX_EVENTS * 8);
        assert_eq!(
            offset_of!(ShmBlock, in_right),
            40 + MAX_EVENTS * 8 + MAX_FRAMES * 4
        );
        assert_eq!(
            offset_of!(ShmBlock, left),
            40 + MAX_EVENTS * 8 + MAX_FRAMES * 8
        );
        assert_eq!(
            offset_of!(ShmBlock, right),
            40 + MAX_EVENTS * 8 + MAX_FRAMES * 12
        );
    }

    #[test]
    fn tagged_command_framing_and_replies() {
        let transport = MockTransport::new([
            Some("READY\tVital\t3"),
            Some("1\tOK"),
            Some("2\tOK"),
            Some("3\tSTATE\tAQAAAA=="),
            Some("4\tOK"),
            Some("5\tNAMES\t36=Kick\t38=Snare"),
            Some("6\tRENDERED\t22050\t0\t0.850000"),
        ]);

        let mut session = HostSession::from_transport(transport).unwrap();
        assert_eq!(session.plugin_name(), "Vital");
        assert_eq!(session.protocol_version(), 3);

        session.show().unwrap();
        session.hide().unwrap();
        assert_eq!(session.get_state().unwrap(), "AQAAAA==");
        session.set_state("AQAAAA==").unwrap();
        assert_eq!(
            session.note_names().unwrap(),
            vec![(36, "Kick".to_string()), (38, "Snare".to_string())]
        );

        let render = session
            .render(&RenderRequest {
                out_path: PathBuf::from("out.f32"),
                max_frames: 44100,
                quiet_frames: 2048,
                threshold: 0.0005,
                events: vec![
                    MidiEvent {
                        frame: 0,
                        status: 0x90,
                        data1: 60,
                        data2: 100,
                    },
                    MidiEvent {
                        frame: 4410,
                        status: 0x80,
                        data1: 60,
                        data2: 0,
                    },
                ],
            })
            .unwrap();
        assert_eq!(
            render,
            RenderResult {
                frames: 22050,
                truncated: false,
                peak: 0.85,
            }
        );

        session.quit().unwrap();
        let transport = session.into_transport();
        assert_eq!(
            transport.sent,
            vec![
                "1\tSHOW",
                "2\tHIDE",
                "3\tGETSTATE",
                "4\tSETSTATE\tAQAAAA==",
                "5\tNOTENAMES",
                "6\tRENDER\tout.f32\t44100\t2048\t0.0005\t0:144:60:100;4410:128:60:0",
                "7\tQUIT",
            ]
        );
    }

    #[test]
    fn rejects_wrong_protocol_version() {
        let transport = MockTransport::new([Some("READY\tOldHost\t2")]);
        let err = HostSession::from_transport(transport).err().unwrap();
        assert_eq!(
            err,
            HostError::ProtocolMismatch {
                expected: 3,
                actual: 2
            }
        );
    }

    #[test]
    fn detects_crash_on_unexpected_eof() {
        let transport = MockTransport::new([Some("READY\tCrashy\t3"), None]);
        let mut session = HostSession::from_transport(transport).unwrap();
        let err = session.show().unwrap_err();
        assert!(matches!(err, HostError::Crashed(_)));
        assert!(session.is_crashed());
        // Subsequent calls also report Crashed without hanging.
        assert!(matches!(session.hide(), Err(HostError::Crashed(_))));
    }
}
