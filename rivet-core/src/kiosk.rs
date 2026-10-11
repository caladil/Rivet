//! **The kiosk control socket** — what `rivet-launcher` may ask the supervisor.
//!
//! `ADR-036` §5.4, ruled 2026-09-01 (`.rulings/` R-364): the launcher is given a
//! narrow socket of its own, and the ability to restart the device is added to
//! what that socket allows.
//!
//! # Why this is not `ADR-032` §3.2's query socket
//!
//! That socket is the operator's whole control surface — status, service
//! control, and whatever §6.3.1 admits next. It is `0600 root:root`, and the
//! design that put the launcher's requests on it was written when the launcher
//! was going to be an image service running as root.
//!
//! `§104.36.1` ruled the launcher an **application**, so it runs as its own uid
//! and cannot open that socket — and that is the good outcome rather than the
//! obstacle. Granting an application the operator's surface would let a kiosk
//! menu install payloads and rewrite the document. This socket exists so the
//! grant can be **exactly three verbs wide**.
//!
//! # ~~Three~~ five verbs, and why each is here
//!
//! | verb | what it asks | why a launcher needs it |
//! | :-- | :-- | :-- |
//! | `select <name>` | give the screen to this application | it is the menu |
//! | `return` | take the screen back | the way out of an application |
//! | `restart` | reboot the machine | ruled: a kiosk needs to offer it |
//! | `health <name>` | is it up, and is it flapping | added 2026-09-01, `PLAN.md` §108.2 |
//! | `offer` | **what is on the menu, and what is the machine for** | added 2026-09-05, `ADR-042` §8.2 |
//!
//! **What is deliberately absent** is everything else: no install, no document
//! edit, no arbitrary service control, no power-off. A launcher that could stop
//! `rivet-mcpd` could lock an operator out of the board it is running on.
//!
//! # `offer` is the verb `cmd/rivet-launcher`'s `offered` module was written to
//! be deleted by
//!
//! That module read `/state/config/machine.toml` and walked `/state/app`
//! itself, and said so in its own header: *"A `get kiosk` on the control socket
//! is where this belongs. That verb does not exist yet, and this is written to
//! be deleted when it does."*
//!
//! **`ADR-042`'s Landlock ruleset is what made it exist.** A confined
//! application cannot read the operator's document or another application's
//! installed files, and on the first confined boot the launcher was refused
//! both: no `[kiosk]` table, no menu, and `auto-run` never honoured. The two
//! ways out were a hole in the policy for the one application that is also the
//! arbiter, or this. The supervisor already holds both answers — it parses the
//! document, and `display_offered` is the same *enabled AND declares a display
//! AND is not the launcher* rule the launcher was computing for itself — so
//! this verb removes a second reader (`ADR-011`) as well as a second grant.
//!
//! **And it closes `.rulings/` R-137's fail-open by making it unreachable.** A
//! launcher that cannot read the document cannot mis-read it: `locked` now
//! comes from the supervisor's own parse, which is the parse that decides
//! everything else about the machine. R-137 chose reachability over a lock that
//! survives a corrupt file, and it is not overturned — a document that will not
//! parse still leaves the supervisor reporting the mode it last knew — but
//! there is no longer a *second* reader that can fail differently from the
//! first.
//!
//! `poweroff` is absent because it was not asked for, not because it was
//! rejected — an appliance menu that can restart and not shut down is odd, and
//! the day it is wanted it is one variant here and one arm in the supervisor.
//!
//! # The line protocol, and why it is not the operation machinery
//!
//! One line in, one line out, `\n`-terminated ASCII. `crate::operation` is the
//! shape for an **operator-facing** subject: it carries a schema, a CLI verb and
//! an MCP exposure, none of which this has or should. This surface has exactly
//! one caller, which is a program in this repository, and both halves are parsed
//! from the same module — so the wire form is an implementation detail that a
//! test holds in place rather than a published contract.

/// Where the socket lives.
///
/// Beside `ADR-032` §3.2's query socket and deliberately not it — see the module
/// note. `/run` is a tmpfs, so this cannot outlive a boot, which is correct: a
/// grant made to a uid is only meaningful for as long as that application is
/// the one running.
pub const SOCKET_PATH: &str = "/run/rivet/kiosk";

/// The provider name an application names to be granted this socket.
///
/// `ADR-015` §5.3's `connects` is the request — the launcher declares
/// `connects = ["rivet-sysd:kiosk"]` and the supervisor chowns the socket to
/// its uid, as it does for an application-to-application endpoint
/// (`cmd/rivet-sysd::endpoints`). **One launcher, not several**: the machine
/// document's `[kiosk] launcher` names the one service whose request is
/// honoured, and every other service declaring it is refused by name.
pub const PROVIDER: &str = "rivet-sysd";

/// The endpoint name, the other half of `connects = ["rivet-sysd:kiosk"]`.
pub const ENDPOINT: &str = "kiosk";

/// The most a request may be. A line longer than this is refused unread.
///
/// A service name is bounded far below this (`crate::service::ServiceName`), so
/// the only thing that reaches it is a caller that is confused or hostile, and
/// both are answered the same way.
pub const REQUEST_MAX_BYTES: usize = 256;

/// The most an **answer** may be, which is not the same number.
///
/// **Added 2026-09-05, and it repairs something that was one application away
/// from being live.** `cmd/rivet-launcher`'s `ask` read
/// [`REQUEST_MAX_BYTES`]` + 1` bytes back off the socket, so every answer was
/// truncated at 257 — and [`Answer::Refused`] carries the supervisor's own
/// sentence, which for `select` on an unoffered name **quotes the whole offered
/// set**. Measured rather than asserted: that refusal is 253 bytes on a machine
/// offering five applications and 231 on one offering three, so today's board
/// fits and a sixth application does not. The truncation would arrive as a
/// refusal that stops mid-word, on a machine somebody had just added something
/// to.
///
/// [`Answer::Offer`] is what forced the number to be thought about rather than
/// left at the edge: it carries a list whose length *is* the number of
/// applications on the machine, so the answer grows without the message
/// changing. Four kilobytes is about sixty service names at the maximum length
/// `crate::service::ServiceName` admits, which is far past any kiosk, and it is
/// still a bound rather than a read to EOF.
pub const ANSWER_MAX_BYTES: usize = 4096;

/// What the launcher may ask.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Give the screen to this application.
    Select { name: String },
    /// Take the screen back; run nothing.
    Return,
    /// Reboot the machine.
    Restart,
    /// **How is this application doing?** — `PLAN.md` §108.2.
    ///
    /// The only verb here that READS. A locked kiosk has to know whether the
    /// one application it may show is up, and whether it keeps going down —
    /// and **the supervisor already counts that** (`Managed::attempts`). The
    /// launcher asking is the alternative to the launcher keeping a second
    /// count of the same restarts, which is the drift `§107.8` was spent
    /// removing from the action path.
    Health { name: String },
    /// **What is on the menu, and what is this machine for** — `ADR-042` §8.2.
    ///
    /// Added 2026-09-05. Reads, like [`Self::Health`], and takes no argument:
    /// there is one launcher on a machine and one answer to give it. It
    /// replaces the launcher reading the operator's document and walking
    /// `/state/app`, neither of which a confined application may do.
    Offer,
}

/// Why a request could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestError {
    /// A verb this build does not define.
    UnknownVerb,
    /// `select` with no name, or a name that is not one.
    BadName,
    /// Longer than [`REQUEST_MAX_BYTES`], or not text.
    TooLong,
}

impl core::fmt::Display for RequestError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnknownVerb => write!(
                f,
                "this socket takes `select <name>`, `return`, `restart`, `health \
                 <name>` and `offer`, and nothing else. It is deliberately narrow: the \
                 operator's control surface is ADR-032 §3.2's query socket and an \
                 application is not given that. `health` and `offer` are the two that read \
                 rather than act"
            ),
            Self::BadName => write!(
                f,
                "`select` takes the name of an application, and a service name is what \
                 rivet_core::service::ServiceName admits"
            ),
            Self::TooLong => write!(f, "a request is at most {REQUEST_MAX_BYTES} bytes of text"),
        }
    }
}

impl Request {
    /// Read one request off the wire.
    #[must_use = "a request that is not answered leaves the caller waiting"]
    pub fn parse(line: &str) -> Result<Self, RequestError> {
        if line.len() > REQUEST_MAX_BYTES {
            return Err(RequestError::TooLong);
        }
        let line = line.trim_end_matches(['\r', '\n']);
        let (verb, rest) = line.split_once(' ').unwrap_or((line, ""));
        match verb {
            "select" => {
                let name = rest.trim();
                // Validated HERE and not by the supervisor, on `ADR-011`: the
                // wire is where a value stops being text, and a name that got
                // past this would be interpolated into a path by whatever acted
                // on it.
                if crate::service::ServiceName::new(name).is_none() {
                    return Err(RequestError::BadName);
                }
                Ok(Self::Select {
                    name: name.to_owned(),
                })
            }
            "health" => {
                let name = rest.trim();
                // Validated here for `select`'s reason: the wire is where a
                // value stops being text.
                if crate::service::ServiceName::new(name).is_none() {
                    return Err(RequestError::BadName);
                }
                Ok(Self::Health {
                    name: name.to_owned(),
                })
            }
            "return" if rest.trim().is_empty() => Ok(Self::Return),
            "restart" if rest.trim().is_empty() => Ok(Self::Restart),
            "offer" if rest.trim().is_empty() => Ok(Self::Offer),
            _ => Err(RequestError::UnknownVerb),
        }
    }

    /// Write one request, as the launcher sends it.
    #[must_use]
    pub fn render(&self) -> String {
        match self {
            Self::Select { name } => format!("select {name}\n"),
            Self::Return => "return\n".to_owned(),
            Self::Restart => "restart\n".to_owned(),
            Self::Health { name } => format!("health {name}\n"),
            Self::Offer => "offer\n".to_owned(),
        }
    }
}

/// What the supervisor says back.
///
/// **A sentence and not a code.** The only caller is a program that prints it
/// into the journal, and a refusal that a person reads is worth more than one a
/// program branches on.
///
/// **~~nothing here has a decision to make on the answer~~ — struck 2026-09-01,
/// `PLAN.md` §106.4.** That was true until the launcher started cutting a
/// display lease *before* asking. It now has two decisions on the answer, and
/// both are wrong by default: whether to stop drawing, and whether to revoke a
/// lease it has already handed over. Taking the sentence at its word cost a
/// frozen screen and a leaked lease on the first hardware test.
///
/// The variants were always distinguishable; only the advice was wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// Taken. The convergence that acts on it runs after this is written.
    Taken { what: String },
    /// Not taken, and why.
    Refused { why: String },
    /// The answer to [`Request::Health`] — `PLAN.md` §108.2.
    ///
    /// **Structured, where the other two are a sentence**, because this one is
    /// read by a program that has to decide whether to keep waiting or to put
    /// an error on a screen. A refusal a person reads is worth more than a code
    /// when nothing branches on it; here something does.
    Health {
        /// Whether it is running right now.
        running: bool,
        /// How many times the supervisor has restarted it after a failure.
        ///
        /// **Not a monotonic count of starts** - the supervisor clears it once
        /// a service has stayed up long enough to count as healthy - so it says
        /// *is this thing flapping*, and it cannot say *is this a different
        /// process from the one I handed a display to*.
        attempts: u32,
        /// What is running, if anything.
        ///
        /// **The instance signal**, and the reason this field exists. Found on
        /// the board: stopping a locked application let the supervisor restart
        /// it faster than the launcher polls, so the launcher saw `running:
        /// true`, did nothing, and left an application up and BLIND - a display
        /// lease reaches a process once, at `execve`. A changed pid is that
        /// event and nothing else reports it.
        pid: Option<u32>,
    },
    /// **The answer to [`Request::Offer`]** — `ADR-042` §8.2, 2026-09-05.
    ///
    /// Everything the launcher used to read out of `/state/config/machine.toml`
    /// and `/state/app`, from the supervisor that already holds it.
    Offer {
        /// `kiosk`, `headless` or `unknown` — the supervisor's own word,
        /// carried through unchanged (`AGENTS.md` §5).
        ///
        /// **`unknown` is the one that matters to a launcher** and is why this
        /// field is here at all. A supervisor that was never told what the
        /// document says reports it, rather than reporting `headless` and
        /// leaving a machine that might be locked looking definitely unlocked
        /// (`crate::operation::status::KioskMode::unknown`).
        mode: String,
        /// `[kiosk] locked`.
        locked: bool,
        /// `[kiosk] auto-run`, where the table names one.
        auto_run: Option<String>,
        /// **What may be given the screen**, by name, in the supervisor's
        /// order.
        ///
        /// The supervisor's `display_offered`: enabled by the document, a
        /// manifest that declares `devices = ["display"]`, and not the launcher
        /// itself. **The same set `select` validates against**, which is the
        /// property that makes this worth a verb — the launcher used to compute
        /// its own answer from the same two facts and could disagree, and a
        /// menu offering something the supervisor then refuses is a menu that
        /// lies.
        offered: Vec<String>,
    },
}

/// What an absent or empty field is written as, in a format with no quoting.
///
/// A service name can never be this — `crate::service::ServiceName` requires a
/// leading ASCII letter or digit — so it cannot be mistaken for one.
const NOTHING: &str = "-";

impl Answer {
    #[must_use]
    pub fn render(&self) -> String {
        match self {
            Self::Taken { what } => format!("ok {what}\n"),
            Self::Health {
                running,
                attempts,
                pid,
            } => format!(
                "health {} {attempts} {}\n",
                if *running { "running" } else { "stopped" },
                pid.map_or_else(|| NOTHING.to_owned(), |p| p.to_string())
            ),
            // **Four positional fields, and the list is last** because it is
            // the only one whose length varies. Every value in the first three
            // is a single token by construction — the three mode words are
            // asserted below, and `auto-run` is a service name — so there is
            // nothing here that needs quoting, which is the same argument
            // `crate::settings`'s format makes.
            Self::Offer {
                mode,
                locked,
                auto_run,
                offered,
            } => format!(
                "offer {mode} {} {} {}\n",
                if *locked { "locked" } else { "unlocked" },
                auto_run.as_deref().unwrap_or(NOTHING),
                if offered.is_empty() {
                    NOTHING.to_owned()
                } else {
                    offered.join(",")
                }
            ),
            Self::Refused { why } => format!("refused {why}\n"),
        }
    }

    /// Read an answer, for the launcher's side.
    #[must_use]
    pub fn parse(line: &str) -> Self {
        let line = line.trim_end_matches(['\r', '\n']);
        // **`health` first, because it is the one with structure.**
        //
        // Until 2026-09-01 this function did not know the verb existed, so a
        // health report fell through to `Refused` and a launcher watching a
        // perfectly healthy application put an ERROR SCREEN on the machine. An
        // answer nobody can parse is worse than one nobody sent.
        // **`offer` before `ok`, for `health`'s reason exactly.** An answer
        // this end cannot parse falls through to `Refused`, and a launcher
        // told its own menu was refused would put an error screen on a machine
        // that is working.
        if let Some(rest) = line.strip_prefix("offer ") {
            let mut words = rest.split_whitespace();
            let mode = words.next().unwrap_or("unknown").to_owned();
            let locked = words.next() == Some("locked");
            let auto_run = match words.next() {
                None | Some(NOTHING) => None,
                Some(name) => Some(name.to_owned()),
            };
            let offered = match words.next() {
                None | Some(NOTHING) => Vec::new(),
                Some(list) => list
                    .split(',')
                    .filter(|n| !n.is_empty())
                    .map(str::to_owned)
                    .collect(),
            };
            return Self::Offer {
                mode,
                locked,
                auto_run,
                offered,
            };
        }
        if let Some(rest) = line.strip_prefix("health ") {
            let mut words = rest.split_whitespace();
            let running = words.next() == Some("running");
            let attempts = words.next().and_then(|w| w.parse().ok()).unwrap_or(0);
            let pid = words.next().and_then(|w| w.parse().ok());
            return Self::Health {
                running,
                attempts,
                pid,
            };
        }
        line.strip_prefix("ok ").map_or_else(
            || Self::Refused {
                why: line.strip_prefix("refused ").unwrap_or(line).to_owned(),
            },
            |what| Self::Taken {
                what: what.to_owned(),
            },
        )
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;

    #[test]
    fn the_three_verbs_round_trip() {
        for request in [
            Request::Select {
                name: "aura-renderer".to_owned(),
            },
            Request::Return,
            Request::Restart,
            Request::Offer,
            Request::Health {
                name: "aura-renderer".to_owned(),
            },
        ] {
            assert_eq!(Request::parse(&request.render()), Ok(request.clone()), "{request:?}");
        }
    }

    /// **The narrowness is the point**, so it is asserted rather than assumed.
    /// Every one of these is a verb the operator's socket has or might grow, and
    /// none of them is reachable from here.
    #[test]
    fn nothing_but_the_three_verbs_is_accepted() {
        for line in [
            "status\n",
            "install /state/payload.rvt\n",
            "service-restart rivet-mcpd\n",
            "edit /state/config/machine.toml\n",
            "poweroff\n",
            "reload\n",
            "\n",
            "SELECT aura\n",
            // **`get kiosk` is what the launcher's own module said it was
            // waiting for, and the verb is spelled `offer`.** A caller that
            // guessed the other spelling is refused rather than answered.
            "get kiosk\n",
        ] {
            assert_eq!(
                Request::parse(line),
                Err(RequestError::UnknownVerb),
                "{line:?} was accepted"
            );
        }
    }

    /// A name is validated on the wire, because whatever acts on it will put it
    /// in a path.
    #[test]
    fn a_select_name_is_a_service_name_or_it_is_refused() {
        for bad in [
            "select \n",
            "select ../../etc/passwd\n",
            "select has space\n",
            "select /absolute\n",
            "select .leading-dot\n",
        ] {
            assert_eq!(Request::parse(bad), Err(RequestError::BadName), "{bad:?}");
        }
        // **Uppercase IS a valid service name** and is accepted here, which
        // is `ServiceName::RULE`'s answer and not this module's. A test that
        // refused it would be this socket inventing a second, stricter idea
        // of what a service is called -- the two-authority failure ADR-011
        // names. Written down because the first draft of this test asserted
        // the opposite and the rule, not the code, was wrong.
        assert!(Request::parse("select UPPER\n").is_ok());
        assert!(Request::parse("select kiosk-demo\n").is_ok());
    }

    /// `return` and `restart` take no argument, and one that arrives with a
    /// word after it is a caller that means something else.
    #[test]
    fn a_bare_verb_with_an_argument_is_not_that_verb() {
        assert_eq!(Request::parse("return now\n"), Err(RequestError::UnknownVerb));
        assert_eq!(
            Request::parse("restart --force\n"),
            Err(RequestError::UnknownVerb)
        );
    }

    #[test]
    fn a_request_longer_than_the_cap_is_refused_before_it_is_read() {
        let long = format!("select {}\n", "x".repeat(REQUEST_MAX_BYTES));
        assert_eq!(Request::parse(&long), Err(RequestError::TooLong));
    }

    /// `offer` takes no argument, and one that arrives with a word after it is
    /// a caller that means something else — the same rule `return` obeys.
    #[test]
    fn offer_takes_no_argument() {
        assert_eq!(Request::parse("offer\n"), Ok(Request::Offer));
        assert_eq!(
            Request::parse("offer everything\n"),
            Err(RequestError::UnknownVerb)
        );
    }

    /// **The wire format has no quoting, so every field must be one token.**
    ///
    /// `AGENTS.md` §4's cheap guard: [`Answer::Offer`]'s `mode` is the
    /// supervisor's own word carried through unchanged, and a mode word with a
    /// space in it would be silently truncated by [`Answer::parse`]. Nothing
    /// today produces one; this fails the day something does, in `rivet-core`,
    /// rather than on a board.
    #[test]
    fn every_mode_word_is_a_single_token() {
        for mode in [
            crate::operation::status::MODE_KIOSK,
            crate::operation::status::MODE_HEADLESS,
            crate::operation::status::MODE_UNKNOWN,
        ] {
            assert_eq!(
                mode.split_whitespace().count(),
                1,
                "`{mode}` is more than one token, and rivet_core::kiosk's answer format has no \
                 quoting — a launcher would read only its first word"
            );
        }
    }

    /// **The offer answer round-trips, in every shape a machine produces.**
    ///
    /// `ADR-042` §8.2. The empty list and the absent `auto-run` are the two
    /// that matter: both are written `-`, and a service name can never be `-`,
    /// so neither can be confused for one.
    #[test]
    fn an_offer_answer_round_trips() {
        for answer in [
            Answer::Offer {
                mode: "kiosk".to_owned(),
                locked: true,
                auto_run: Some("aura-renderer".to_owned()),
                offered: vec!["aura-renderer".to_owned(), "kiosk-demo".to_owned()],
            },
            Answer::Offer {
                mode: "kiosk".to_owned(),
                locked: false,
                auto_run: None,
                offered: vec!["kiosk-demo".to_owned()],
            },
            // A machine with a screen and nothing installed to put on it.
            Answer::Offer {
                mode: "headless".to_owned(),
                locked: false,
                auto_run: None,
                offered: Vec::new(),
            },
            // **A supervisor that was never told**, which must not read as
            // `headless, unlocked` at the other end.
            Answer::Offer {
                mode: "unknown".to_owned(),
                locked: false,
                auto_run: Some("aura-renderer".to_owned()),
                offered: vec!["aura-renderer".to_owned()],
            },
        ] {
            assert_eq!(Answer::parse(&answer.render()), answer, "{answer:?}");
        }
    }

    /// **An offer answer is not a refusal**, which is the failure `health` had
    /// on 2026-09-01: a verb the parser did not know fell through to `Refused`
    /// and the launcher put an error screen on a working machine.
    #[test]
    fn an_offer_answer_is_never_read_as_a_refusal() {
        let rendered = Answer::Offer {
            mode: "kiosk".to_owned(),
            locked: true,
            auto_run: Some("aura-renderer".to_owned()),
            offered: vec!["aura-renderer".to_owned()],
        }
        .render();
        assert!(
            !matches!(Answer::parse(&rendered), Answer::Refused { .. }),
            "the launcher would show a fault screen for its own menu"
        );
    }

    /// **Every answer fits in [`ANSWER_MAX_BYTES`], and a real refusal does not
    /// reliably fit in the cap that was being used.**
    ///
    /// The launcher read `REQUEST_MAX_BYTES + 1` = 257 bytes back. A refusal
    /// from `select_display` quotes the whole offered set, so its length is a
    /// function of how many applications the machine has: the fixture below is
    /// the supervisor's own sentence with six names and it is over the old cap.
    /// This asserts the new bound holds the widest thing this module can
    /// produce, and that the old one did not.
    #[test]
    fn an_answer_bound_has_to_hold_a_refusal_and_a_full_menu() {
        let offered: Vec<String> = (0..40).map(|i| format!("application-{i:02}")).collect();
        let full = Answer::Offer {
            mode: "kiosk".to_owned(),
            locked: false,
            auto_run: Some("application-00".to_owned()),
            offered,
        }
        .render();
        assert!(
            full.len() <= ANSWER_MAX_BYTES,
            "a forty-application menu is {} bytes and the cap is {ANSWER_MAX_BYTES}",
            full.len()
        );
        // **The supervisor's own sentence**, copied from `select_display`,
        // with the offered set this project's own test board actually reports.
        let long_refusal = Answer::Refused {
            why: "`x` is not offered a screen on this machine. An application is offered when \
                  the document enables it AND its manifest declares devices = [\"display\"] \
                  (ADR-015 §6.2). Offered here: aura-renderer, kiosk-demo, template-kiosk, \
                  mesa-demo, sqlite-demo, photo-index"
                .to_owned(),
        }
        .render();
        assert!(
            long_refusal.len() > REQUEST_MAX_BYTES + 1,
            "this fixture is {} bytes and is meant to be longer than the {} the launcher used to \
             read with; if it is not, the assertion below proves nothing",
            long_refusal.len(),
            REQUEST_MAX_BYTES + 1
        );
        assert!(long_refusal.len() <= ANSWER_MAX_BYTES);
    }

    #[test]
    fn an_answer_round_trips() {
        let taken = Answer::Taken {
            what: "kiosk-demo holds the screen".to_owned(),
        };
        assert_eq!(Answer::parse(&taken.render()), taken);
        let refused = Answer::Refused {
            why: "no such application is offered".to_owned(),
        };
        assert_eq!(Answer::parse(&refused.render()), refused);
    }

    /// The socket is not the operator's, and the constant says so by being a
    /// different path. A build that pointed them at one file would hand an
    /// application the whole control surface.
    #[test]
    fn the_kiosk_socket_is_not_the_query_socket() {
        assert_ne!(SOCKET_PATH, crate::operation::QUERY_SOCKET_PATH);
        assert_ne!(SOCKET_PATH, crate::journal::INGEST_SOCKET_PATH);
    }
}
