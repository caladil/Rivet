//! **The framing and the close-on-error rule**, over a socket pair.
//!
//! [`Sender::take`] is not tested: it needs a descriptor a supervisor handed
//! over, and a test that faked `LISTEN_FDS` would be asserting things about the
//! SDK's environment parsing rather than about this module. What is here is the
//! part with a rule in it.

use std::io::{BufRead as _, BufReader};

use super::*;

/// A sender and a reader standing in for the renderer.
fn paired() -> (Sender, BufReader<UnixStream>) {
    let (ours, theirs) = UnixStream::pair().expect("a socket pair");
    (Sender::on(ours), BufReader::new(theirs))
}

/// Newline-delimited JSON, one message per line — the transport the schema
/// names. A reader takes lines, so a message that did not end in one would
/// leave the renderer waiting for the rest of it.
#[test]
fn a_message_arrives_as_one_line_of_json() {
    let (sender, mut renderer) = paired();
    sender
        .send(&Message::cmd("brightness", "100"))
        .expect("a connected sender writes");

    let mut line = String::new();
    renderer.read_line(&mut line).expect("a line arrives");
    assert!(line.ends_with('\n'), "no newline terminated the message");
    let parsed: serde_json::Value =
        serde_json::from_str(line.trim_end()).expect("the line is JSON");
    assert_eq!(parsed["cmd"], "brightness");
    assert_eq!(parsed["value"], "100");
}

#[test]
fn several_messages_arrive_as_several_lines_in_order() {
    let (sender, renderer) = paired();
    for value in ["10", "20", "30"] {
        sender.send(&Message::cmd("brightness", value)).expect("written");
    }
    let values: Vec<String> = renderer
        .lines()
        .take(3)
        .map(|line| {
            let parsed: serde_json::Value =
                serde_json::from_str(&line.expect("a line")).expect("JSON");
            parsed["value"].as_str().expect("a value").to_string()
        })
        .collect();
    assert_eq!(values, ["10", "20", "30"]);
}

/// **A failed write closes the connection**, because a half-written line would
/// leave the renderer's framer reading the next message as the tail of this
/// one. Reconnecting is the only way back from that.
///
/// The closing is observed rather than inspected: once the connection has been
/// dropped, a further send is the silent no-op of a disconnected sender instead
/// of another error off a dead descriptor.
#[test]
fn a_write_to_a_departed_renderer_closes_the_connection() {
    let (sender, renderer) = paired();
    drop(renderer);

    // The first writes after the peer has gone may succeed into the socket
    // buffer; what matters is that once one fails, the connection is dropped
    // rather than retried.
    let mut failed = false;
    for _ in 0..64 {
        if sender.send(&Message::cmd("brightness", "50")).is_err() {
            failed = true;
            break;
        }
    }
    assert!(failed, "writing to a closed peer never reported an error");
    sender
        .send(&Message::cmd("brightness", "50"))
        .expect("after the failure the sender is disconnected, so this is dropped");
}

/// With no renderer connected the send is a no-op and **not** an error: the
/// controller's job is to hold the facts, and it goes on holding them while the
/// renderer is restarting.
#[test]
fn sending_with_nobody_connected_is_not_a_failure() {
    let sender = Sender {
        connection: Mutex::new(None),
    };
    sender
        .send(&Message::cmd("brightness", "100"))
        .expect("a disconnected send is silently dropped");
}

/// The endpoint's name is the one thing that must match the renderer's
/// `connects` and the manifest's `provides`, and nothing else checks it.
#[test]
fn the_endpoint_is_named_the_same_as_the_go_sockets_last_component() {
    assert_eq!(ENDPOINT, "kiosk.sock");
}
