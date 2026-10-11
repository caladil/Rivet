#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;

const HEAD_ID: &str = "0123456789abcdef0123456789abcdef01234567";
const OTHER_ID: &str = "89abcdef0123456789abcdef0123456789abcdef";

fn name(text: &str) -> RepoName {
    RepoName::new(text).expect("a repository name")
}

fn commit_request(message: &str, document: &str) -> CommitRequest {
    CommitRequest {
        repository: name("machines"),
        branch: Branch::new("main").expect("main"),
        expected_head: CommitId::new(HEAD_ID).expect("an id"),
        path: DocumentPath::new("fleet/machines/kitchen pi.toml").expect("a path"),
        message: message.to_owned(),
        document: document.to_owned(),
    }
}

// ---- paths ------------------------------------------------------------------

/// **Nothing outside `fleet/machines/<label>.toml` is a path these operations
/// take.** Each of these would put git's reading or writing somewhere the
/// operation is not about.
#[test]
fn a_path_outside_the_machines_directory_is_refused() {
    for bad in [
        "",
        "/fleet/machines/rivet-01.toml",
        "/etc/passwd",
        "fleet/machines/../../etc/passwd.toml",
        "fleet/machines/../README.toml",
        "fleet/machines/sub/rivet-01.toml",
        "fleet/machines/rivet-01.txt",
        "fleet/machines/.toml",
        "fleet/machines/..toml",
        "fleet/machines/.hidden.toml",
        "fleet/machines/-option.toml",
        "fleet/machines\\rivet-01.toml",
        "fleet/machines/rivet\\01.toml",
        "fleet/machines/rivet\n01.toml",
        "README.md",
        "fleet/rivet-01.toml",
        "fleet/machines",
        "fleet/machines/",
        "./fleet/machines/rivet-01.toml",
        "fleet//machines/rivet-01.toml",
    ] {
        let refused = DocumentPath::new(bad).expect_err(bad);
        assert_eq!(refused.kind, Kind::BadRequest, "{bad:?}: {refused:?}");
    }
    let long = format!("fleet/machines/{}.toml", "a".repeat(MAX_LABEL_BYTES + 1));
    assert!(DocumentPath::new(&long).is_err());
}

/// A label a person typed — a space, a capital, a non-ASCII letter — is a
/// document path.
#[test]
fn a_machine_document_path_is_accepted_and_names_its_label() {
    for (path, label) in [
        ("fleet/machines/rivet-01.toml", "rivet-01"),
        ("fleet/machines/kitchen pi.toml", "kitchen pi"),
        ("fleet/machines/Garage.Pi.toml", "Garage.Pi"),
        ("fleet/machines/caf\u{e9}.toml", "caf\u{e9}"),
    ] {
        let parsed = DocumentPath::new(path).unwrap_or_else(|e| panic!("{path}: {e:?}"));
        assert_eq!(parsed.label(), label);
        assert_eq!(parsed.as_path(), path);
        assert_eq!(path_of(label), path);
    }
}

// ---- names ------------------------------------------------------------------

/// `ADR-052`'s `fleet` repository is the workstation's, and a commit made into
/// it would be undone by the next push.
#[test]
fn the_fleet_repository_is_refused_and_any_other_declared_name_is_accepted() {
    let refused = repository(crate::repository_objects::REPOSITORY).expect_err("fleet is refused");
    assert_eq!(refused.kind, Kind::BadRequest);
    assert!(refused.why.contains("cargo rivet fleet push"), "{}", refused.why);
    assert_eq!(repository("machines").expect("a name").as_str(), "machines");
    for bad in ["", "../fleet", "/state/git/x", "x.git", "https://example.invalid/x.git", "-x", "Machines"] {
        assert!(repository(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn a_branch_is_a_plain_name_and_never_an_option_a_path_or_head() {
    for good in ["main", "fleet", "release-1.2", "a_b"] {
        assert!(Branch::new(good).is_ok(), "{good}");
    }
    for bad in ["", "-main", ".main", "main.", "main.lock", "a..b", "HEAD", "feature/x", "a b", "a\nb", "ma~in"] {
        assert!(Branch::new(bad).is_err(), "{bad:?}");
    }
    assert_eq!(Branch::new("main").expect("main").reference(), "refs/heads/main");
    assert!(Branch::new(&"a".repeat(MAX_BRANCH_BYTES + 1)).is_err());
}

/// An abbreviation is refused: git would resolve it against whatever the
/// repository holds, and two commits can share a prefix.
#[test]
fn a_commit_id_is_full_and_lowercase() {
    assert!(CommitId::new(HEAD_ID).is_ok());
    assert!(CommitId::new(&"a".repeat(64)).is_ok());
    for bad in ["", "0123456", &HEAD_ID.to_uppercase(), &format!("{HEAD_ID}0"), "main", "HEAD~1", "--all"] {
        assert!(CommitId::new(bad).is_err(), "{bad:?}");
    }
}

// ---- who may ask ------------------------------------------------------------

#[test]
fn the_operator_reads_and_writes_and_every_route_names_its_access() {
    assert!(permits(Caller::Operator, Access::Read));
    assert!(permits(Caller::Operator, Access::Write));
    assert_eq!(Operation::Commit.access(), Access::Write);
    for op in [Operation::Head, Operation::Machines, Operation::File] {
        assert_eq!(op.access(), Access::Read, "{op:?}");
    }
    for op in Operation::ALL {
        assert_eq!(Operation::from_route(op.route()), Some(op));
        assert!(op.route().starts_with(PREFIX));
    }
    assert_eq!(Operation::from_route("/fleet-as-code/"), None);
    assert_eq!(Operation::from_route("/fleet-as-code/head/"), None);
}

// ---- requests ---------------------------------------------------------------

/// Every request survives its own wire, and a commit's message and document
/// arrive byte for byte — blank lines, a trailer, and text that looks like a
/// request line included.
#[test]
fn every_request_round_trips() {
    let requests = [
        Request::Head(HeadRequest { repository: name("machines"), branch: Branch::new("main").unwrap() }),
        Request::Machines(MachinesRequest { repository: name("machines"), commit: CommitId::new(HEAD_ID).unwrap() }),
        Request::File(FileRequest {
            repository: name("machines"),
            commit: CommitId::new(HEAD_ID).unwrap(),
            path: DocumentPath::new("fleet/machines/rivet-01.toml").unwrap(),
        }),
        Request::Commit(commit_request(
            "Rename it\n\nFleet-Override: overwrite-drift kitchen pi\n",
            "version = 1\n\nhostname = \"x\"\nrepository fleet\n\n\n",
        )),
    ];
    for request in requests {
        let body = request.render();
        let back = Request::parse(request.operation(), &body)
            .unwrap_or_else(|e| panic!("{:?}: {e:?}\n{}", request.operation(), String::from_utf8_lossy(&body)));
        assert_eq!(back, request);
    }
}

#[test]
fn a_request_with_a_line_it_does_not_read_or_one_given_twice_is_refused() {
    let unknown = b"repository machines\nbranch main\nforce yes\n";
    assert_eq!(Request::parse(Operation::Head, unknown).unwrap_err().kind, Kind::BadRequest);
    let twice = b"repository machines\nbranch main\nbranch other\n";
    assert_eq!(Request::parse(Operation::Head, twice).unwrap_err().kind, Kind::BadRequest);
    let missing = b"repository machines\n";
    assert_eq!(Request::parse(Operation::Head, missing).unwrap_err().kind, Kind::BadRequest);
    // A path is a line File and Commit read; Head does not.
    let path = b"repository machines\nbranch main\npath fleet/machines/x.toml\n";
    assert_eq!(Request::parse(Operation::Head, path).unwrap_err().kind, Kind::BadRequest);
    let fleet = b"repository fleet\nbranch main\n";
    assert_eq!(Request::parse(Operation::Head, fleet).unwrap_err().kind, Kind::BadRequest);
    assert_eq!(Request::parse(Operation::Head, b"\xff\xfe").unwrap_err().kind, Kind::BadRequest);
}

/// The path check runs in the parser, so no route can be reached with a path
/// it would refuse.
#[test]
fn a_file_or_commit_request_naming_a_path_outside_the_directory_is_refused_by_the_parser() {
    let file = format!("repository machines\ncommit {HEAD_ID}\npath ../../etc/passwd\n");
    assert_eq!(Request::parse(Operation::File, file.as_bytes()).unwrap_err().kind, Kind::BadRequest);

    let mut request = commit_request("m\n", "d\n");
    let good = String::from_utf8(Request::Commit(request.clone()).render()).unwrap();
    let bad = good.replace("path fleet/machines/kitchen pi.toml", "path fleet/machines/../../x.toml");
    assert_ne!(good, bad);
    assert_eq!(Request::parse(Operation::Commit, bad.as_bytes()).unwrap_err().kind, Kind::BadRequest);

    request.document = "x".repeat(MAX_DOCUMENT_BYTES + 1);
    let large = Request::Commit(request).render();
    let refused = Request::parse(Operation::Commit, &large).unwrap_err();
    assert_eq!(refused.kind, Kind::TooLarge, "{refused:?}");
}

#[test]
fn a_commit_whose_declared_lengths_disagree_with_its_payload_is_refused() {
    let body = String::from_utf8(Request::Commit(commit_request("message\n", "doc\n")).render()).unwrap();
    let short = body.replace("document-bytes 4", "document-bytes 3");
    assert_eq!(Request::parse(Operation::Commit, short.as_bytes()).unwrap_err().kind, Kind::BadRequest);
    let no_payload = body.split("\n\n").next().unwrap().to_owned();
    assert_eq!(Request::parse(Operation::Commit, no_payload.as_bytes()).unwrap_err().kind, Kind::BadRequest);
    let empty_message = String::from_utf8(Request::Commit(commit_request("  \n", "doc\n")).render()).unwrap();
    assert_eq!(Request::parse(Operation::Commit, empty_message.as_bytes()).unwrap_err().kind, Kind::BadRequest);
}

// ---- answers and refusals ---------------------------------------------------

#[test]
fn every_answer_round_trips() {
    let head = Head { commit: CommitId::new(HEAD_ID).unwrap(), message: "one\n\nFleet-Override: overwrite-drift a\n".into() };
    assert_eq!(Head::parse(&head.render()).unwrap(), head);

    let machines = Machines { labels: vec!["kitchen pi".into(), "rivet-01".into()], ignored: vec!["\".x\"".into()] };
    assert_eq!(Machines::parse(&machines.render()).unwrap(), machines);
    assert_eq!(Machines::parse("").unwrap(), Machines::default());

    for file in [File::Present("version = 1\n\nhostname = \"a\"\n".into()), File::Present(String::new()), File::Absent] {
        assert_eq!(File::parse(&file.render()).unwrap(), file);
    }
    let committed = Committed { commit: CommitId::new(OTHER_ID).unwrap() };
    assert_eq!(Committed::parse(&committed.render()).unwrap(), committed);
}

/// A document whose declared length disagrees with what arrived is not read:
/// a truncated document is the one that gets written back.
#[test]
fn a_file_answer_whose_length_disagrees_is_refused() {
    assert!(File::parse("present 10\nshort").is_err());
    assert!(File::parse("present\nx").is_err());
    assert!(File::parse("maybe\n").is_err());
}

/// **A stale head is its own word and its own status**, so a caller tells a
/// lost race from every other failure without reading a sentence.
#[test]
fn every_refusal_round_trips_and_a_stale_head_is_a_conflict() {
    for kind in Kind::ALL {
        let refusal = Refusal::new(kind, format!("because {}", kind.word()));
        assert_eq!(Refusal::parse(&refusal.render()), Some(refusal));
    }
    assert_eq!(Kind::StaleHead.status(), (409, "Conflict"));
    let words: std::collections::BTreeSet<&str> = Kind::ALL.iter().map(|k| k.word()).collect();
    assert_eq!(words.len(), Kind::ALL.len(), "two kinds share a word");
    assert_eq!(Refusal::parse("commit 0123\n"), None);
    assert_eq!(Refusal::parse("refused no-such-word\nwhy\n"), None);
}

#[test]
fn no_sentence_here_carries_a_run_of_spaces() {
    let mut sentences = vec![Branch::RULE.to_owned()];
    sentences.push(repository("fleet").unwrap_err().why);
    sentences.push(repository("NOPE").unwrap_err().why);
    sentences.push(DocumentPath::new("x").unwrap_err().why);
    sentences.push(check_label("a/b").unwrap_err().why);
    for sentence in sentences {
        assert!(!sentence.contains("    "), "{sentence:?}");
    }
}
