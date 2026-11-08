//! Framed messages on the daemon's Unix socket.
//!
//! The daemon pushes bare [`super::snapshot::SidebarSnapshot`] frames and
//! answers [`DaemonRequest`] frames with [`DaemonResponse`]. All three share
//! one framing: a 4-byte big-endian length followed by JSON. Replies carry the
//! id of the request they answer, so pushes and replies can interleave on one
//! connection.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::git::GitStatus;

/// Frame payload cap. A longer length prefix means the stream is desynced.
pub const MAX_FRAME: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequestPayload {
    GetGitStatuses {
        paths: Vec<PathBuf>,
        /// Ask the daemon to refresh these paths early instead of waiting for
        /// the next sweep. The reply still comes from the current cache.
        #[serde(default)]
        prioritise: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonRequest {
    pub id: u64,
    pub payload: RequestPayload,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResponsePayload {
    GitStatuses(HashMap<PathBuf, GitStatus>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonResponse {
    pub id: u64,
    pub payload: ResponsePayload,
}

/// Write one length-prefixed JSON frame.
pub fn write_frame<W: Write>(w: &mut W, value: &impl Serialize) -> std::io::Result<()> {
    let data = serde_json::to_vec(value)?;
    w.write_all(&(data.len() as u32).to_be_bytes())?;
    w.write_all(&data)
}

/// Read one length-prefixed frame body. `None` on EOF, on error, or on a
/// length past [`MAX_FRAME`] — in the last case the stream is desynced and the
/// caller must stop reading it.
pub fn read_frame<R: Read>(r: &mut R) -> Option<Vec<u8>> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len).ok()?;
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_FRAME {
        return None;
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).ok()?;
    Some(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_round_trips() {
        let req = DaemonRequest {
            id: 7,
            payload: RequestPayload::GetGitStatuses {
                paths: vec![PathBuf::from("/wt/a")],
                prioritise: true,
            },
        };
        let mut buf = Vec::new();
        write_frame(&mut buf, &req).unwrap();
        let body = read_frame(&mut buf.as_slice()).unwrap();
        assert_eq!(serde_json::from_slice::<DaemonRequest>(&body).unwrap(), req);
    }

    #[test]
    fn an_oversized_length_is_refused_without_allocating() {
        let mut framed = ((MAX_FRAME + 1) as u32).to_be_bytes().to_vec();
        framed.extend_from_slice(b"{}");
        assert!(read_frame(&mut framed.as_slice()).is_none());
    }

    #[test]
    fn a_prioritise_hint_defaults_to_false() {
        let req: DaemonRequest =
            serde_json::from_str(r#"{"id":1,"payload":{"GetGitStatuses":{"paths":[]}}}"#).unwrap();
        assert_eq!(
            req.payload,
            RequestPayload::GetGitStatuses {
                paths: vec![],
                prioritise: false,
            }
        );
    }
}
