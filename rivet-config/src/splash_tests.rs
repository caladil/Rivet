//! What `[splash]` means, and what its absence means.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;

/// **Both on by default**, which is the ruling: a headless board whose address
/// came from DHCP is otherwise a machine nobody can find without reading a
/// router's lease table.
#[test]
fn a_machine_that_said_nothing_shows_both_lines() {
    let quiet = Splash::default();
    assert!(quiet.endpoint());
    assert!(quiet.version());
}

#[test]
fn each_line_is_independent() {
    assert!(!Splash::new(false, true).endpoint());
    assert!(Splash::new(false, true).version());
    assert!(Splash::new(true, false).endpoint());
    assert!(!Splash::new(true, false).version());
}

/// The default passes no suppression flags at all — only the port, which the
/// splash cannot know on its own.
#[test]
fn the_default_passes_only_the_port() {
    assert_eq!(
        Splash::default().arguments(7777, None),
        vec!["--port".to_string(), "7777".to_string()]
    );
}

#[test]
fn suppressing_a_line_passes_its_flag() {
    let args = Splash::new(false, true).arguments(7777, None);
    assert!(args.contains(&"--no-endpoint".to_string()), "{args:?}");
    assert!(!args.contains(&"--no-version".to_string()), "{args:?}");

    let args = Splash::new(true, false).arguments(7777, None);
    assert!(args.contains(&"--no-version".to_string()), "{args:?}");
    assert!(!args.contains(&"--no-endpoint".to_string()), "{args:?}");
}

#[test]
fn suppressing_both_passes_both_flags() {
    let args = Splash::new(false, false).arguments(9999, None);
    assert!(args.contains(&"--no-endpoint".to_string()), "{args:?}");
    assert!(args.contains(&"--no-version".to_string()), "{args:?}");
    assert!(args.contains(&"9999".to_string()), "{args:?}");
}

/// **The port is always passed, even when the endpoint line is off.** A
/// conditional would mean the splash's default and the document's could differ
/// on a machine where nobody noticed, and the flag costs nothing.
#[test]
fn the_port_is_passed_even_when_the_endpoint_is_suppressed() {
    let args = Splash::new(false, false).arguments(7777, None);
    let port = args.iter().position(|a| a == "--port").expect("--port");
    assert_eq!(args.get(port + 1).map(String::as_str), Some("7777"));
}

// ---------------------------------------------------------------------------
// The endpoint NAME — both halves required
// ---------------------------------------------------------------------------

fn doc(text: &str) -> crate::Document {
    crate::Document::parse(text).expect("a valid document")
}

/// **A hostname and an announcing mdnsd together mean a person can type what
/// they see.** That is the whole value of the line.
#[test]
fn a_hostname_with_mdns_enabled_becomes_a_dot_local_name() {
    let d = doc("version = 1\nhostname = \"devpi\"\n[services.rivet-mdnsd]\nenabled = true\n");
    assert_eq!(Splash::endpoint_name(&d).as_deref(), Some("devpi.local"));
}

/// **A hostname alone resolves nowhere.** `ADR-025` puts it in the machine's own
/// `/etc/hosts`, which does nothing for the laptop reading the screen.
#[test]
fn a_hostname_without_mdns_is_not_a_name_anyone_can_use() {
    let d = doc("version = 1\nhostname = \"devpi\"\n");
    assert_eq!(Splash::endpoint_name(&d), None);
}

/// And mdnsd DISABLED is not enabled, which is the case a document states
/// explicitly rather than by omission.
#[test]
fn mdns_present_but_disabled_is_not_a_name() {
    let d = doc("version = 1\nhostname = \"devpi\"\n[services.rivet-mdnsd]\nenabled = false\n");
    assert_eq!(Splash::endpoint_name(&d), None);
}

/// mdnsd without a hostname announces a name the document never chose.
#[test]
fn mdns_without_a_hostname_is_not_a_name() {
    let d = doc("version = 1\n[services.rivet-mdnsd]\nenabled = true\n");
    assert_eq!(Splash::endpoint_name(&d), None);
}

/// **A name that does not resolve is worse than an address**, so a document
/// that says nothing gets the address the splash discovers.
#[test]
fn a_silent_document_gets_no_name_and_therefore_its_address() {
    assert_eq!(Splash::endpoint_name(&doc("version = 1\n")), None);
}

/// Another enabled service is not mdnsd. The check is on the NAME of the
/// service, and a second spelling of it would be a second thing to get wrong.
#[test]
fn some_other_enabled_service_does_not_announce_a_name() {
    let d = doc("version = 1\nhostname = \"devpi\"\n[services.rivet-timed]\nenabled = true\n");
    assert_eq!(Splash::endpoint_name(&d), None);
}

/// When there is a name it is passed through, and the port goes with it — the
/// splash draws `devpi.local:7777` and needs both halves.
#[test]
fn a_name_is_passed_to_the_splash_with_its_port() {
    let args = Splash::default().arguments(7777, Some("devpi.local"));
    let at = args.iter().position(|a| a == "--name").expect("--name");
    assert_eq!(args.get(at + 1).map(String::as_str), Some("devpi.local"));
    assert!(args.contains(&"7777".to_string()), "{args:?}");
}
