//! **`POST /api/commit/<device>`** — R39, `.rulings/` R-167.
//!
//! Wave 3c's Config screen (`ADR-043` §6) used to compose a document and
//! send it straight to the machine with `apply edit`. That ran against
//! `ADR-044` §1's own single source of truth: a machine's desired state
//! lives in the fleet repository and the machine converges from it, the same
//! way every other write this server makes already does (`crate::code`).
//! R39 keeps the form exactly as built — the reading, the ordering, the
//! conditions — and changes only where Save's write lands. This module is
//! that write, and the commit itself is made by this server's own control
//! plane (`crate::repo::Repository::commit_file`), which runs git where this
//! confined program cannot.
//!
//! # A new route, reported rather than added quietly
//!
//! `ADR-043` §2's "the server grew no new route" was about READING: every
//! view still asks the TARGET MACHINE through `POST /mcp/<device>`, and this
//! module does not touch that. Committing is not a question a machine can
//! answer — it is an action on THIS server's own repository, and a browser
//! cannot reach that repository's operations itself. There was no existing route that
//! could carry it, so `POST /api/commit/<device>` is a genuine new one. It
//! is reported at the top of this session's summary as the decision it is,
//! per the brief that asked for exactly that rather than a silent addition.
//!
//! # The credential guard runs HERE, before a commit is even attempted
//!
//! `crate::code::one` already refuses a document carrying an unknown key
//! before it reaches a machine — `ADR-007` §7's schema has no key a
//! credential could be written under, so that guard catches a literal
//! wherever the schema itself would refuse it. **`ADR-038` §4 opens a
//! second region that guard cannot see**: `[applications.<name>]` is a
//! pass-through the schema deliberately does not validate, so a
//! credential-shaped setting written there is not an unknown key at all —
//! it parses cleanly. `rivet-01`'s own document carries exactly this shape,
//! read 2026-09-13: `[applications.tempmon-rivet] write-token` in clear
//! text (`STATE.md` §0.2 item 1). So this module refuses BEFORE calling
//! `crate::repo::Repository::commit_file` at all: any `[applications.*]` key
//! whose name `crate::toml_doc::is_credential_like_key` reads as
//! credential-shaped — `token`, `secret`, `password`, `passwd` or
//! `credential` anywhere in the name, `key` as a whole word, all
//! case-insensitively (`.rulings/` R-210 D16) — stops the write, names the
//! table and the key, and points at `apply install-secret` (`ADR-038` §5) as
//! where a value like this belongs instead. **Where credentials such as
//! `write-token` should actually live is the maintainer's to decide** — this
//! guard only ever refuses; it does not relocate anything, and relocating
//! `rivet-01`'s real token is reported
//! as an open question rather than done here.
//!
//! **Two readers see the document, not one**, because either alone misses a
//! spelling the other catches. `crate::toml_doc::credential_like_settings`
//! is lossless but recognises only an unindented bare key; a quoted key, an
//! indented key, or a quoted table segment is valid TOML that reader cannot
//! see. `normalized_credential_settings` reads the same text through
//! `rivetos_config`'s own parser, which resolves every one of those
//! spellings to the same name — the two scans can disagree about which KEYS
//! they can see without ever disagreeing about which NAMES count, because
//! both call the one predicate this key rule lives in.

use rivet_core::json::Json;
use rivet_core::journal::write_json_string;
use rivetos_register::Register;

use crate::code::Fleet;
use crate::repo::{ControlPlane, Repository};

/// One `POST /api/commit/<device>` request's body:
/// `{"expected": <string|null>, "document": <string>, "message": <string>}`.
struct Request {
    /// What the caller believes the repository holds now. `None` means "this
    /// machine has no document in the repository yet — create it."
    expected: Option<String>,
    /// The whole new document text.
    document: String,
    /// What to name the commit. Composed client-side, from the fields the
    /// operator actually touched (`web/config.js`'s `touched` set) — a
    /// server-side diff over `rivet_core::leaf` was considered and rejected:
    /// the client already knows precisely what changed, because it is the
    /// thing that changed it, and re-deriving that server-side from two
    /// whole-document texts would be a second, less precise account of the
    /// same fact.
    message: String,
}

fn parse_request(body: &str) -> Result<Request, String> {
    let parsed =
        rivet_core::json::parse(body).map_err(|e| format!("the request body is not JSON: {e:?}"))?;
    let document = parsed
        .get("document")
        .and_then(Json::as_str)
        .ok_or_else(|| "the request carries no `document`".to_owned())?
        .to_owned();
    let message = parsed
        .get("message")
        .and_then(Json::as_str)
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .unwrap_or("Edit the configuration")
        .to_owned();
    let expected = match parsed.get("expected") {
        Some(Json::Str(s)) => Some(s.clone()),
        _ => None,
    };
    Ok(Request { expected, document, message })
}

/// Every `[applications.*]` key in `text` whose name looks like a
/// credential, as `"[table] key"` strings ready for a refusal to print.
///
/// **Two scans, because either reader alone misses something the other
/// sees.** `crate::toml_doc`'s reader is lossless but recognises only an
/// unindented bare key — a quoted key, an indented key, or a quoted table
/// segment reads as an opaque line it never inspects (that module's own
/// header). `rivetos_config`'s parser reads every valid TOML spelling
/// correctly, but only of a document that is well-formed enough to reach
/// this far — see [`normalized_credential_settings`]. Both call
/// [`crate::toml_doc::is_credential_like_key`] for the one question that
/// must not have two answers: whether a given name counts.
///
/// # Errors
///
/// [`normalized_credential_settings`]'s — `text`'s `[applications]` region is
/// not well-formed TOML, or is not a table of tables.
fn credential_settings(text: &str) -> Result<Vec<String>, String> {
    let doc = crate::toml_doc::parse(text);
    let mut bad: Vec<String> = crate::toml_doc::credential_like_settings(&doc)
        .into_iter()
        .map(|(table, key)| format!("[{table}] {key}"))
        .collect();
    for name in normalized_credential_settings(text)? {
        if !bad.contains(&name) {
            bad.push(name);
        }
    }
    Ok(bad)
}

/// Every `[applications.*]` key `rivetos_config`'s own TOML grammar finds in
/// `text`, whose normalized name looks like a credential — the same
/// [`crate::toml_doc::is_credential_like_key`] the line-based scan checks
/// bare keys against, applied here to names that reader cannot see at all.
///
/// # Errors
///
/// `text`'s `[applications]` region is not well-formed TOML, or is not a
/// table of tables — refused rather than treated as empty, because a scan
/// that cannot parse the region cannot claim to have seen everything written
/// there (`AGENTS.md` §5: refuse rather than guess).
fn normalized_credential_settings(text: &str) -> Result<Vec<String>, String> {
    let applications = rivet_config::applications::parse_only(text).map_err(|e| {
        format!(
            "could not read this document's [applications.*] region to scan it for a \
             credential-shaped setting: {e}"
        )
    })?;
    let mut out = Vec::new();
    for app in applications.all() {
        for key in app.values().keys() {
            if crate::toml_doc::is_credential_like_key(key) {
                out.push(format!("[applications.{}] {key}", app.name()));
            }
        }
    }
    Ok(out)
}

/// Every `[services.*]` `cpu` cap in `text` the kernel will not keep, each
/// already worded as a refusal names it — `.rulings/` R-210 D22.
///
/// **Needs no second, line-based reader the way the credential scan does.**
/// `cpu` lives under `[services.<name>]`, a table `rivet_config::Document`'s
/// schema already validates and already types as
/// `rivet_core::service::CpuLimit` — there is no free-text region here the
/// way `[applications.*]` is, so one parse sees everything.
///
/// A document that does not parse at all carries nothing this can check —
/// refused elsewhere, for a reason this function has no opinion about — so
/// this answers empty rather than propagating that error.
fn unenforceable_cpu_caps(text: &str) -> Vec<String> {
    let Ok(document) = rivet_config::Document::parse(text) else {
        return Vec::new();
    };
    document
        .services()
        .iter()
        .filter_map(|service| {
            let limit = service.cpu()?;
            if limit.is_below_kernel_minimum() {
                Some(format!("[services.{}] cpu = \"{limit}\" — {}", service.name(), limit.kernel_refusal()))
            } else {
                None
            }
        })
        .collect()
}

/// Handle one `POST /api/commit/<device>`.
///
/// `device` is whatever the URL named — an identifier or a label, resolved
/// through `Register::resolve` exactly as `POST /mcp/<device>` already does,
/// so the same name works in both places.
///
/// # Errors
///
/// A sentence: no repository configured, a credential-shaped setting, a
/// device the register does not resolve, this server's control plane that
/// could not be found (`control`'s error), or `Repository::commit_file`'s own
/// words (either compare-and-swap's refusal, or the control plane's).
pub fn handle(
    fleet: Option<&Fleet>,
    register: &Register,
    control: Result<&dyn ControlPlane, String>,
    device: &str,
    body: &str,
) -> Result<String, String> {
    let fleet = fleet.ok_or_else(|| {
        format!(
            "this fleet server has no repository configured, so there is nothing to commit to. \
             Set `{}` under [applications.rivet-fleetd] in THIS SERVER'S OWN document (ADR-044 \
             section 5.4) to turn fleet-as-code on.",
            crate::code::setting::REPOSITORY
        )
    })?;

    let request = parse_request(body)?;

    let bad = credential_settings(&request.document)?;
    if !bad.is_empty() {
        return Err(format!(
            "REFUSED: nothing was committed. This document carries what looks like a \
             credential, written as an ordinary application setting: {}. ADR-038 section 5 \
             sends a value like that through `apply install-secret`, to one machine, never \
             through a document a fleet repository publishes to every clone and every \
             reviewer. Move it there instead of saving it here.",
            bad.join(", ")
        ));
    }

    // **`.rulings/` R-210 D22.** A cap this fine parses under the schema
    // unchanged — a document already saved with one must keep reading back
    // at boot — but cgroup v2 refuses it at any period, and rivet-sysd
    // journals that refusal on every start with no cause named for whoever
    // wrote the document.
    let bad_cpu = unenforceable_cpu_caps(&request.document);
    if !bad_cpu.is_empty() {
        return Err(format!(
            "REFUSED: nothing was committed. This document sets a cpu cap the kernel will not \
             keep. {}",
            bad_cpu.join(" ")
        ));
    }

    // **The same reader the guard just scanned with must agree with itself on
    // what it read.** `crate::toml_doc::parse` is built to preserve every
    // line verbatim (its own module documentation), so this can only
    // disagree if that promise has a defect — and if it does, the credential
    // scan above ran against a reading this line proves is not what will
    // actually be committed. Refusing here, rather than trusting the scan,
    // is the same "refuse rather than guess" rule `AGENTS.md` §5 states for
    // everything else this server reads.
    let parsed = crate::toml_doc::parse(&request.document);
    if crate::toml_doc::serialize(&parsed) != request.document {
        return Err(
            "REFUSED: nothing was committed. This server's own TOML reader did not read this \
             document back as written, which means the credential guard above cannot be \
             trusted to have seen everything in it. This is a defect in cmd/rivet-fleetd/src/\
             toml_doc.rs, not in the document — report it rather than retyping the same text."
                .to_owned(),
        );
    }

    let device_record = register.resolve(device).map_err(|e| e.to_string())?;
    let path = crate::repo::path_of(&device_record.name);

    let repository = Repository { name: &fleet.repository, control: control? };
    let commit = repository.commit_file(
        &fleet.branch,
        &path,
        request.expected.as_deref(),
        &request.document,
        &request.message,
    )?;

    let mut out = String::from("{\"commit\":");
    write_json_string(&commit, &mut out);
    out.push('}');
    Ok(out)
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use std::cell::{Cell, RefCell};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use rivet_core::fleet_as_code::{self as fac, CommitId, Committed, Head, Kind, Operation, Refusal, Request};
    use rivetos_register::Device;

    use super::*;

    /// **With no fleet repository configured, the write is refused before
    /// anything else is even looked at** — the server-side half of "Save is
    /// disabled", so a client that somehow posted anyway still cannot write.
    #[test]
    fn with_no_repository_configured_the_write_is_refused() {
        let register = Register::default();
        let why = handle(None, &register, Err("not asked".to_owned()), "rivet-01", "{\"document\":\"x\"}")
            .expect_err("must be refused");
        assert!(why.contains("fleet-repository"), "{why}");
        assert!(why.contains("no repository"), "{why}");
    }

    /// **A credential-shaped application setting is refused, and names the
    /// table and the key** — this is R39's hard stop, proven against
    /// `rivet-01`'s real shape (`write-token`, placeholder value).
    #[test]
    fn a_credential_shaped_setting_is_refused_and_named() {
        let bad = credential_settings(
            "version = 1\n[applications.tempmon-rivet]\nwrite-token = \"PLACEHOLDER\"\n",
        )
        .expect("parses");
        assert_eq!(bad, vec!["[applications.tempmon-rivet] write-token".to_owned()]);
    }

    /// An ordinary application setting is not flagged.
    #[test]
    fn an_ordinary_setting_passes_the_guard() {
        assert!(credential_settings(
            "version = 1\n[applications.rivet-proxyd]\nupstream-name = \"rivet-01.local\"\n"
        )
        .expect("parses")
        .is_empty());
    }

    // ---- D22 (`.rulings/` R-210): a cpu cap the kernel will not keep -------

    /// **`unenforceable_cpu_caps` finds the cap and names the service and the
    /// value.**
    #[test]
    fn a_cpu_cap_below_the_kernel_minimum_is_found_and_named() {
        let found = unenforceable_cpu_caps(
            "version = 1\n[services.photo-index]\nenabled = true\ncpu = \"0.005\"\n",
        );
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("photo-index"), "{found:?}");
        assert!(found[0].contains("0.005"), "{found:?}");
    }

    /// The kernel's own minimum, no cap at all, and a document that does not
    /// parse are all silently fine — the last because refusing it is a
    /// different guard's job, not this one's.
    #[test]
    fn an_enforceable_cap_no_cap_and_an_unparsable_document_are_all_unflagged() {
        assert!(unenforceable_cpu_caps(
            "version = 1\n[services.photo-index]\nenabled = true\ncpu = \"0.01\"\n"
        )
        .is_empty());
        assert!(unenforceable_cpu_caps("version = 1\n[services.photo-index]\nenabled = true\n").is_empty());
        assert!(unenforceable_cpu_caps("[services\n").is_empty());
    }

    /// **The commit path refuses before any commit call** — the same shape as
    /// `every_valid_spelling_of_a_credential_shaped_key_is_refused_before_any_commit_call`
    /// below, for the other D22 writer this ruling names.
    #[test]
    fn a_cpu_cap_below_the_kernel_minimum_is_refused_before_any_commit_call() {
        let fleet = a_fleet();
        let register = Register::default();
        let document = "version = 1\n[services.photo-index]\nenabled = true\ncpu = \"0.005\"\n";
        let body = request_body(None, document, "Edit the configuration");
        let control: Result<&dyn ControlPlane, String> = Err(
            "this fake must never be reached — a cpu cap the kernel will not keep must be \
             refused before any control-plane call"
                .to_owned(),
        );
        let why = handle(Some(&fleet), &register, control, "rivet-01", &body).unwrap_err();
        assert!(why.contains("photo-index"), "{why}");
        assert!(why.contains("cpu"), "{why}");
        assert!(!why.contains("must never be reached"), "the guard did not run first: {why}");
    }

    /// The request body is parsed permissively about `message` — an absent
    /// or blank one gets a sentence rather than an empty commit message,
    /// which the control plane's commit refuses outright.
    #[test]
    fn a_missing_message_gets_a_default_rather_than_an_empty_commit() {
        let request = parse_request("{\"document\":\"x\"}").expect("parses");
        assert_eq!(request.message, "Edit the configuration");
        assert_eq!(request.expected, None);
    }

    /// `expected` carries through when present, and stays `None` for
    /// anything that is not a JSON string — `null` included, which is how a
    /// browser spells "create this document new".
    #[test]
    fn expected_is_read_when_present_and_none_otherwise() {
        let with = parse_request("{\"document\":\"x\",\"expected\":\"old\"}").expect("parses");
        assert_eq!(with.expected.as_deref(), Some("old"));
        let without = parse_request("{\"document\":\"x\",\"expected\":null}").expect("parses");
        assert_eq!(without.expected, None);
    }

    // ---- B16: every valid spelling of a credential-shaped key -------------

    fn request_body(expected: Option<&str>, document: &str, message: &str) -> String {
        let mut out = String::from("{\"document\":");
        write_json_string(document, &mut out);
        out.push_str(",\"message\":");
        write_json_string(message, &mut out);
        out.push_str(",\"expected\":");
        match expected {
            Some(text) => write_json_string(text, &mut out),
            None => out.push_str("null"),
        }
        out.push('}');
        out
    }

    fn a_fleet() -> Fleet {
        Fleet { repository: "machines".to_owned(), branch: "main".to_owned(), baselines: PathBuf::new(), dry_run: false }
    }

    /// **`R39`'s hard stop, for every valid way TOML lets a key be spelled**
    /// — `B16`. `crate::toml_doc`'s own reader sees only an unindented bare
    /// key; a quoted key, an indented key, a quoted table segment, and a
    /// single-quoted (literal) key are all TOML exactly as valid, and
    /// `handle` must refuse every one of them — before `fleet`/`control` are
    /// even touched, which is why `control` here is a value that fails the
    /// test loudly if `handle` ever reaches for it.
    #[test]
    fn every_valid_spelling_of_a_credential_shaped_key_is_refused_before_any_commit_call() {
        let fleet = a_fleet();
        let forms: &[(&str, &str)] = &[
            ("a quoted key", "[applications.tempmon-rivet]\n\"write-token\" = \"SYNTHETIC-PLACEHOLDER\"\n"),
            ("an indented key", "[applications.tempmon-rivet]\n    write-token = \"SYNTHETIC-PLACEHOLDER\"\n"),
            (
                "a quoted table segment",
                "[\"applications\".\"tempmon-rivet\"]\nwrite-token = \"SYNTHETIC-PLACEHOLDER\"\n",
            ),
            (
                "a literal (single-quoted) key",
                "[applications.tempmon-rivet]\n'write-token' = \"SYNTHETIC-PLACEHOLDER\"\n",
            ),
        ];
        for (label, document) in forms {
            let register = Register::default();
            let body = request_body(None, document, "Edit the configuration");
            let control: Result<&dyn ControlPlane, String> = Err(
                "this fake must never be reached — a credential-shaped setting must be refused \
                 before any control-plane call"
                    .to_owned(),
            );
            let why = handle(Some(&fleet), &register, control, "rivet-01", &body)
                .unwrap_err();
            assert!(why.contains("write-token"), "{label}: {why}");
            assert!(why.contains("credential"), "{label}: {why}");
            assert!(!why.contains("must never be reached"), "{label}: the guard did not run first: {why}");
        }
    }

    /// **D16, at the fleet-document boundary** — `.rulings/` R-210. Unlike
    /// the test above, which varies the TOML SPELLING of one name
    /// (`write-token`), this varies the NAME itself: `apitoken` and
    /// `authToken` carry no hyphen at all, `API_TOKEN`/`db_passwd`/`apiKey`
    /// use underscores or mixed case a manifest's own setting-name alphabet
    /// never allows, and `secretary`/`tokenizer` merely contain `secret` and
    /// `token`. Every one must still be refused before `fleet`/`control` are
    /// touched.
    #[test]
    fn every_d16_credential_shaped_name_is_refused_before_any_commit_call() {
        let fleet = a_fleet();
        for bad in [
            "apitoken", "authToken", "API_TOKEN", "db-passwd", "db_passwd", "usercredentials",
            "write-token", "api-key", "api_key", "apiKey", "key", "secret-santa", "secretary",
            "tokenizer", "Password",
        ] {
            let register = Register::default();
            let document = format!("[applications.tempmon-rivet]\n{bad} = \"SYNTHETIC-PLACEHOLDER\"\n");
            let body = request_body(None, &document, "Edit the configuration");
            let control: Result<&dyn ControlPlane, String> = Err(
                "this fake must never be reached — a credential-shaped setting must be refused \
                 before any control-plane call"
                    .to_owned(),
            );
            let why = handle(Some(&fleet), &register, control, "rivet-01", &body).unwrap_err();
            assert!(why.contains(bad), "{bad:?}: {why}");
            assert!(why.contains("credential"), "{bad:?}: {why}");
            assert!(!why.contains("must never be reached"), "{bad:?}: the guard did not run first: {why}");
        }
    }

    /// **`keyboard-layout` commits** — `.rulings/` R-210 D16. The old
    /// substring rule on `key` refused it as a false positive (`keyboard`
    /// starts with the three letters `k`, `e`, `y`); the whole-word rule
    /// does not.
    #[test]
    fn a_setting_containing_key_only_as_a_substring_still_commits() {
        let before = "version = 1\nhostname = \"rivet-01\"\n[applications.aura-controller]\n\
                       keyboard-layout = \"us\"\n";
        let after = "version = 1\nhostname = \"rivet-01\"\n[applications.aura-controller]\n\
                      keyboard-layout = \"uk\"\n";
        let path = crate::repo::path_of("rivet-01");
        let fake = Fake::seeded(&[(&path, before)]);
        let fleet = a_fleet();
        let register = a_register();
        let body = request_body(Some(before), after, "Edit keyboard-layout");

        let control: Result<&dyn ControlPlane, String> = Ok(&fake);
        let result = handle(Some(&fleet), &register, control, "rivet-01", &body);
        let commit = result.expect("keyboard-layout must not be refused as a credential");
        assert!(commit.contains("\"commit\":"), "{commit}");
        assert_eq!(fake.files.borrow().get(&path), Some(&after.to_owned()), "bytes must be preserved exactly");
    }

    /// Malformed TOML in `[applications]` is refused BY NAME — citing the
    /// region it could not read — rather than silently treated as though it
    /// held no credential at all. `AGENTS.md` §5: refuse rather than guess.
    #[test]
    fn malformed_applications_toml_is_refused_by_name_rather_than_treated_as_credential_free() {
        let fleet = a_fleet();
        let register = Register::default();
        let body = request_body(None, "[applications\n", "Edit the configuration");
        let control: Result<&dyn ControlPlane, String> =
            Err("must never be reached".to_owned());
        let why = handle(Some(&fleet), &register, control, "rivet-01", &body).unwrap_err();
        assert!(why.contains("applications"), "{why}");
        assert!(!why.contains("must never be reached"), "{why}");
    }

    // ---- a recording fake for the repository and control plane ------------

    /// A tiny in-memory fleet-as-code repository — one branch, seeded with
    /// whatever files a test gives it. `crate::repo::tests`' own `Fake`
    /// proves the wire protocol end to end; this one exists only so this
    /// module's own tests can prove WHEN the control plane is reached —
    /// every route asked is recorded, in order, and a credential-shaped
    /// setting above must add nothing to it.
    struct Fake {
        files: RefCell<BTreeMap<String, String>>,
        commit_id: RefCell<String>,
        next: Cell<u32>,
        asked: RefCell<Vec<String>>,
    }

    impl Fake {
        fn seeded(files: &[(&str, &str)]) -> Self {
            Self {
                files: RefCell::new(files.iter().map(|(p, t)| ((*p).to_owned(), (*t).to_owned())).collect()),
                commit_id: RefCell::new(format!("{:040x}", 1u32)),
                next: Cell::new(1),
                asked: RefCell::new(Vec::new()),
            }
        }

        fn answer(&self, route: &str, body: &[u8]) -> Result<String, Refusal> {
            let op = Operation::from_route(route).ok_or_else(|| Refusal::new(Kind::NotAllowed, "an unknown route"))?;
            let request = Request::parse(op, body)?;
            match request {
                Request::Head(_) => Ok(Head {
                    commit: CommitId::new(&self.commit_id.borrow()).expect("a commit id"),
                    message: "seed".to_owned(),
                }
                .render()),
                Request::Machines(_) => Err(Refusal::new(Kind::Failed, "not used by these tests")),
                Request::File(r) => {
                    let answer = self
                        .files
                        .borrow()
                        .get(&r.path.as_path())
                        .map_or(fac::File::Absent, |t| fac::File::Present(t.clone()));
                    Ok(answer.render())
                }
                Request::Commit(r) => {
                    if r.expected_head.as_str() != self.commit_id.borrow().as_str() {
                        return Err(Refusal::new(Kind::StaleHead, "REFUSED, nothing was committed: stale head"));
                    }
                    self.files.borrow_mut().insert(r.path.as_path(), r.document.clone());
                    self.next.set(self.next.get() + 1);
                    let id = format!("{:040x}", self.next.get());
                    *self.commit_id.borrow_mut() = id.clone();
                    Ok(Committed { commit: CommitId::new(&id).expect("a commit id") }.render())
                }
            }
        }
    }

    impl ControlPlane for Fake {
        fn post(&self, route: &str, body: &[u8]) -> Result<(u16, Vec<u8>), String> {
            self.asked.borrow_mut().push(route.to_owned());
            Ok(match self.answer(route, body) {
                Ok(text) => (200, text.into_bytes()),
                Err(refusal) => (refusal.kind.status().0, refusal.render().into_bytes()),
            })
        }
    }

    fn a_register() -> Register {
        let mut register = Register::empty("unused-in-this-test");
        register
            .enroll(Device { id: "0011223344556677".to_owned(), name: "rivet-01".to_owned(), ..Device::default() })
            .expect("a fresh device enrolls");
        register
    }

    /// **An ordinary application setting still commits, and its bytes are
    /// preserved exactly** — the credential guard above must refuse a real
    /// match and nothing else, and the control plane sees exactly the
    /// document that was sent.
    #[test]
    fn an_ordinary_setting_still_commits_with_its_bytes_preserved() {
        let before = "version = 1\nhostname = \"rivet-01\"\n[applications.rivet-proxyd]\n\
                       upstream-name = \"rivet-01.local\"\n";
        let after = "version = 1\nhostname = \"rivet-01\"\n[applications.rivet-proxyd]\n\
                      upstream-name = \"rivet-02.local\"\n";
        let path = crate::repo::path_of("rivet-01");
        let fake = Fake::seeded(&[(&path, before)]);
        let fleet = a_fleet();
        let register = a_register();
        let body = request_body(Some(before), after, "Edit upstream-name");

        let control: Result<&dyn ControlPlane, String> = Ok(&fake);
        let result = handle(Some(&fleet), &register, control, "rivet-01", &body);
        let commit = result.expect("an ordinary setting must commit");
        assert!(commit.contains("\"commit\":"), "{commit}");
        assert_eq!(fake.files.borrow().get(&path), Some(&after.to_owned()), "bytes must be preserved exactly");
        assert_eq!(
            *fake.asked.borrow(),
            vec!["/fleet-as-code/head", "/fleet-as-code/file", "/fleet-as-code/commit"]
        );
    }

    /// **A cpu cap the kernel keeps still commits** — `.rulings/` R-210 D22.
    /// The refusal is only for a cap BELOW the minimum; the minimum itself
    /// is one cgroup v2 enforces.
    #[test]
    fn an_enforceable_cpu_cap_still_commits() {
        let before = "version = 1\nhostname = \"rivet-01\"\n[services.photo-index]\n\
                       enabled = true\n";
        let after = "version = 1\nhostname = \"rivet-01\"\n[services.photo-index]\n\
                      enabled = true\ncpu = \"0.5\"\n";
        let path = crate::repo::path_of("rivet-01");
        let fake = Fake::seeded(&[(&path, before)]);
        let fleet = a_fleet();
        let register = a_register();
        let body = request_body(Some(before), after, "Cap photo-index at half a core");

        let control: Result<&dyn ControlPlane, String> = Ok(&fake);
        let result = handle(Some(&fleet), &register, control, "rivet-01", &body);
        let commit = result.expect("an enforceable cpu cap must commit");
        assert!(commit.contains("\"commit\":"), "{commit}");
        assert_eq!(fake.files.borrow().get(&path), Some(&after.to_owned()), "bytes must be preserved exactly");
    }
}
