//! Versioned bounded framing, shared by the service and its desktop client.
use serde::{Deserialize, Serialize};
use std::io;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const VERSION: u32 = 1;
pub const MAX_FRAME: usize = 16 * 1024;
pub const PIPE_NAME: &str = r"\\.\pipe\com.unidesk.app.service.v1";
pub const SERVICE_NAME: &str = "UniDeskService";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub version: u32,
    pub request_id: u64,
    pub command: Command,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Command {
    Status,
}

/// Deliberately excludes user content, trust material, routes, and arbitrary logs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
// Independent availability flags, not mutually exclusive lifecycle states.
#[allow(clippy::struct_excessive_bools)]
pub struct Status {
    pub package_version: String,
    pub session_epoch: u64,
    pub client_session_id: u32,
    pub network_running: bool,
    pub local_device_id: String,
    pub control_port: u16,
    pub peer_count: usize,
    pub desktop_bridge_available: bool,
    pub input_helper_available: bool,
    pub installer_available: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub version: u32,
    pub request_id: u64,
    pub result: Reply,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Reply {
    Status(Status),
    UnsupportedVersion,
}

/// Read the bounded frame before decoding JSON. EOF and incomplete frames fail.
/// # Errors
/// Rejects zero/oversized lengths, truncated input, and malformed/unknown fields.
pub async fn read_frame<R, T>(reader: &mut R) -> io::Result<T>
where
    R: AsyncRead + Unpin,
    T: serde::de::DeserializeOwned,
{
    let size = reader.read_u32_le().await? as usize;
    if size == 0 || size > MAX_FRAME {
        return Err(io::Error::other("invalid IPC frame length"));
    }
    let mut body = vec![0; size];
    reader.read_exact(&mut body).await?;
    // Do not expose deserializer diagnostics (which can contain caller text).
    serde_json::from_slice(&body).map_err(|_| io::Error::other("invalid IPC frame"))
}
/// # Errors
/// Rejects serialization errors, oversized output, and failed writes.
pub async fn write_frame<W, T>(writer: &mut W, message: &T) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let body = serde_json::to_vec(message).map_err(|_| io::Error::other("invalid IPC response"))?;
    if body.is_empty() || body.len() > MAX_FRAME {
        return Err(io::Error::other("invalid IPC frame length"));
    }
    writer
        .write_u32_le(
            u32::try_from(body.len()).map_err(|_| io::Error::other("IPC frame overflow"))?,
        )
        .await?;
    writer.write_all(&body).await?;
    writer.flush().await
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn frame_round_trip_and_correlation() {
        let req = Request {
            version: VERSION,
            request_id: 42,
            command: Command::Status,
        };
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &req).await.unwrap();
        assert_eq!(
            read_frame::<_, Request>(&mut bytes.as_slice())
                .await
                .unwrap(),
            req
        );
    }
    #[tokio::test]
    async fn rejects_lengths_before_allocating_or_waiting_for_body() {
        for length in [0_u32, 16_385, u32::MAX] {
            assert!(
                read_frame::<_, Request>(&mut length.to_le_bytes().as_slice())
                    .await
                    .is_err()
            );
        }
        assert!(
            read_frame::<_, Request>(&mut &[1, 0, 0, 0][..])
                .await
                .is_err()
        );
    }
    #[tokio::test]
    async fn rejects_unknown_commands_fields_and_does_not_echo_payload() {
        for value in [
            r#"{"version":1,"request_id":1,"command":"RunShell"}"#,
            r#"{"version":1,"request_id":1,"command":"Status","clipboard":"secret"}"#,
        ] {
            let mut body = u32::try_from(value.len()).unwrap().to_le_bytes().to_vec();
            body.extend_from_slice(value.as_bytes());
            let error = read_frame::<_, Request>(&mut body.as_slice())
                .await
                .unwrap_err();
            assert_eq!(error.to_string(), "invalid IPC frame");
        }
    }
    #[tokio::test]
    async fn oversized_response_never_writes_a_partial_frame() {
        let mut bytes = Vec::new();
        assert!(
            write_frame(&mut bytes, &"x".repeat(MAX_FRAME + 1))
                .await
                .is_err()
        );
        assert!(bytes.is_empty());
    }
}
