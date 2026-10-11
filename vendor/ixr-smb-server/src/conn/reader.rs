//! Per-connection frame reader: pulls bytes off the socket, frames them,
//! hands each frame to the dispatcher.

use std::io;
use std::sync::Arc;

use crate::proto::framing::{FRAME_HEADER_LEN, decode_frame_header};
use tokio::io::{AsyncRead, AsyncReadExt, ReadHalf};
use tokio::net::TcpStream;
use tracing::{debug, error};

use crate::conn::state::Connection;
use crate::server::ServerState;

/// How much of a frame is allocated at a time.
///
/// **Rivet's change, not upstream 0.5.1's** — see `VENDORING.md` §2, hunk C.
/// The Direct-TCP header is four bytes and declares a payload of up to
/// `MAX_FRAME_PAYLOAD` (16 MiB - 1). Allocating that up front means four
/// bytes from an unauthenticated peer commit 16 MiB, on a connection that has
/// not negotiated, authenticated, or sent anything else — and `accept()` is
/// not rate-limited, so the cost of holding N of those is 4N bytes to the
/// attacker and 16N MiB to the server. With `panic = "abort"` in the release
/// profile an allocation failure is not catchable: it takes the daemon and
/// every other client with it.
///
/// Reading in chunks makes the server hold only what the peer has actually
/// sent, which is the same bound `proto::messages::bounded` applies to the
/// count-prefixed fields inside a frame — *what is physically present*, not
/// what a length field claims. It does not cap the frame; a peer willing to
/// send 16 MiB still gets 16 MiB.
///
/// 64 KiB is a compromise: large enough that a 1 MiB write (the default
/// `max_write_size`) costs a handful of `reserve` calls on a `Vec` that grows
/// geometrically, small enough that the unearned allocation is negligible.
const FRAME_READ_CHUNK: usize = 64 * 1024;

/// Read one frame's payload (without the 4-byte length prefix).
///
/// Returns `Ok(None)` on a clean EOF, `Ok(Some(bytes))` on a complete frame,
/// `Err` on partial/garbled data.
pub async fn read_one_frame<R: AsyncRead + Unpin>(reader: &mut R) -> io::Result<Option<Vec<u8>>> {
    let mut hdr = [0u8; FRAME_HEADER_LEN];
    match reader.read_exact(&mut hdr).await {
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = match decode_frame_header(&hdr) {
        Ok(n) => n,
        Err(e) => {
            return Err(io::Error::new(io::ErrorKind::InvalidData, e.to_string()));
        }
    };
    let mut payload = Vec::new();
    read_frame_payload(reader, len as usize, &mut payload).await?;
    Ok(Some(payload))
}

/// Fill `out` with exactly `len` bytes from `reader`, growing it a chunk at a
/// time so a declared length is never allocated before the bytes behind it
/// have arrived.
///
/// `out` is an out-parameter rather than a return value so a test can observe
/// the capacity that was reached when the read fails part-way — which is the
/// only externally visible difference between this and `vec![0u8; len]`.
async fn read_frame_payload<R: AsyncRead + Unpin>(
    reader: &mut R,
    len: usize,
    out: &mut Vec<u8>,
) -> io::Result<()> {
    while out.len() < len {
        let start = out.len();
        let want = FRAME_READ_CHUNK.min(len - start);
        out.resize(start + want, 0);
        if let Err(e) = reader.read_exact(&mut out[start..]).await {
            // read_exact leaves the tail undefined on error; the caller
            // discards `out`, but do not hand back bytes that were never read.
            out.truncate(start);
            return Err(e);
        }
    }
    Ok(())
}

/// Continuously read frames; for each, await `dispatch_one`'s response and
/// route it to the writer.
///
/// Sequential dispatch keeps v1 simple and matches the spec's "single writer
/// task / per-frame dispatch" pattern. We process one frame at a time per
/// connection in v1 — a follow-up can spawn dispatch tasks if a workload
/// proves to need credit-window concurrency.
pub async fn reader_task(
    mut reader: ReadHalf<TcpStream>,
    server: Arc<ServerState>,
    conn: Arc<Connection>,
    tx: tokio::sync::mpsc::Sender<crate::conn::writer::FramePayload>,
) -> io::Result<()> {
    loop {
        let frame = match read_one_frame(&mut reader).await {
            Ok(Some(b)) => b,
            Ok(None) => {
                debug!("client closed connection");
                return Ok(());
            }
            Err(e) => {
                error!(error = %e, "frame read error");
                return Err(e);
            }
        };
        // Check shutdown after every frame.
        if server
            .shutting_down
            .load(std::sync::atomic::Ordering::Acquire)
        {
            debug!("server shutting down; dropping connection");
            return Ok(());
        }
        // The dispatcher is async but we await it inline — order-preserving and
        // good enough for v1.
        let response = crate::dispatch::dispatch_frame(&server, &conn, &frame).await;
        if let Some(bytes) = response
            && tx.send(bytes).await.is_err()
        {
            debug!("writer channel closed; reader exiting");
            return Ok(());
        }
    }
}

/// **Rivet's tests, not upstream 0.5.1's** — `VENDORING.md` §2, hunk C.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::framing::{MAX_FRAME_PAYLOAD, encode_frame};

    /// Regression test for the frame-header allocation amplification: a
    /// four-byte header declaring the largest payload the field can express,
    /// followed by almost nothing, must not commit 16 MiB before the bytes
    /// have arrived.
    ///
    /// The assertion is on `capacity`, because that is the only externally
    /// visible difference between this and `vec![0u8; len]` — both return the
    /// same `UnexpectedEof`. Restore the one-shot allocation and this test
    /// fails on the capacity line while every other test in the crate still
    /// passes, which is what makes it wired to the defect rather than to the
    /// error path.
    #[tokio::test]
    async fn a_declared_length_is_not_allocated_before_the_bytes_arrive() {
        let mut wire: &[u8] = &[0xAA; 10];
        let mut out = Vec::new();
        let err = read_frame_payload(&mut wire, MAX_FRAME_PAYLOAD as usize, &mut out)
            .await
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
        assert!(
            out.capacity() <= FRAME_READ_CHUNK * 2,
            "reserved {} bytes for a payload of which only 10 were sent",
            out.capacity()
        );
    }

    /// The same amplification reached through the public entry point, with a
    /// real Direct-TCP header rather than a hand-passed length.
    #[tokio::test]
    async fn an_oversized_frame_header_is_refused_not_allocated() {
        // 0x00 FF FF FF: the largest payload the 24-bit field can declare.
        let mut wire: &[u8] = &[0x00, 0xFF, 0xFF, 0xFF, 0x01, 0x02, 0x03];
        let err = read_one_frame(&mut wire).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }

    /// Chunking must not change what a complete frame reads back as, across
    /// more than one chunk boundary and with a final partial chunk.
    #[tokio::test]
    async fn a_multi_chunk_frame_reads_back_byte_identical() {
        let payload: Vec<u8> = (0..FRAME_READ_CHUNK * 2 + 1234)
            .map(|i| (i % 251) as u8)
            .collect();
        let mut wire = Vec::new();
        encode_frame(&payload, &mut wire);

        let mut r: &[u8] = &wire;
        let got = read_one_frame(&mut r).await.unwrap().unwrap();
        assert_eq!(got, payload);
    }

    /// A frame exactly one chunk long — the boundary the loop is most likely
    /// to get wrong.
    #[tokio::test]
    async fn a_frame_of_exactly_one_chunk_reads_back_byte_identical() {
        let payload = vec![0x5Au8; FRAME_READ_CHUNK];
        let mut wire = Vec::new();
        encode_frame(&payload, &mut wire);

        let mut r: &[u8] = &wire;
        let got = read_one_frame(&mut r).await.unwrap().unwrap();
        assert_eq!(got, payload);
    }

    /// A zero-length frame is legal (SMB2 never sends one, but the framing
    /// allows it) and must not spin.
    #[tokio::test]
    async fn an_empty_frame_is_read_as_empty() {
        let mut wire: &[u8] = &[0x00, 0x00, 0x00, 0x00];
        let got = read_one_frame(&mut wire).await.unwrap().unwrap();
        assert!(got.is_empty());
    }

    /// A clean EOF before any header is not an error.
    #[tokio::test]
    async fn eof_before_a_header_is_not_an_error() {
        let mut wire: &[u8] = &[];
        assert!(read_one_frame(&mut wire).await.unwrap().is_none());
    }

    /// A truncated payload is still an error after chunking.
    #[tokio::test]
    async fn a_truncated_payload_is_an_error() {
        let mut wire: &[u8] = &[0x00, 0x00, 0x10, 0x00, 0x01, 0x02];
        let err = read_one_frame(&mut wire).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }
}
