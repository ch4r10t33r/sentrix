//! InvokeCodec — request_response::Codec for /borgkit/invoke/1.0.0
//!
//! Wire format: 4-byte big-endian length prefix + UTF-8 JSON body.

use async_trait::async_trait;
use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use libp2p::request_response;
use std::io;

use crate::node::{AgentRequest, AgentResponse};

#[derive(Clone, Default)]
pub struct InvokeCodec;

#[async_trait]
impl request_response::Codec for InvokeCodec {
    type Protocol = libp2p::StreamProtocol;
    type Request  = AgentRequest;
    type Response = AgentResponse;

    async fn read_request<T>(&mut self, _proto: &Self::Protocol, io: &mut T) -> io::Result<Self::Request>
    where T: AsyncRead + Unpin + Send {
        let json = read_lp(io).await?;
        serde_json::from_slice(&json).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    async fn read_response<T>(&mut self, _proto: &Self::Protocol, io: &mut T) -> io::Result<Self::Response>
    where T: AsyncRead + Unpin + Send {
        let json = read_lp(io).await?;
        serde_json::from_slice(&json).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    async fn write_request<T>(&mut self, _proto: &Self::Protocol, io: &mut T, req: Self::Request) -> io::Result<()>
    where T: AsyncWrite + Unpin + Send {
        let json = serde_json::to_vec(&req).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        write_lp(io, &json).await
    }

    async fn write_response<T>(&mut self, _proto: &Self::Protocol, io: &mut T, resp: Self::Response) -> io::Result<()>
    where T: AsyncWrite + Unpin + Send {
        let json = serde_json::to_vec(&resp).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        write_lp(io, &json).await
    }
}

// ── LP framing ────────────────────────────────────────────────────────────────

async fn read_lp<T: AsyncRead + Unpin>(io: &mut T) -> io::Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    io.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    let mut body = vec![0u8; len];
    io.read_exact(&mut body).await?;
    Ok(body)
}

async fn write_lp<T: AsyncWrite + Unpin>(io: &mut T, data: &[u8]) -> io::Result<()> {
    let len = (data.len() as u32).to_be_bytes();
    io.write_all(&len).await?;
    io.write_all(data).await?;
    io.flush().await
}
