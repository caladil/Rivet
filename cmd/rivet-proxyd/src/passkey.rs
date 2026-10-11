//! **Passkeys beside client certificates** — `ADR-063` §4, `ADR-046` §4.
//!
//! This proxy is a WebAuthn relying party. `rivet_webauthn` makes every check
//! a ceremony needs; this module keeps what those checks cannot: the
//! challenges it issued, the sessions it granted, the enrolled credentials and
//! the one-time enrollment code — which is to say the files, the clock and the
//! randomness.
//!
//! # Where each thing lives
//!
//! * **Enrolled credentials**: `passkeys` in this proxy's own data directory,
//!   `rivet_webauthn::store`'s text format. Public keys only; nothing in it can
//!   sign. Read at every ceremony and written by rename, so there is no copy in
//!   memory to go stale.
//! * **The enrollment code**: `passkey-enrolment` beside it, written by
//!   `rivet apply passkey-code` (`rivet_core::passkey`). Honoured for its
//!   lifetime from the file's modification time, deleted when a passkey is
//!   enrolled with it or when it has been guessed at too often.
//! * **Challenges and sessions**: memory only. A challenge is single-use and
//!   lives `CHALLENGE_LIFETIME`; a session lives `SESSION_LIFETIME` and
//!   **dies with the process**, so restarting the proxy signs every browser
//!   out. That is deliberate: a session table on disk would be a bearer
//!   credential at rest, and signing in again costs one touch.
//!
//! # One relying party per name
//!
//! **A ceremony is answered at each name this proxy's certificate carries**,
//! and at no other. The RP ID and origin come from the request's `Host`, read
//! without its port, and only when that name is one of the certificate's; any
//! other `Host` is refused before anything is read or issued. A passkey belongs
//! to the name it was enrolled at: the store records it, sign-in offers and
//! accepts only the passkeys enrolled at the name being signed in to, and a
//! challenge and a session are each bound to the name they were issued at.
//! A line in the store written before names were recorded belongs to the
//! machine's `upstream-name`, which was the one relying party then.
//!
//! # The reserved paths
//!
//! The pages a person visits are `/sign-in` and `/sign-in/enroll`, the
//! same address on every machine; `sign-in` is a reserved application path
//! (`rivet_core::package::application::RESERVED_PROXY_PATHS`). The calls those
//! pages make are served under `/.rivet/`, which no application can claim
//! either: a `proxy-path` must begin with a lowercase letter
//! (`rivet_core::package::application::is_proxy_path`).

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use rivet_webauthn::{RelyingParty, UserVerification, b64url, store};

/// The reserved prefix the pages' calls are served under. No `proxy-path` can
/// begin with a dot.
pub(crate) const PREFIX: &str = "/.rivet";
/// The sign-in page — `ADR-046` §4.
pub(crate) const SIGN_IN_PAGE: &str = rivet_core::passkey::SIGN_IN_PAGE;
/// The enrollment page, under [`SIGN_IN_PAGE`].
pub(crate) const ENROLL_PAGE: &str = rivet_core::passkey::ENROLL_PAGE;
/// **Where the sign-in page used to be.** A bookmark or a printed note still
/// names it, so it answers a permanent redirect to [`SIGN_IN_PAGE`].
pub(crate) const OLD_SIGN_IN_PAGE: &str = "/.rivet/login";
/// Where the enrollment page used to be; it redirects to [`ENROLL_PAGE`].
pub(crate) const OLD_ENROLL_PAGE: &str = "/.rivet/enrol";

/// The session cookie's name.
pub(crate) const COOKIE: &str = "rivet-session";

/// The enrolled credentials, in the data directory.
pub(crate) const STORE_FILE: &str = "passkeys";

/// How long a challenge may be answered. Two minutes covers a person finding
/// their phone; the browser's own timeout, sent with it, is one.
pub(crate) const CHALLENGE_LIFETIME: Duration = Duration::from_secs(120);
/// How long a browser stays signed in.
pub(crate) const SESSION_LIFETIME: Duration = Duration::from_secs(12 * 60 * 60);
/// At most this many challenges outstanding; the oldest goes first.
const MAX_CHALLENGES: usize = 32;
/// At most this many sessions; the oldest goes first.
const MAX_SESSIONS: usize = 64;
/// The largest body a ceremony may send.
pub(crate) const MAX_BODY: usize = 64 * 1024;

const LOGIN_HTML: &str = include_str!("login.html");
const ENROLL_HTML: &str = include_str!("enroll.html");

/// What a registration was authorised by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Authority {
    /// A signed-in session: an enrolled operator adding a device.
    Session,
    /// The one-time code, identified by its file's modification time so that a
    /// code withdrawn or replaced between the two halves is not honoured.
    Code(SystemTime),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ceremony {
    Register(Authority),
    Login,
}

struct Challenge {
    bytes: Vec<u8>,
    ceremony: Ceremony,
    /// The name it was issued at; it is answered there or not at all.
    name: String,
    issued: Instant,
}

struct Session {
    token: String,
    /// The name it was granted at. The cookie is host-only, so a browser sends
    /// it nowhere else; this makes the proxy refuse it anywhere else too.
    name: String,
    expires: Instant,
}

#[derive(Default)]
struct State {
    challenges: Vec<Challenge>,
    sessions: Vec<Session>,
    /// Wrong codes since the code file with this modification time appeared.
    wrong: Option<(SystemTime, u32)>,
}

/// One answer, before it is written to a socket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Reply {
    pub status: u16,
    pub content_type: &'static str,
    pub body: String,
    pub set_cookie: Option<String>,
}

impl Reply {
    fn json(status: u16, body: String) -> Self {
        Self {
            status,
            content_type: "application/json",
            body,
            set_cookie: None,
        }
    }

    fn error(status: u16, why: &str) -> Self {
        Self::json(status, format!("{{\"error\":{}}}", json_string(why)))
    }

    fn html(page: &str) -> Self {
        Self {
            status: 200,
            content_type: "text/html; charset=utf-8",
            body: page.to_string(),
            set_cookie: None,
        }
    }

    fn with_cookie(mut self, cookie: String) -> Self {
        self.set_cookie = Some(cookie);
        self
    }
}

/// A JSON string literal.
pub(crate) fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The relying party, its state and where its files are.
pub(crate) struct Passkeys {
    /// The DNS names on this proxy's own certificate, lowercase: the names a
    /// ceremony is answered at.
    names: Vec<String>,
    /// The machine's `upstream-name`: the name a store line without one was
    /// enrolled at.
    legacy: String,
    data: PathBuf,
    state: Mutex<State>,
    rng: ring::rand::SystemRandom,
    journal: rivet_sdk::journal::Journal,
}

impl Passkeys {
    /// The relying party answering at `names` — the certificate's — whose
    /// oldest passkeys were enrolled at `legacy`, keeping its files in `data`.
    pub(crate) fn new(
        data: &Path,
        names: &[String],
        legacy: &str,
        journal: rivet_sdk::journal::Journal,
    ) -> Self {
        Self {
            names: names.iter().map(|n| n.to_ascii_lowercase()).collect(),
            legacy: legacy.to_ascii_lowercase(),
            data: data.to_path_buf(),
            state: Mutex::new(State::default()),
            rng: ring::rand::SystemRandom::new(),
            journal,
        }
    }

    /// **The name this request arrived at, if the certificate carries it.**
    /// Exactly one `Host` header, compared without its port and without regard
    /// to case. Anything else — no `Host`, two, an address, a name the
    /// certificate does not carry — is `None`, and no ceremony is answered.
    pub(crate) fn name_of(&self, head: &str) -> Option<&str> {
        let mut hosts = rivet_core::cross_site::header_values(head, "host");
        let host = hosts.next()?;
        if hosts.next().is_some() {
            return None;
        }
        let bare = match host.rsplit_once(':') {
            Some((name, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => {
                name
            }
            _ => host,
        };
        self.names
            .iter()
            .find(|n| n.eq_ignore_ascii_case(bare))
            .map(String::as_str)
    }

    /// Whether `stored` belongs to `name`.
    fn at(&self, stored: &store::Stored, name: &str) -> bool {
        stored.enrolled_at(&self.legacy) == name
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        // A poisoned lock means another request panicked holding it, which
        // `panic = "abort"` makes impossible; recovering the data is still the
        // answer that keeps the gate serving.
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn random(&self, n: usize) -> Option<Vec<u8>> {
        let mut out = vec![0u8; n];
        ring::rand::SecureRandom::fill(&self.rng, &mut out).ok()?;
        Some(out)
    }

    /// **Whether the request head carries a live session cookie.** This is
    /// the gate's second way in; a client certificate is the first.
    /// A session counts only at the name it was granted at.
    pub(crate) fn signed_in(&self, head: &str) -> bool {
        let (Some(token), Some(name)) = (session_token(head), self.name_of(head)) else {
            return false;
        };
        let now = Instant::now();
        let mut state = self.state();
        state.sessions.retain(|s| s.expires > now);
        state
            .sessions
            .iter()
            .any(|s| s.name == name && constant_time_eq(s.token.as_bytes(), token.as_bytes()))
    }

    /// A live session at `name` as a `Cookie` pair, for the gate's tests over
    /// TLS.
    #[cfg(test)]
    pub(crate) fn session_for_test(&self, name: &str) -> String {
        self.open_session(name)
            .and_then(|c| c.split(';').next().map(str::to_string))
            .unwrap_or_default()
    }

    fn open_session(&self, name: &str) -> Option<String> {
        let token: String = self
            .random(32)?
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let mut state = self.state();
        let now = Instant::now();
        state.sessions.retain(|s| s.expires > now);
        if state.sessions.len() >= MAX_SESSIONS {
            state.sessions.remove(0);
        }
        state.sessions.push(Session {
            token: token.clone(),
            name: name.to_string(),
            expires: now + SESSION_LIFETIME,
        });
        Some(format!(
            "{COOKIE}={token}; Secure; HttpOnly; SameSite=Strict; Path=/; Max-Age={}",
            SESSION_LIFETIME.as_secs()
        ))
    }

    fn issue_challenge(&self, ceremony: Ceremony, name: &str) -> Option<Vec<u8>> {
        let bytes = self.random(32)?;
        self.hold_challenge(bytes.clone(), ceremony, name);
        Some(bytes)
    }

    /// A challenge with these bytes, as though this proxy had issued it — so
    /// a ceremony captured from a real browser can be answered in a test.
    #[cfg(test)]
    pub(crate) fn plant_login_challenge(&self, bytes: Vec<u8>, name: &str) {
        self.hold_challenge(bytes, Ceremony::Login, name);
    }

    fn hold_challenge(&self, bytes: Vec<u8>, ceremony: Ceremony, name: &str) {
        let mut state = self.state();
        let now = Instant::now();
        state
            .challenges
            .retain(|c| now.duration_since(c.issued) < CHALLENGE_LIFETIME);
        if state.challenges.len() >= MAX_CHALLENGES {
            state.challenges.remove(0);
        }
        state.challenges.push(Challenge {
            bytes,
            ceremony,
            name: name.to_string(),
            issued: now,
        });
    }

    /// Remove and return the live challenge a ceremony answers — single use,
    /// whatever the outcome, and answered only at the name it was issued at.
    fn take_challenge(&self, client_data_json: &[u8], name: &str) -> Option<(Vec<u8>, Ceremony)> {
        let wanted = rivet_webauthn::challenge_of(client_data_json)?;
        let mut state = self.state();
        let at = state.challenges.iter().position(|c| c.bytes == wanted)?;
        let challenge = state.challenges.remove(at);
        (challenge.issued.elapsed() < CHALLENGE_LIFETIME && challenge.name == name)
            .then_some((challenge.bytes, challenge.ceremony))
    }

    fn load(&self) -> Result<Vec<store::Stored>, String> {
        match std::fs::read_to_string(self.data.join(STORE_FILE)) {
            Ok(text) => store::parse(&text).map_err(|e| e.to_string()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(format!("the passkey store could not be read: {e}")),
        }
    }

    fn save(&self, stored: &[store::Stored]) -> Result<(), String> {
        let path = self.data.join(STORE_FILE);
        let temporary = self.data.join(format!(".{STORE_FILE}.new"));
        std::fs::write(&temporary, store::render(stored))
            .and_then(|()| std::fs::rename(&temporary, &path))
            .map_err(|e| format!("the passkey store could not be written: {e}"))
    }

    /// The enrollment code on file, and when it was written — or why not.
    fn code_on_file(&self) -> Result<(String, SystemTime), &'static str> {
        let path = self.data.join(rivet_core::passkey::ENROLLMENT_FILE);
        let (Ok(text), Ok(modified)) = (
            std::fs::read_to_string(&path),
            std::fs::metadata(&path).and_then(|m| m.modified()),
        ) else {
            return Err("no enrollment code is waiting on this machine");
        };
        let lifetime = Duration::from_secs(rivet_core::passkey::ENROLLMENT_LIFETIME_SECONDS);
        // A modification time in the future is treated as expired: it means
        // the clock moved backwards since, and honouring it would stretch the
        // code's life by however far the clock was wrong.
        let fresh = SystemTime::now()
            .duration_since(modified)
            .is_ok_and(|age| age < lifetime);
        if !fresh {
            let _ = std::fs::remove_file(&path);
            self.journal
                .warn("an expired passkey enrollment code was offered, and has been deleted");
            return Err("the enrollment code has expired");
        }
        Ok((text.trim().to_string(), modified))
    }

    fn delete_code(&self) {
        let _ = std::fs::remove_file(self.data.join(rivet_core::passkey::ENROLLMENT_FILE));
    }

    /// The `421` for a request at a name the certificate does not carry.
    fn wrong_name(&self, head: &str) -> Reply {
        Reply::error(
            421,
            &format!(
                "passkeys are answered only at the names this machine's certificate carries \
                 ({}), and this request named {}",
                self.names.join(", "),
                rivet_core::cross_site::header(head, "host").unwrap_or("no host"),
            ),
        )
    }

    /// **Answer one request for a page under [`SIGN_IN_PAGE`]** — `ADR-046`
    /// §4. `rest` is the path after it: empty or `/` for the sign-in page,
    /// `/enroll` for enrollment. Answered only at a name the certificate
    /// carries, because the ceremonies the pages run are answered nowhere else.
    pub(crate) fn sign_in(&self, method: &str, rest: &str, head: &str) -> Reply {
        if self.name_of(head).is_none() {
            return self.wrong_name(head);
        }
        let page = matches!(method, "GET" | "HEAD");
        match rest {
            "" | "/" if page => Reply::html(LOGIN_HTML),
            "/enroll" | "/enroll/" if page => Reply::html(ENROLL_HTML),
            "" | "/" | "/enroll" | "/enroll/" => Reply::error(405, "this page is read with GET"),
            _ => Reply::error(
                404,
                &format!("nothing is served at that path under {SIGN_IN_PAGE}/"),
            ),
        }
    }

    /// **Answer one request under `/.rivet/`.** `rest` is the path after the
    /// prefix, `head` the request head, `body` what followed it.
    pub(crate) fn answer(&self, method: &str, rest: &str, head: &str, body: &[u8]) -> Reply {
        // **Only at a name the certificate carries.** A ceremony's RP ID and
        // origin are this name, so there is nothing to answer at another.
        let Some(name) = self.name_of(head) else {
            return self.wrong_name(head);
        };
        let name = name.to_string();
        match rest {
            "/passkey/login/options" | "/passkey/login" | "/passkey/enroll/options"
            | "/passkey/enroll" | "/logout"
                if method != "POST" =>
            {
                Reply::error(405, "this endpoint takes POST")
            }
            "/passkey/login/options" => self.login_options(&name),
            "/passkey/login" => self.login(&name, body),
            "/passkey/enroll/options" => self.enroll_options(&name, head, body),
            "/passkey/enroll" => self.enroll(&name, head, body),
            "/logout" => self.logout(head),
            _ => Reply::error(404, "nothing is served at that path under /.rivet/"),
        }
    }

    /// The IDs of the passkeys enrolled at `name`, as a JSON list's insides.
    fn ids(&self, stored: &[store::Stored], name: &str) -> String {
        stored
            .iter()
            .filter(|s| self.at(s, name))
            .map(|s| json_string(&b64url(&s.credential.id)))
            .collect::<Vec<_>>()
            .join(",")
    }

    fn login_options(&self, name: &str) -> Reply {
        let stored = match self.load() {
            Ok(stored) => stored,
            Err(why) => return self.broken(&why),
        };
        if !stored.iter().any(|s| self.at(s, name)) {
            return Reply::error(
                409,
                &format!(
                    "no passkey is enrolled at {name} yet, and a passkey belongs to the one name \
                     it was enrolled at. Issue a one-time code over the control plane (cargo \
                     rivet run <machine> -- apply passkey-code confirm) and enroll one at \
                     https://{name}{ENROLL_PAGE}"
                ),
            );
        }
        let Some(challenge) = self.issue_challenge(Ceremony::Login, name) else {
            return self.broken("no random bytes for a challenge");
        };
        Reply::json(
            200,
            format!(
                "{{\"challenge\":{},\"rpId\":{},\"allowCredentials\":[{}],\
                 \"userVerification\":\"required\",\"timeout\":60000}}",
                json_string(&b64url(&challenge)),
                json_string(name),
                self.ids(&stored, name),
            ),
        )
    }

    fn login(&self, name: &str, body: &[u8]) -> Reply {
        let answer = match rivet_webauthn::browser::assertion(body) {
            Ok(answer) => answer,
            Err(why) => return Reply::error(400, &why.to_string()),
        };
        let Some((challenge, Ceremony::Login)) =
            self.take_challenge(&answer.client_data_json, name)
        else {
            return self.refused(
                "sign-in",
                name,
                "the challenge is unknown, used, expired or issued at another name; start again",
            );
        };
        let mut stored = match self.load() {
            Ok(stored) => stored,
            Err(why) => return self.broken(&why),
        };
        let legacy = self.legacy.as_str();
        let Some(entry) = stored
            .iter_mut()
            .find(|s| s.credential.id == answer.credential_id)
        else {
            return self.refused("sign-in", name, "that passkey is not enrolled on this machine");
        };
        if entry.enrolled_at(legacy) != name {
            let why = format!(
                "that passkey was enrolled at {}, and a passkey belongs to the one name it was \
                 enrolled at; sign in there, or enroll one here",
                entry.enrolled_at(legacy)
            );
            return self.refused("sign-in", name, &why);
        }
        let origin = RelyingParty::origin_for(name);
        let rp = RelyingParty { id: name, origin: &origin };
        match rivet_webauthn::verify_assertion(
            rp,
            UserVerification::Required,
            &challenge,
            &entry.credential,
            &answer.credential_id,
            &answer.client_data_json,
            &answer.authenticator_data,
            &answer.signature,
        ) {
            Ok(asserted) => {
                if asserted.sign_count != entry.credential.sign_count {
                    entry.credential.sign_count = asserted.sign_count;
                    if let Err(why) = self.save(&stored) {
                        return self.broken(&why);
                    }
                }
                let Some(cookie) = self.open_session(name) else {
                    return self.broken("no random bytes for a session");
                };
                self.journal.emit(
                    rivet_sdk::journal::Severity::Info,
                    "an operator signed in with a passkey",
                    &[
                        ("credential", short_id(&answer.credential_id)),
                        ("name", rivet_sdk::journal::Value::Str(name.to_string())),
                    ],
                );
                Reply::json(200, "{\"ok\":true}".to_string()).with_cookie(cookie)
            }
            Err(refusal @ rivet_webauthn::Refusal::CounterRegression { .. }) => {
                self.journal.emit(
                    rivet_sdk::journal::Severity::Warn,
                    "a passkey's signature counter did not increase, which is WebAuthn's sign of \
                     a cloned authenticator; the sign-in was refused",
                    &[
                        ("credential", short_id(&answer.credential_id)),
                        ("name", rivet_sdk::journal::Value::Str(name.to_string())),
                        ("why", rivet_sdk::journal::Value::Str(refusal.to_string())),
                    ],
                );
                Reply::error(403, &refusal.to_string())
            }
            Err(refusal) => self.refused("sign-in", name, &refusal.to_string()),
        }
    }

    fn enroll_options(&self, name: &str, head: &str, body: &[u8]) -> Reply {
        let code = match rivet_webauthn::browser::enrollment_code(body) {
            Ok(code) => code,
            Err(why) => return Reply::error(400, &why.to_string()),
        };
        // `signed_in` counts a session only at the name it was granted at, so
        // a session at one name never enrolls a passkey at another.
        let authority = if self.signed_in(head) {
            Authority::Session
        } else {
            let Some(typed) = code else {
                return Reply::error(
                    403,
                    &format!(
                        "enrolling a passkey needs this machine's one-time code, or a browser \
                         already signed in with a passkey at {name}"
                    ),
                );
            };
            match self.check_code(&typed) {
                Ok(modified) => Authority::Code(modified),
                Err(reply) => return reply,
            }
        };
        let stored = match self.load() {
            Ok(stored) => stored,
            Err(why) => return self.broken(&why),
        };
        let (Some(challenge), Some(user)) = (
            self.issue_challenge(Ceremony::Register(authority), name),
            self.random(16),
        ) else {
            return self.broken("no random bytes for a challenge");
        };
        Reply::json(
            200,
            format!(
                "{{\"challenge\":{},\"rp\":{{\"id\":{id},\"name\":{id}}},\
                 \"user\":{{\"id\":{},\"name\":{},\"displayName\":{}}},\
                 \"pubKeyCredParams\":[{{\"type\":\"public-key\",\"alg\":-8}},\
                 {{\"type\":\"public-key\",\"alg\":-7}}],\
                 \"authenticatorSelection\":{{\"residentKey\":\"preferred\",\
                 \"userVerification\":\"required\"}},\"attestation\":\"none\",\
                 \"excludeCredentials\":[{}],\"timeout\":60000}}",
                json_string(&b64url(&challenge)),
                json_string(&b64url(&user)),
                json_string(&format!("operator@{name}")),
                json_string(&format!("Operator of {name}")),
                self.ids(&stored, name),
                id = json_string(name),
            ),
        )
    }

    /// Check a typed code against the one on file, counting wrong ones.
    fn check_code(&self, typed: &str) -> Result<SystemTime, Reply> {
        let (stored, modified) = self.code_on_file().map_err(|why| {
            Reply::error(
                403,
                &format!(
                    "{why}. Issue one over the control plane: cargo rivet run <machine> -- apply \
                     passkey-code confirm"
                ),
            )
        })?;
        if rivet_core::passkey::matches(&stored, typed) {
            return Ok(modified);
        }
        let mut state = self.state();
        let count = match state.wrong {
            Some((when, n)) if when == modified => n + 1,
            _ => 1,
        };
        state.wrong = Some((modified, count));
        drop(state);
        let limit = rivet_core::passkey::ENROLLMENT_ATTEMPTS;
        if count >= limit {
            self.delete_code();
            self.journal.warn(&format!(
                "the passkey enrollment code was offered wrongly {count} times and has been \
                 deleted; issue another to enroll"
            ));
            return Err(Reply::error(
                403,
                "that is not the code, and it has now been guessed at too often: it is deleted. \
                 Issue another over the control plane",
            ));
        }
        self.journal.warn(&format!(
            "a wrong passkey enrollment code was offered ({count} of {limit})"
        ));
        Err(Reply::error(
            403,
            &format!("that is not the code issued ({} tries left)", limit - count),
        ))
    }

    fn enroll(&self, name: &str, head: &str, body: &[u8]) -> Reply {
        let answer = match rivet_webauthn::browser::registration(body) {
            Ok(answer) => answer,
            Err(why) => return Reply::error(400, &why.to_string()),
        };
        let Some((challenge, Ceremony::Register(authority))) =
            self.take_challenge(&answer.client_data_json, name)
        else {
            return self.refused(
                "enrollment",
                name,
                "the challenge is unknown, used, expired or issued at another name; start again",
            );
        };
        let origin = RelyingParty::origin_for(name);
        let credential = match rivet_webauthn::verify_registration(
            RelyingParty { id: name, origin: &origin },
            UserVerification::Required,
            &challenge,
            &answer.client_data_json,
            &answer.attestation_object,
        ) {
            Ok(credential) => credential,
            Err(refusal) => return self.refused("enrollment", name, &refusal.to_string()),
        };
        match authority {
            Authority::Code(modified) => match self.code_on_file() {
                Ok((_, now)) if now == modified => {}
                _ => {
                    return self.refused(
                        "enrollment",
                        name,
                        "the enrollment code was withdrawn, replaced or expired during the \
                         ceremony",
                    );
                }
            },
            Authority::Session => {
                if !self.signed_in(head) {
                    return self.refused("enrollment", name, "the session ended during the ceremony");
                }
            }
        }
        let mut stored = match self.load() {
            Ok(stored) => stored,
            Err(why) => return self.broken(&why),
        };
        if stored.iter().any(|s| s.credential.id == credential.id) {
            return self.refused("enrollment", name, "that passkey is already enrolled");
        }
        let id = credential.id.clone();
        let algorithm = credential.algorithm.word();
        stored.push(store::Stored {
            credential,
            enrolled: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
            rp_id: Some(name.to_string()),
        });
        if let Err(why) = self.save(&stored) {
            return self.broken(&why);
        }
        if let Authority::Code(_) = authority {
            self.delete_code();
        }
        self.journal.emit(
            rivet_sdk::journal::Severity::Info,
            "a passkey was enrolled",
            &[
                ("credential", short_id(&id)),
                ("algorithm", rivet_sdk::journal::Value::Str(algorithm.to_string())),
                (
                    "authority",
                    rivet_sdk::journal::Value::Str(
                        match authority {
                            Authority::Code(_) => "one-time code",
                            Authority::Session => "signed-in session",
                        }
                        .to_string(),
                    ),
                ),
                ("name", rivet_sdk::journal::Value::Str(name.to_string())),
                (
                    "enrolled",
                    rivet_sdk::journal::Value::Int(i64::try_from(stored.len()).unwrap_or(i64::MAX)),
                ),
            ],
        );
        let Some(cookie) = self.open_session(name) else {
            return self.broken("no random bytes for a session");
        };
        Reply::json(200, "{\"ok\":true}".to_string()).with_cookie(cookie)
    }

    fn logout(&self, head: &str) -> Reply {
        if let Some(token) = session_token(head) {
            self.state().sessions.retain(|s| s.token != token);
        }
        Reply::json(200, "{\"ok\":true}".to_string()).with_cookie(format!(
            "{COOKIE}=; Secure; HttpOnly; SameSite=Strict; Path=/; Max-Age=0"
        ))
    }

    fn refused(&self, what: &str, name: &str, why: &str) -> Reply {
        self.journal.emit(
            rivet_sdk::journal::Severity::Warn,
            &format!("a passkey {what} was refused"),
            &[
                ("name", rivet_sdk::journal::Value::Str(name.to_string())),
                ("why", rivet_sdk::journal::Value::Str(why.to_string())),
            ],
        );
        Reply::error(403, why)
    }

    fn broken(&self, why: &str) -> Reply {
        self.journal.error(why);
        Reply::error(500, why)
    }
}

/// The first eight characters of a credential ID, for a journal record that
/// says which passkey without printing a whole identifier.
fn short_id(id: &[u8]) -> rivet_sdk::journal::Value {
    rivet_sdk::journal::Value::Str(b64url(id).chars().take(8).collect())
}

/// The session token in the head's `Cookie` headers, if one is there.
pub(crate) fn session_token(head: &str) -> Option<String> {
    rivet_core::cross_site::header_values(head, "cookie")
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, _)| *name == COOKIE)
        .map(|(_, value)| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// **`Cookie` without this proxy's session.** A backend never needs it, and
/// one that saw it could replay it — a public route's application included,
/// since a `Path=/` cookie is sent to every path on the origin. `None` when
/// nothing else was in the header.
pub(crate) fn without_session(cookie_value: &str) -> Option<String> {
    let kept: Vec<&str> = cookie_value
        .split(';')
        .map(str::trim)
        .filter(|pair| !pair.is_empty())
        .filter(|pair| pair.split_once('=').map_or(*pair, |(name, _)| name).trim() != COOKIE)
        .collect();
    (!kept.is_empty()).then(|| kept.join("; "))
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

#[cfg(test)]
#[path = "passkey_tests.rs"]
mod tests;
