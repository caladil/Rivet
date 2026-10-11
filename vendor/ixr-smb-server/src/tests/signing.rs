//! **Hunks E, F and G end to end, over a real socket** (`VENDORING.md`).
//!
//! A server is started on loopback and driven by hand-built frames through
//! NEGOTIATE, an NTLMv2 SESSION_SETUP from `ntlm::test_client`, and
//! TREE_CONNECT. These live inside the crate rather than under `tests/`
//! because the client needs the crate's own NTLM and signing primitives,
//! which are not public. The client is hand-composed from MS-NLMP and
//! MS-SMB2; passing proves the server agrees with this crate's reading of
//! them, and nothing about a real client.

use std::net::SocketAddr;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use super::memfs::MemFsBackend;
use crate::ntstatus;
use crate::proto::auth::ntlm::test_client::{self, Mic};
use crate::proto::crypto::{SigningAlgo, sign, signing_key_30};
use crate::proto::header::{Command, HeaderTail, SMB2_FLAGS_SIGNED, Smb2Header};
use crate::proto::messages::{
    NegotiateRequest, NegotiateResponse, SessionSetupRequest, SessionSetupResponse,
    TreeConnectRequest,
};
use crate::{Access, Share, SmbServer};

async fn start(server: SmbServer) -> SocketAddr {
    server.bind().await.expect("bind");
    let addr = server.local_addr().await.expect("addr");
    tokio::spawn(async move { server.serve().await });
    tokio::task::yield_now().await;
    addr
}

/// Only `alice`, only by password: no session on this server can be anonymous.
fn private_server() -> SmbServer {
    SmbServer::builder()
        .listen("127.0.0.1:0".parse().unwrap())
        .user("alice", "password")
        .share(Share::new("home", MemFsBackend::new()).user("alice", Access::ReadWrite))
        .build()
        .expect("build")
}

fn public_server() -> SmbServer {
    SmbServer::builder()
        .listen("127.0.0.1:0".parse().unwrap())
        .share(Share::new("public", MemFsBackend::new()).public())
        .build()
        .expect("build")
}

fn header(command: Command, message_id: u64, session_id: u64, flags: u32) -> Smb2Header {
    Smb2Header {
        credit_charge: 1,
        channel_sequence_status: 0,
        command,
        credit_request_response: 64,
        flags,
        next_command: 0,
        message_id,
        tail: HeaderTail::sync(0),
        session_id,
        signature: [0u8; 16],
    }
}

/// Send one request, signed under `key` when one is given.
async fn send(s: &mut TcpStream, hdr: Smb2Header, body: &[u8], key: Option<[u8; 16]>) {
    let mut payload = Vec::new();
    hdr.write(&mut payload).expect("header");
    payload.extend_from_slice(body);
    if let Some(key) = key {
        sign(&mut payload, &key, SigningAlgo::HmacSha256).expect("sign");
    }
    let len = payload.len();
    let mut framed = vec![0, (len >> 16) as u8, (len >> 8) as u8, len as u8];
    framed.extend_from_slice(&payload);
    s.write_all(&framed).await.expect("write");
}

/// One response: its header and its body.
async fn receive(s: &mut TcpStream) -> (Smb2Header, Vec<u8>) {
    let mut len = [0u8; 4];
    s.read_exact(&mut len).await.expect("frame header");
    let len = (usize::from(len[1]) << 16) | (usize::from(len[2]) << 8) | usize::from(len[3]);
    let mut frame = vec![0u8; len];
    s.read_exact(&mut frame).await.expect("frame");
    let (hdr, body) = Smb2Header::parse(&frame).expect("response header");
    (hdr, body.to_vec())
}

async fn negotiate(s: &mut TcpStream) -> NegotiateResponse {
    let req = NegotiateRequest {
        structure_size: 36,
        dialect_count: 2,
        security_mode: 0x0001,
        reserved: 0,
        capabilities: 0,
        client_guid: [0xCD; 16],
        negotiate_context_offset_or_client_start_time: 0,
        dialects: vec![0x0202, 0x0210],
    };
    let mut body = Vec::new();
    req.write_to(&mut body).expect("negotiate request");
    send(s, header(Command::Negotiate, 0, 0, 0), &body, None).await;
    let (hdr, body) = receive(s).await;
    assert_eq!(hdr.channel_sequence_status, ntstatus::STATUS_SUCCESS);
    NegotiateResponse::parse(&body).expect("negotiate response")
}

fn session_setup_body(token: &[u8]) -> Vec<u8> {
    let req = SessionSetupRequest {
        structure_size: 25,
        flags: 0,
        security_mode: 0x01,
        capabilities: 0,
        channel: 0,
        security_buffer_offset: 88,
        security_buffer_length: token.len() as u16,
        previous_session_id: 0,
        security_buffer: token.to_vec(),
    };
    let mut body = Vec::new();
    req.write_to(&mut body).expect("session setup request");
    body
}

/// NEGOTIATE then a two-round raw-NTLMSSP SESSION_SETUP as `alice`. Answers
/// the final status, the session id and the key the session signs with.
async fn log_on(s: &mut TcpStream, password: &str, mic: Mic) -> (u32, u64, [u8; 16]) {
    let _ = negotiate(s).await;

    let ntlm_negotiate = test_client::negotiate();
    let body = session_setup_body(&ntlm_negotiate);
    send(s, header(Command::SessionSetup, 1, 0, 0), &body, None).await;
    let (hdr, body) = receive(s).await;
    assert_eq!(hdr.channel_sequence_status, ntstatus::STATUS_MORE_PROCESSING_REQUIRED);
    let session_id = hdr.session_id;
    let challenge = SessionSetupResponse::parse(&body).expect("challenge").security_buffer;

    let (authenticate, session_key) =
        test_client::authenticate(&ntlm_negotiate, &challenge, "alice", "", password, mic);
    let body = session_setup_body(&authenticate);
    send(s, header(Command::SessionSetup, 2, session_id, 0), &body, None).await;
    let (hdr, _) = receive(s).await;
    (hdr.channel_sequence_status, session_id, signing_key_30(&session_key))
}

fn tree_connect_body(path: &str) -> Vec<u8> {
    let path: Vec<u8> = path.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let req = TreeConnectRequest {
        structure_size: 9,
        flags: 0,
        path_offset: 64 + 8,
        path_length: path.len() as u16,
        path,
    };
    let mut body = Vec::new();
    req.write_to(&mut body).expect("tree connect request");
    body
}

/// **Hunk E.** On an authenticated session an unsigned request is refused, so
/// clearing the signed flag no longer skips the signature check, and the same
/// request signed under the session's key is served.
#[tokio::test]
async fn an_authenticated_session_refuses_an_unsigned_request_and_serves_a_signed_one() {
    let addr = start(private_server()).await;
    let mut s = TcpStream::connect(addr).await.expect("connect");
    let (status, session_id, key) = log_on(&mut s, "password", Mic::Valid).await;
    assert_eq!(status, ntstatus::STATUS_SUCCESS, "the logon itself must succeed");

    let body = tree_connect_body("\\\\127.0.0.1\\home");
    send(&mut s, header(Command::TreeConnect, 3, session_id, 0), &body, None).await;
    let (hdr, _) = receive(&mut s).await;
    assert_eq!(
        hdr.channel_sequence_status,
        ntstatus::STATUS_ACCESS_DENIED,
        "an unsigned request on an authenticated session must be refused"
    );

    let signed = header(Command::TreeConnect, 4, session_id, SMB2_FLAGS_SIGNED);
    send(&mut s, signed, &body, Some(key)).await;
    let (hdr, _) = receive(&mut s).await;
    assert_eq!(
        hdr.channel_sequence_status,
        ntstatus::STATUS_SUCCESS,
        "the same request, signed, must be served"
    );
}

/// **Hunk E.** A signed request whose bytes were altered after signing is
/// refused — the check that the signed flag used to be able to skip.
#[tokio::test]
async fn an_altered_signed_request_is_refused() {
    let addr = start(private_server()).await;
    let mut s = TcpStream::connect(addr).await.expect("connect");
    let (status, session_id, key) = log_on(&mut s, "password", Mic::Valid).await;
    assert_eq!(status, ntstatus::STATUS_SUCCESS);

    let mut payload = Vec::new();
    header(Command::TreeConnect, 3, session_id, SMB2_FLAGS_SIGNED)
        .write(&mut payload)
        .expect("header");
    payload.extend_from_slice(&tree_connect_body("\\\\127.0.0.1\\home"));
    sign(&mut payload, &key, SigningAlgo::HmacSha256).expect("sign");
    let last = payload.len() - 1;
    payload[last] ^= 0x01;
    let len = payload.len();
    let mut framed = vec![0, (len >> 16) as u8, (len >> 8) as u8, len as u8];
    framed.extend_from_slice(&payload);
    s.write_all(&framed).await.expect("write");
    let (hdr, _) = receive(&mut s).await;
    assert_eq!(hdr.channel_sequence_status, ntstatus::STATUS_ACCESS_DENIED);
}

/// **Hunk F, at the door.** A client that sends no MIC is refused at
/// SESSION_SETUP, with the right password.
#[tokio::test]
async fn a_logon_without_a_mic_is_refused_at_session_setup() {
    let addr = start(private_server()).await;
    let mut s = TcpStream::connect(addr).await.expect("connect");
    let (status, _, _) = log_on(&mut s, "password", Mic::Absent).await;
    assert_eq!(status, ntstatus::STATUS_LOGON_FAILURE);
}

/// **Hunk G.** A server on which every session is authenticated says so in
/// NEGOTIATE, so a client that signs only when asked signs from the start; a
/// server with a public share still cannot, because an anonymous session has
/// no key to sign with.
#[tokio::test]
async fn negotiate_requires_signing_exactly_when_no_session_can_be_anonymous() {
    let addr = start(private_server()).await;
    let mut s = TcpStream::connect(addr).await.expect("connect");
    assert_eq!(negotiate(&mut s).await.security_mode, 0x0003, "enabled and required");

    let addr = start(public_server()).await;
    let mut s = TcpStream::connect(addr).await.expect("connect");
    assert_eq!(negotiate(&mut s).await.security_mode, 0x0001, "enabled only");
}
