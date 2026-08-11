//! Malware/virus scanning of uploaded file bytes against a `clamd` daemon
//! (ClamAV), using its `INSTREAM` wire protocol directly - no external crate
//! needed, the protocol is a handful of length-prefixed chunks.
//!
//! `clamd` runs as its own OS user and generally can't read arbitrary
//! application-owned paths, so this scans **bytes already in memory**
//! (exactly what [`djangors_core::extract::Multipart`] already hands you) over
//! a socket, rather than asking `clamd` to open a file by path - which also
//! means the scan can happen *before* anything is ever written to disk or a
//! [`crate::storage::Storage`] backend at all.
//!
//! Off by default: this module only compiles under the `clamav` Cargo feature,
//! and using it requires a real `clamd` daemon running and reachable - nothing
//! here starts one for you.

use std::io;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UnixStream};

/// How to reach a running `clamd` daemon.
#[derive(Debug, Clone)]
pub enum ClamdAddr {
    /// A Unix domain socket path (`clamd.conf`'s `LocalSocket`), the common
    /// case when `clamd` runs on the same host as the application.
    Unix(std::path::PathBuf),
    /// A `host:port` TCP address (`clamd.conf`'s `TCPSocket`/`TCPAddr`).
    Tcp(String, u16),
}

/// The outcome of a scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanResult {
    /// No signature matched.
    Clean,
    /// A signature matched; the name is `clamd`'s own signature identifier
    /// (e.g. `Eicar-Test-Signature` for the standard AV test file).
    Infected(String),
}

/// Errors talking to `clamd`.
#[derive(thiserror::Error, Debug)]
pub enum ClamAvError {
    /// Could not connect to, write to, or read from the `clamd` socket.
    #[error("clamd connection error: {0}")]
    Io(#[from] io::Error),
    /// `clamd` returned a response this client didn't recognize (a version
    /// mismatch or a genuinely malformed reply).
    #[error("unexpected response from clamd: {0}")]
    UnexpectedResponse(String),
}

/// A `clamd` client speaking the `INSTREAM` protocol.
#[derive(Debug, Clone)]
pub struct ClamAvScanner {
    addr: ClamdAddr,
    chunk_size: usize,
}

/// `clamd`'s own default `StreamMaxLength` is 25MB; chunking well under that
/// (and under typical socket buffer sizes) avoids ever holding an oversized
/// single write pending.
const DEFAULT_CHUNK_SIZE: usize = 8192;

impl ClamAvScanner {
    /// Creates a scanner that connects to `clamd` over a Unix domain socket.
    pub fn unix(socket_path: impl Into<std::path::PathBuf>) -> Self {
        Self {
            addr: ClamdAddr::Unix(socket_path.into()),
            chunk_size: DEFAULT_CHUNK_SIZE,
        }
    }

    /// Creates a scanner that connects to `clamd` over TCP.
    pub fn tcp(host: impl Into<String>, port: u16) -> Self {
        Self {
            addr: ClamdAddr::Tcp(host.into(), port),
            chunk_size: DEFAULT_CHUNK_SIZE,
        }
    }

    /// Scans `data` in-memory via `clamd`'s `INSTREAM` command: a `zINSTREAM\0`
    /// handshake, then the payload as a sequence of `<u32 big-endian length><chunk
    /// bytes>` frames terminated by a zero-length frame, per the real
    /// [ClamAV protocol](https://docs.clamav.net/manual/Usage/Scanning.html#stream-scan).
    pub async fn scan(&self, data: &[u8]) -> Result<ScanResult, ClamAvError> {
        match &self.addr {
            ClamdAddr::Unix(path) => {
                let stream = UnixStream::connect(path).await?;
                self.scan_over(stream, data).await
            }
            ClamdAddr::Tcp(host, port) => {
                let stream = TcpStream::connect((host.as_str(), *port)).await?;
                self.scan_over(stream, data).await
            }
        }
    }

    async fn scan_over<S>(&self, mut stream: S, data: &[u8]) -> Result<ScanResult, ClamAvError>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        stream.write_all(b"zINSTREAM\0").await?;

        for chunk in data.chunks(self.chunk_size.max(1)) {
            let len_prefix = (chunk.len() as u32).to_be_bytes();
            stream.write_all(&len_prefix).await?;
            stream.write_all(chunk).await?;
        }
        // Zero-length chunk signals end of stream.
        stream.write_all(&0u32.to_be_bytes()).await?;

        let mut response = Vec::new();
        stream.read_to_end(&mut response).await?;
        parse_response(&response)
    }
}

fn parse_response(raw: &[u8]) -> Result<ScanResult, ClamAvError> {
    let text = String::from_utf8_lossy(raw);
    let text = text.trim_end_matches('\0').trim();

    // Real clamd replies look like "stream: OK" or
    // "stream: Eicar-Test-Signature FOUND".
    let Some(rest) = text.strip_prefix("stream:") else {
        return Err(ClamAvError::UnexpectedResponse(text.to_string()));
    };
    let rest = rest.trim();

    if rest == "OK" {
        Ok(ScanResult::Clean)
    } else if let Some(signature) = rest.strip_suffix("FOUND") {
        Ok(ScanResult::Infected(signature.trim().to_string()))
    } else {
        Err(ClamAvError::UnexpectedResponse(text.to_string()))
    }
}

#[cfg(test)]
#[path = "clamav_tests.rs"]
mod tests;
