//! **Handing a payload to the installer.** The maintainer's 2026-08-24 ruling.
//!
//! `rivet_core::operation::machine::Deployer` is the domain port
//! `rivet apply install` reaches, and this module is the adapter under it.
//!
//! ~~It is the one adapter in this binary that **refuses everything**, on every
//! platform, including a real board … the refusal is the honest state of the
//! system.~~
//!
//! **Superseded 2026-08-24 evening.** That was true from the moment
//! `cmd/rivet-deploy` was built until the maintainer ruled the channel the
//! same evening, and it is struck rather than deleted because it is the shape
//! `AGENTS.md` §1 asks for: the decision was written into `PLAN.md` with options
//! and a recommendation, and it was not guessed at in a commit. What was
//! missing was a ruling; the ruling arrived.
//!
//! # The channel, in four lines
//!
//! ```text
//!   rivet apply install /state/x.rvt confirm
//!        │  writes                                  (this module)
//!        ▼
//!   /state/deploy-request        one line, 0600:
//!        │                       from=<client> [key=<label>] -- /state/x.rvt
//!        │  read on the next convergence pass       (cmd/rivet-sysd)
//!        ▼
//!   definitions::deploy("/state/x.rvt")  →  a service instance, started once
//!        +  a record naming who asked                   (the supervisor's own)
//! ```
//!
//! # It says who asked, and [`this_process`] is the whole of how
//!
//! Added 2026-08-24 under `ADR-028` §4.5. The principal is read out of this
//! process's environment — the control-plane peer that authenticated, as
//! `rivet_core::deploy::PEER_KEY_ENV` carries it — and travels in the request
//! to a record.
//!
//! **It is attribution and not authorisation**, and this module is not the place
//! that argument is made once and forgotten: `rivet_core::deploy::Principal`
//! carries it, the supervisor's record repeats it, and `ADR-028` §4.5 rules it.
//! Nothing here checks anything, nothing here grants anything, and the trust
//! boundary is exactly where it was — root on `/state`.
//!
//! `rivet_core::deploy` owns the path, the mode, the cap and the format,
//! because the writer and the reader are two binaries that never link against
//! one another. `ADR-017` §3.2.2 is the amendment that lets the supervisor start
//! a service it did not know about when it started — the one new idea the
//! ruling needed, and the reason it was a ruling rather than an implementation
//! detail.
//!
//! # `Ok(())` means the request is on the card, and nothing more
//!
//! Not that anything was installed, not that the installer started, not that the
//! payload is any good. `rivet_core::operation::machine::Deployer` states that
//! contract and `ADR-006` §4.2 makes it enforceable: *a request that was not
//! delivered is a refusal, never a result with a `false` in it.* So this returns
//! **before** the 640 MiB write it causes, which is the maintainer's 2026-08-24 ruling
//! whole reason for the installer being a separate supervised program: an
//! operator's session must not own that write.
//!
//! # What is checked here, and what deliberately is not
//!
//! **Checked** — both of them facts about *the machine*, which is exactly what
//! `Deployer::can_request_install` says may be refused here:
//!
//! 1. **`/state` is there.** Without it there is nowhere to put a request and
//!    nothing would ever read one. On a development host this is also what says
//!    *this is not a Rivet machine* — see [`crate::refusal`].
//! 2. **No request is already waiting.** A second `apply install` would
//!    otherwise silently replace the first, and the operator who asked for the
//!    first would be told it was delivered and then watch a different payload
//!    install. The refusal names the payload that is already waiting, which is
//!    the fact they need.
//! 3. **The artifact is not what this machine is already running** — since
//!    2026-09-12, `Requests::not_already_running`. Its first 4 KiB are compared
//!    **as bytes** with the headers retained beside the running slot's build and
//!    each application's `current`; nothing is parsed, and the installer asks
//!    the same question again.
//! 4. **A system update is not about to write the slot an ordinary reset falls
//!    back to** — `Requests::not_uncommitted_trial`, `PLAN-Feature-Set-2026-09-12`
//!    §3. Only a name-derived guess at the artifact's kind, and only ever
//!    silent rather than wrong: an unreadable slot or a name this build did not
//!    compose leaves the question to the installer, which asks it again from the
//!    signed header and fails closed.
//!
//! **Not checked: whether the payload is any good.** Not the signature, not the
//! digests, not the product, not the architecture, and **not even that the file
//! exists**. `ADR-029` §11: three formats means three verifiers each with a
//! third of the scrutiny, and a check here would be a second reader of the
//! container in a process that is not the one performing the install. The
//! installer refuses it, in the journal, under one correlation identifier.
//!
//! A file that is not there is the case worth stating explicitly, because it is
//! the one that looks like an oversight. `rivet apply install /state/typo.rvt
//! confirm` is **accepted**, and `rivet get install` then reports an install
//! that refused the payload. That is one answer from one authority rather than
//! two answers from two, and it is `ADR-029` §11's rule applied where it costs
//! something.
//!
//! # Durability, and the order the two files are touched in
//!
//! A request is written to `rivet_core::deploy::REQUEST_TEMP_PATH`, flushed,
//! and **renamed** onto the real path. The reader is a different process that is
//! not waiting for this one and may look at any moment; `rename(2)` within one
//! directory is atomic, so it sees either no request or a whole one — never a
//! truncated path naming a file nobody asked for.
//!
//! The directory is flushed afterwards, and a failure to flush is **not** a
//! refusal. The request is on the running machine either way and the supervisor
//! will read it within one convergence pass; what an unflushed directory risks
//! is only that a power cut in the next moments loses a request the operator
//! already knows about. Refusing there would report *not delivered* about
//! something that was.

use rivet_core::artifact;
use rivet_core::deploy;
use rivet_core::uninstall;
use rivet_core::uninstall_library;
use rivet_core::operation::{Deployer, Held, MachineError};

/// The filesystem, as this module needs it.
///
/// **An adapter's own I/O seam, not a domain port** — `AGENTS.md` §3.1's
/// distinction. Every method here is in the vocabulary of files and modes, which
/// is why it is named in this crate rather than in `rivet-core`, whose
/// `[dependencies]` table is empty so that the build refuses rather than a human
/// noticing.
///
/// It exists so this module's decisions — a refusal when there is nowhere to
/// write, a refusal when a request is already waiting, the rename, the flush —
/// are reachable from a test on a development host that has no `/state` and must
/// not grow one.
pub trait RequestStore {
    /// Whether a directory is there. Used for one thing: `/state`.
    fn directory_exists(&mut self, path: &str) -> bool;

    /// A file's contents, or `None` if it is not there.
    ///
    /// `Ok(None)` and `Err` are deliberately different: a request that is not
    /// there means *go ahead*, and a request that could not be read means *stop*
    /// — reading the second as the first would overwrite somebody's pending
    /// install on a failing card.
    fn read(&mut self, path: &str) -> Result<Option<String>, String>;

    /// Create or replace a file with `mode`, and flush its contents.
    fn write_private(&mut self, path: &str, bytes: &[u8], mode: u32) -> Result<(), String>;

    /// `rename(2)`, which is atomic within one directory.
    fn rename(&mut self, from: &str, to: &str) -> Result<(), String>;

    /// `fsync(2)` a directory, so a rename survives a power cut.
    fn flush_directory(&mut self, path: &str) -> Result<(), String>;

    /// What a directory holds, one entry per name, in no promised order.
    ///
    /// **Added 2026-09-12 for exactly one caller** — resolving a digest in
    /// `rivet_core::artifact`'s store, which is a directory holding one file
    /// whose name is the artifact's. `Ok(None)` for a directory that is not
    /// there, which is what *this machine does not hold that digest* looks
    /// like; `Err` for one that could not be read, which is a failing card and
    /// is a different answer.
    ///
    /// It is deliberately not a `read_dir` of anything else: this port is the
    /// install and uninstall channels and it stays that. **One more directory
    /// since 2026-09-12**: the application root, so the channel can find each
    /// application's `current` — see [`Requests::not_already_running`].
    fn list_directory(&mut self, path: &str) -> Result<Option<Vec<String>>, String>;

    /// **The directories in `path`, and nothing else** — no file, and no
    /// symbolic link — or `None` when `path` is not there.
    ///
    /// For the library uninstall channel, which reads the library root and the
    /// application tree through `rivet_core::package::closure`. It answers
    /// directories only because that is what the supervisor's own discovery
    /// sees, and the two must agree about which entries are applications and
    /// which are library builds: a reader that also saw an application's
    /// `settings` file would call a directory holding only its data an
    /// application with something installed, and refuse where the supervisor
    /// would not.
    fn list_directories(&mut self, path: &str) -> Result<Option<Vec<String>>, String>;

    /// Whether anything is at `path`, following a link as the supervisor's own
    /// check does. `Err` when the question could not be answered, which is not
    /// the same as `false`.
    fn exists(&mut self, path: &str) -> Result<bool, String>;

    /// **At most the first `len` bytes of a file**, or `None` if it is not there.
    ///
    /// Added 2026-09-12 for [`Requests::not_already_running`], which compares an
    /// artifact's 4 KiB signed header with the headers retained beside what this
    /// machine runs. Bytes rather than text because a header is not UTF-8, and a
    /// bound because an artifact in the store can be 700 MB and only its head is
    /// wanted.
    fn read_head(&mut self, path: &str, len: usize) -> Result<Option<Vec<u8>>, String>;
}

/// The channel: write a request, and let the supervisor pick it up.
pub struct Requests<S: RequestStore> {
    store: S,
    /// Who this process can honestly say is asking. See [`this_process`].
    ///
    /// **Captured once, at construction, from the environment** rather than read
    /// at the point of use. That is the opposite of `AGENTS.md` §5's rule about
    /// the kernel, and the difference is the point: an interface's state is the
    /// kernel's to answer and must never be cached, but *how this process was
    /// invoked* is a fact about a moment that has already passed. Re-reading it
    /// later could only produce a different answer if something rewrote this
    /// process's environment, and the later answer would be the wrong one.
    principal: deploy::Principal,
    /// The A/B slot the firmware says this machine booted, when it said one.
    ///
    /// **Handed in rather than read here**, because this adapter's seam is files
    /// and the firmware's answer is the boot medium's (`crate::boot`), and
    /// **captured at construction**, which is the point of use: `crate::boot`
    /// builds a `Requests` for each call. It is the same reading
    /// `cmd/rivet-deploy` computes its target slot from, so the channel and the
    /// installer agree about which slot is running.
    running_slot: Option<rivet_core::state::Slot>,
    /// **Which slot an ordinary reset would boot** — `crate::boot`'s reading of
    /// the same record `cmd/rivet-deploy`'s [`uncommitted_trial`] compares
    /// against, for [`Requests::not_uncommitted_trial`].
    ///
    /// `None` when nothing asked for it (a caller with no use for the trial
    /// check, or a test); `Some(Err(_))` when the card would not say — and
    /// **never treated as a refusal here**, for the reason
    /// [`Requests::not_already_running`] already gives: everything unreadable is
    /// left to the installer, which fails closed on exactly this question.
    ///
    /// [`uncommitted_trial`]: rivet_core::deploy::uncommitted_trial
    default_slot: Option<Result<rivet_core::state::Slot, String>>,
}

impl<S: RequestStore> Requests<S> {
    pub const fn new(store: S, principal: deploy::Principal) -> Self {
        Self {
            store,
            principal,
            running_slot: None,
            default_slot: None,
        }
    }

    /// The same channel, knowing which slot this machine booted — or that
    /// nothing said, which leaves only applications to compare against.
    #[must_use]
    pub fn running_slot(mut self, slot: Option<rivet_core::state::Slot>) -> Self {
        self.running_slot = slot;
        self
    }

    /// The same channel, knowing which slot an ordinary reset would boot — or
    /// that the card would not say. `PLAN-Feature-Set-2026-09-12` §3.
    #[must_use]
    pub fn default_slot(mut self, default: Option<Result<rivet_core::state::Slot, String>>) -> Self {
        self.default_slot = default;
        self
    }

    /// What was written, for a test in another module of this crate.
    ///
    /// `crate::main`'s whole-binary tests drive `rivet apply install` through
    /// `run` and then have to look at the card; there is no other way for them
    /// to tell a request that was delivered from one that was not.
    #[cfg(test)]
    pub const fn store(&self) -> &S {
        &self.store
    }

    /// Everything that is true of the **machine** rather than of the payload.
    ///
    /// Run on both paths — the plan and the confirmation — which is
    /// `rivet_core::operation::stop`'s argument reused: an agent shown a plan
    /// and then a refusal would reasonably conclude the confirmation was the
    /// problem and re-issue it.
    fn interlock(&mut self) -> Result<(), MachineError> {
        if !self.store.directory_exists(rivet_core::state::STATE_MOUNT_POINT) {
            return Err(MachineError::Refused(format!(
                "REFUSED there is no {state} on this machine, so there is nowhere to put an \
                 install request and NOTHING HAS BEEN DELIVERED. {state} is the writable \
                 partition a Rivet card carries (ADR-018 §4); a machine without one is either \
                 not a Rivet or is one whose /state did not mount, and the journal from this \
                 boot says which.",
                state = rivet_core::state::STATE_MOUNT_POINT,
            )));
        }

        match self.store.read(deploy::REQUEST_PATH) {
            Ok(None) => Ok(()),
            // **A request that is already waiting is not overwritten.** The
            // operator who asked for that one was told it was delivered; making
            // this command replace it silently would make that a lie, and they
            // would watch a payload they did not name install.
            Ok(Some(pending)) => {
                let waiting = deploy::parse_request(&pending).map_or_else(
                    |_| "an unreadable request".to_string(),
                    |request| request.digest.as_str().to_string(),
                );
                Err(MachineError::Refused(format!(
                    "REFUSED {waiting_words} at {path} and NOTHING HAS \
                     BEEN DELIVERED. It names {waiting}. The supervisor takes one up on its next \
                     convergence pass, so this usually clears within moments — `rivet get \
                     install` is how to watch it. If it does not clear, the supervisor is not \
                     running or could not remove it, and this boot's journal says which.",
                    // **The words are `rivet_core`'s**, because a host-side tool
                    // waits this refusal out by recognising them and every other
                    // refusal from this operation means stop.
                    waiting_words = deploy::REQUEST_WAITING,
                    path = deploy::REQUEST_PATH,
                )))
            }
            Err(detail) => Err(MachineError::Refused(format!(
                "REFUSED {path} could not be read, so whether an install is already waiting is \
                 unknown and NOTHING HAS BEEN DELIVERED. Writing over it might have discarded \
                 somebody's install, which is worse than refusing. Detail: {detail}",
                path = deploy::REQUEST_PATH,
            ))),
        }
    }

    /// The same three checks for the uninstall channel.
    ///
    /// **A second function rather than one parameterised by path**, and the
    /// reason is the messages: every sentence above names *an install* and what
    /// happens next to it. A shared body would have to say "a request", which is
    /// the vaguer wording `AGENTS.md` §5 exists to prevent — an operator reading
    /// a refusal should know which of the two channels is blocked.
    /// Whether a repository-delete request can be delivered right now.
    ///
    /// `uninstall_interlock`'s shape and its argument: **one request at a
    /// time**, because the supervisor reads one exact path and writing over a
    /// waiting request would discard somebody's delete without saying so.
    fn repo_delete_interlock(&mut self) -> Result<(), MachineError> {
        if !self
            .store
            .directory_exists(rivet_core::state::STATE_MOUNT_POINT)
        {
            return Err(MachineError::Refused(format!(
                "REFUSED there is no {state} on this machine, so there is nowhere to put a \
                 repository-delete request and NOTHING HAS BEEN DELIVERED.",
                state = rivet_core::state::STATE_MOUNT_POINT,
            )));
        }

        match self.store.read(rivet_core::repo_delete::REQUEST_PATH) {
            Ok(None) => Ok(()),
            Ok(Some(pending)) => {
                let waiting = rivet_core::repo_delete::parse_request(&pending).map_or_else(
                    |_| "an unreadable request".to_string(),
                    |request| request.name.to_string(),
                );
                Err(MachineError::Refused(format!(
                    "REFUSED a repository-delete request for {waiting} is already waiting at \
                     {path} and NOTHING HAS BEEN DELIVERED. Writing over it would discard a \
                     delete somebody asked for, silently. The supervisor takes it within one \
                     convergence pass; try again after that, or read the journal to see what \
                     happened to it.",
                    path = rivet_core::repo_delete::REQUEST_PATH,
                )))
            }
            Err(detail) => Err(MachineError::Refused(format!(
                "REFUSED {path} could not be read, so whether a delete is already waiting is \
                 unknown and NOTHING HAS BEEN DELIVERED. Detail: {detail}",
                path = rivet_core::repo_delete::REQUEST_PATH,
            ))),
        }
    }

    fn uninstall_interlock(&mut self) -> Result<(), MachineError> {
        if !self
            .store
            .directory_exists(rivet_core::state::STATE_MOUNT_POINT)
        {
            return Err(MachineError::Refused(format!(
                "REFUSED there is no {state} on this machine, so there is nowhere to put an \
                 uninstall request and NOTHING HAS BEEN DELIVERED.",
                state = rivet_core::state::STATE_MOUNT_POINT,
            )));
        }

        match self.store.read(uninstall::REQUEST_PATH) {
            Ok(None) => Ok(()),
            Ok(Some(pending)) => {
                let waiting = uninstall::parse_request(&pending).map_or_else(
                    |_| "an unreadable request".to_string(),
                    |request| request.name.to_string(),
                );
                Err(MachineError::Refused(format!(
                    "REFUSED an uninstall request is already waiting at {path} and NOTHING HAS \
                     BEEN DELIVERED. It names {waiting}. The supervisor takes one up on its next \
                     convergence pass — and an uninstall may be waiting on a service to stop, \
                     which ADR-017 §3.5 allows to take a deadline. `rivet get log \
                     rivet-sysd` is how to watch it.",
                    path = uninstall::REQUEST_PATH,
                )))
            }
            Err(detail) => Err(MachineError::Refused(format!(
                "REFUSED {path} could not be read, so whether an uninstall is already waiting \
                 is unknown and NOTHING HAS BEEN DELIVERED. Writing over it might have \
                 discarded somebody's uninstall. Detail: {detail}",
                path = uninstall::REQUEST_PATH,
            ))),
        }
    }

    /// The same checks for the library uninstall channel, in its own words —
    /// [`Self::uninstall_interlock`]'s argument for a second function.
    fn library_uninstall_interlock(&mut self) -> Result<(), MachineError> {
        if !self
            .store
            .directory_exists(rivet_core::state::STATE_MOUNT_POINT)
        {
            return Err(MachineError::Refused(format!(
                "REFUSED there is no {state} on this machine, so there is nowhere to put a \
                 library uninstall request and NOTHING HAS BEEN DELIVERED.",
                state = rivet_core::state::STATE_MOUNT_POINT,
            )));
        }

        match self.store.read(uninstall_library::REQUEST_PATH) {
            Ok(None) => Ok(()),
            Ok(Some(pending)) => {
                let waiting = uninstall_library::parse_request(&pending).map_or_else(
                    |_| "an unreadable request".to_string(),
                    |request| format!("`{}`", request.selector),
                );
                Err(MachineError::Refused(format!(
                    "REFUSED a library uninstall request is already waiting at {path} and \
                     NOTHING HAS BEEN DELIVERED. It names {waiting}. Writing over it would \
                     discard a removal somebody asked for, silently. The supervisor takes one up \
                     on its next convergence pass; `rivet get log` is how to watch it, on the \
                     records whose `svc` is `rivet-sysd`.",
                    path = uninstall_library::REQUEST_PATH,
                )))
            }
            Err(detail) => Err(MachineError::Refused(format!(
                "REFUSED {path} could not be read, so whether a library uninstall is already \
                 waiting is unknown and NOTHING HAS BEEN DELIVERED. Detail: {detail}",
                path = uninstall_library::REQUEST_PATH,
            ))),
        }
    }

    /// **What the library uninstall would remove**, decided by
    /// `rivet_core::uninstall_library::survey` over this machine's own tree —
    /// the same function the supervisor calls before it removes anything, so a
    /// plan and an act cannot disagree about what is refused.
    fn survey_libraries(
        &mut self,
        selector: &uninstall_library::Selector,
    ) -> Result<uninstall_library::Removal, MachineError> {
        uninstall_library::survey(
            &mut OnThisCard(&mut self.store),
            rivet_core::service::BUILT_IN_VOLUME,
            selector,
        )
        .map_err(|refusal| refusal.machine_error())
    }
}

/// This machine's tree, as `rivet_core::package::closure` reads it — the
/// [`RequestStore`] under the channel, answering in that port's terms.
struct OnThisCard<'a, S: RequestStore>(&'a mut S);

impl<S: RequestStore> rivet_core::package::closure::Tree for OnThisCard<'_, S> {
    fn directories(&mut self, path: &str) -> Result<Option<Vec<String>>, String> {
        self.0.list_directories(path)
    }

    fn exists(&mut self, path: &str) -> Result<bool, String> {
        self.0.exists(path)
    }

    fn read(&mut self, path: &str) -> Result<String, String> {
        match self.0.read(path) {
            Ok(Some(text)) => Ok(text),
            // The port says a file that should be there and is not is an
            // error — every caller reads a manifest that should exist.
            Ok(None) => Err("No such file or directory".to_string()),
            Err(detail) => Err(detail),
        }
    }
}

impl<S: RequestStore> Requests<S> {
    /// **What this machine holds under `digest`** — `ADR-052` §7, and the whole
    /// of what made `apply install` stop taking a path.
    ///
    /// The store is one directory per digest holding one file, so this is a
    /// `readdir` and a count. Three answers, and each of them is a different
    /// thing for an operator to do next:
    ///
    /// * **the directory is not there** — this machine does not have it, and
    ///   the refusal says how to send it;
    /// * **the directory cannot be read** — a failing or read-only `/state`,
    ///   which is not the same as not having it and must not be reported as
    ///   though it were (`AGENTS.md` §5);
    /// * **the directory is there and does not hold exactly one file** — an
    ///   interrupted write or something written by hand. Refused rather than
    ///   picking one, because *"install whichever of these the filesystem
    ///   listed first"* is not a rule anybody could rely on.
    ///
    /// **Nothing is opened.** The digest is not recomputed here and the
    /// artifact is not read: a second hash of 700 MB on a Pi would cost minutes
    /// to re-answer a question the transfer already answered, and a second
    /// reader of the container is what `ADR-029` §11 forbids. The bytes under
    /// that name are the bytes a transfer hashed, and the installer is what
    /// checks whether they are any good.
    fn resolve(&mut self, digest: &artifact::Digest) -> Result<Held, MachineError> {
        let store = artifact::Store::on_this_machine();
        let dir = store.object_dir(digest);
        let entries = match self.store.list_directory(&dir) {
            Ok(Some(entries)) => entries,
            Ok(None) => {
                return Err(MachineError::Refused(format!(
                    "REFUSED this machine's artifact store does not hold {digest}, so there is \
                     nothing to install and NOTHING HAS BEEN DELIVERED. An install names a \
                     digest and resolves it here — there is no form of it that names a path \
                     (ADR-052 §7). Send the artifact from the workstation that has it: `cargo \
                     rivet deploy --rvt <file> --host … confirm` transfers it into this store \
                     and asks for the install by the digest it just sent. The store is {dir}.",
                )));
            }
            Err(detail) => {
                return Err(MachineError::Refused(format!(
                    "REFUSED {dir} could not be read, so whether this machine holds {digest} is \
                     UNKNOWN and NOTHING HAS BEEN DELIVERED. This is not the same as not \
                     holding it: a full, failing or unmounted /state answers this way, and the \
                     journal from this boot says which. Detail: {detail}",
                )));
            }
        };

        let mut files = entries.into_iter().filter(|name| name != "." && name != "..");
        let (Some(name), None) = (files.next(), files.next()) else {
            return Err(MachineError::Refused(format!(
                "REFUSED {dir} is in the store but does not hold exactly one artifact, so which \
                 one {digest} names is UNKNOWN and NOTHING HAS BEEN DELIVERED. An object arrives \
                 by renaming a complete directory into place, so this one was written by \
                 something other than a transfer. Nothing here will pick one for you.",
            )));
        };
        Ok(Held { name })
    }

    /// **Refuse an install of what this machine is already running** —
    /// `PLAN-Feature-Set-2026-09-12` §3, found on `rivet-01`: re-sending the
    /// running system update was installed again, into the inactive slot, and
    /// replaced the rollback with a second copy of the running build.
    ///
    /// # Why here as well as in the installer
    ///
    /// **The installer's check is the one nothing can get past** — it runs
    /// whatever wrote the request — and it refuses in the journal, after the
    /// request was delivered and taken up. That is too late for the one party
    /// who needs the answer at the moment it is asked: `cargo rivet deploy`
    /// would report *install requested* and the operator would learn otherwise
    /// only by reading `get install`. Asked here, the refusal comes back on the
    /// same call, **before a request is written**, so the tool can stop and say
    /// the machine already runs this artifact. One decision
    /// (`rivet_core::deploy::already_running`), consulted at both points — the
    /// store door's arrangement.
    ///
    /// **It is not a separate question the tool asks first**, for the reason
    /// `ADR-052` §7 gives for not polling `get install`: an answer to *is it
    /// running?* followed by a request is a check and an act with a window
    /// between them, and a request is the question whose answer is about
    /// the moment it is acted on.
    ///
    /// # What is compared, and what is not read
    ///
    /// The stored object's first 4 KiB against the retained header of the build
    /// the running slot holds (`rivet_core::deploy::SLOT_HEADERS`) and of each
    /// application's `current`. **Bytes, not fields**: nothing here parses the
    /// container, so this is not a second reader of it (`ADR-029` §11).
    ///
    /// **Everything unreadable is "not known to be running", never a refusal.**
    /// A store object whose head will not read, a slot with no record, an
    /// application with no `current` — each leaves the installer to decide, which
    /// is what this channel did before it asked anything at all.
    fn not_already_running(
        &mut self,
        digest: &artifact::Digest,
        held: &Held,
    ) -> Result<(), MachineError> {
        let header = rivet_core::package::format::HEADER_BYTES;
        let object = artifact::Store::on_this_machine().object_path(digest, &held.name);
        let Ok(Some(head)) = self.store.read_head(&object, header) else {
            return Ok(());
        };

        // One byte more than a header, so a longer file can never compare equal
        // to one.
        let mut retained: Vec<(deploy::Running, Vec<u8>)> = Vec::new();
        if let Some(slot) = self.running_slot
            && let Ok(Some(bytes)) = self.store.read_head(&deploy::slot_header_path(slot), header + 1)
        {
            retained.push((deploy::Running::System(slot), bytes));
        }
        let volume = rivet_core::service::BUILT_IN_VOLUME;
        if let Ok(Some(names)) = self.store.list_directory(&rivet_core::service::app_root(volume)) {
            for name in names {
                let Some(service) = rivet_core::service::ServiceName::new(&name) else {
                    continue;
                };
                let path = deploy::running_application_header_path(volume, &service);
                if let Ok(Some(bytes)) = self.store.read_head(&path, header + 1) {
                    retained.push((deploy::Running::Application(name), bytes));
                }
            }
        }

        match deploy::already_running(&head, &retained) {
            None => Ok(()),
            Some(what) => Err(MachineError::Refused(deploy::already_running_refusal(
                &held.name, what,
            ))),
        }
    }

    /// **Refuse a system update while this boot's trial is undecided** —
    /// `PLAN-Feature-Set-2026-09-12` §3: on `rivet-01`, `apply install`
    /// answered `requested: true` and exit 0 during exactly that window, and
    /// only `get install` — read after the fact — said the installer had
    /// refused it. `ADR-018` §5's installer asks this question too, in its own
    /// words, once a request has already reached it; that check stays as
    /// defence in depth and this one exists so the operator learns it here,
    /// before anything is written to `/state` at all.
    ///
    /// # Applications and libraries are not this check's business
    ///
    /// A trial is a fact about which A/B **slot** this machine runs, and only a
    /// system update ever touches one. The kind comes from `held.name`
    /// ([`artifact::parse_name`]) rather than the signed header, which is
    /// exactly the bound that type documents itself with: advisory, and never
    /// the authoritative reader `ADR-029` §11 reserves for the installer. A name
    /// this build did not compose, or one whose kind is not `update`, leaves
    /// this check silent — an application install is unaffected, and a system
    /// update whose name happens not to parse still meets the installer's own
    /// check.
    ///
    /// # Everything unreadable is left to the installer, exactly as above
    ///
    /// No known running slot, or no known default slot, answers `Ok(())` here —
    /// never a refusal. `cmd/rivet-deploy`'s own comparison fails closed on
    /// precisely this question, independently; this channel's job is the case
    /// it can answer cheaply, not to be stricter than the machine's own
    /// authority.
    fn not_uncommitted_trial(&mut self, held: &Held) -> Result<(), MachineError> {
        let is_update = artifact::parse_name(&held.name)
            .is_some_and(|name| name.kind == rivet_core::package::Kind::update().as_str());
        if !is_update {
            return Ok(());
        }
        let Some(running) = self.running_slot else {
            return Ok(());
        };
        let Some(Ok(default)) = self.default_slot.clone() else {
            return Ok(());
        };
        match deploy::uncommitted_trial(running, default) {
            None => Ok(()),
            Some(default) => Err(MachineError::Refused(deploy::uncommitted_trial_refusal(
                running, default,
            ))),
        }
    }
}

impl<S: RequestStore> Deployer for Requests<S> {
    fn can_request_install(&mut self, digest: &artifact::Digest) -> Result<Held, MachineError> {
        self.interlock()?;
        let held = self.resolve(digest)?;
        self.not_already_running(digest, &held)?;
        self.not_uncommitted_trial(&held)?;
        Ok(held)
    }

    fn request_install(&mut self, digest: &artifact::Digest) -> Result<Held, MachineError> {
        // **Re-run, not remembered.** `can_request_install` may have been called
        // seconds ago on the plan path, or not at all; the card is the authority
        // for whether a request is waiting and it is asked at the point of use
        // (`AGENTS.md` §5). The store is asked again for the same reason, and
        // there is a real window between the two: retention runs when an
        // artifact arrives, so an object that was in the store at plan time can
        // be gone by the time the word is typed.
        self.interlock()?;
        let held = self.resolve(digest)?;
        // Before the request is written, so a refusal delivers nothing.
        self.not_already_running(digest, &held)?;
        self.not_uncommitted_trial(&held)?;

        let bytes = deploy::render_request(digest, &self.principal);
        self.store
            .write_private(
                deploy::REQUEST_TEMP_PATH,
                bytes.as_bytes(),
                deploy::REQUEST_MODE,
            )
            .map_err(|detail| {
                MachineError::Refused(format!(
                    "REFUSED the install request could not be written to {path}, so NOTHING HAS \
                     BEEN DELIVERED and nothing will install. A full or read-only /state is the \
                     usual cause. Detail: {detail}",
                    path = deploy::REQUEST_TEMP_PATH,
                ))
            })?;

        self.store
            .rename(deploy::REQUEST_TEMP_PATH, deploy::REQUEST_PATH)
            .map_err(|detail| {
                MachineError::Refused(format!(
                    "REFUSED the install request was written but could not be moved into place at \
                     {path}, so NOTHING HAS BEEN DELIVERED — the supervisor reads that one exact \
                     path and nothing else. A leftover {temp} is inert and will be replaced by the \
                     next attempt. Detail: {detail}",
                    path = deploy::REQUEST_PATH,
                    temp = deploy::REQUEST_TEMP_PATH,
                ))
            })?;

        // **Not a refusal.** The request is on the running machine and the
        // supervisor reads it within one convergence pass; an unflushed
        // directory risks only a power cut in the next moments losing a request
        // the operator already knows about. Saying "not delivered" about
        // something that was delivered is the failure `ADR-006` §4.2 forbids in
        // the other direction.
        let _ = self
            .store
            .flush_directory(rivet_core::state::STATE_MOUNT_POINT);
        Ok(held)
    }

    fn can_request_uninstall(&mut self, _name: &str) -> Result<(), MachineError> {
        self.uninstall_interlock()
    }

    fn request_uninstall(&mut self, name: &str, purge: bool) -> Result<(), MachineError> {
        // Re-run rather than remembered, exactly as the install path does and
        // for the same reason: the card is the authority for whether a request
        // is already waiting, and it is asked at the point of use.
        self.uninstall_interlock()?;

        let Some(service) = rivet_core::service::ServiceName::new(name) else {
            // Unreachable through `rivet_core::operation`, which checks the
            // name before it reaches a port — and checked again here rather
            // than assumed, because this port is public and a second caller
            // would not know that.
            return Err(MachineError::Unsafe(format!(
                "REFUSED {name:?} is not a usable application name, so no request was written."
            )));
        };

        let bytes = uninstall::render_request(&service, purge, &self.principal);
        self.store
            .write_private(
                uninstall::REQUEST_TEMP_PATH,
                bytes.as_bytes(),
                uninstall::REQUEST_MODE,
            )
            .map_err(|detail| {
                MachineError::Refused(format!(
                    "REFUSED the uninstall request could not be written to {path}, so NOTHING \
                     HAS BEEN DELIVERED and nothing will be removed. A full or read-only /state \
                     is the usual cause. Detail: {detail}",
                    path = uninstall::REQUEST_TEMP_PATH,
                ))
            })?;

        self.store
            .rename(uninstall::REQUEST_TEMP_PATH, uninstall::REQUEST_PATH)
            .map_err(|detail| {
                MachineError::Refused(format!(
                    "REFUSED the uninstall request was written but could not be moved into \
                     place at {path}, so NOTHING HAS BEEN DELIVERED — the supervisor reads that \
                     one exact path and nothing else. Detail: {detail}",
                    path = uninstall::REQUEST_PATH,
                ))
            })?;

        let _ = self
            .store
            .flush_directory(rivet_core::state::STATE_MOUNT_POINT);
        Ok(())
    }

    fn can_request_library_uninstall(
        &mut self,
        selector: &uninstall_library::Selector,
    ) -> Result<uninstall_library::Removal, MachineError> {
        self.library_uninstall_interlock()?;
        self.survey_libraries(selector)
    }

    fn request_library_uninstall(
        &mut self,
        selector: &uninstall_library::Selector,
    ) -> Result<uninstall_library::Removal, MachineError> {
        // Re-run rather than remembered, exactly as the two channels above do:
        // the card is the authority for what is installed and for whether a
        // request is already waiting, and it is asked at the point of use.
        self.library_uninstall_interlock()?;
        let removal = self.survey_libraries(selector)?;

        let bytes = uninstall_library::render_request(selector, &self.principal);
        self.store
            .write_private(
                uninstall_library::REQUEST_TEMP_PATH,
                bytes.as_bytes(),
                uninstall_library::REQUEST_MODE,
            )
            .map_err(|detail| {
                MachineError::Refused(format!(
                    "REFUSED the library uninstall request could not be written to {path}, so \
                     NOTHING HAS BEEN DELIVERED and nothing will be removed. A full or read-only \
                     /state is the usual cause. Detail: {detail}",
                    path = uninstall_library::REQUEST_TEMP_PATH,
                ))
            })?;

        self.store
            .rename(
                uninstall_library::REQUEST_TEMP_PATH,
                uninstall_library::REQUEST_PATH,
            )
            .map_err(|detail| {
                MachineError::Refused(format!(
                    "REFUSED the library uninstall request was written but could not be moved \
                     into place at {path}, so NOTHING HAS BEEN DELIVERED — the supervisor reads \
                     that one exact path and nothing else. Detail: {detail}",
                    path = uninstall_library::REQUEST_PATH,
                ))
            })?;

        // Not a refusal, for `request_install`'s reason: the request is on the
        // running machine either way.
        let _ = self
            .store
            .flush_directory(rivet_core::state::STATE_MOUNT_POINT);
        Ok(removal)
    }

    fn can_request_repo_delete(&mut self, _name: &str) -> Result<(), MachineError> {
        self.repo_delete_interlock()
    }

    fn request_repo_delete(&mut self, name: &str, id: &str) -> Result<(), MachineError> {
        // Re-run rather than remembered, exactly as the two paths above do.
        self.repo_delete_interlock()?;

        let Some(repo) = rivet_core::repo::RepoName::new(name) else {
            // Unreachable through `rivet_core::operation`, which checks the
            // name before it reaches a port - and checked again here rather
            // than assumed, because this port is public and a second caller
            // would not know that. **It is the check that stops `../../system`
            // reaching a remove_tree**, so it is worth twice.
            return Err(MachineError::Unsafe(format!(
                "REFUSED {name:?} is not a usable repository name, so no request was written."
            )));
        };
        if !rivet_core::identifier::is_identifier(id) {
            return Err(MachineError::Unsafe(format!(
                "REFUSED {id:?} is not an identifier, so no request was written. Without one \
                 the machine could not tell whether the directory it is about to destroy is \
                 the repository that was meant."
            )));
        }

        let bytes = rivet_core::repo_delete::render_request(&repo, id, &self.principal);
        self.store
            .write_private(
                rivet_core::repo_delete::REQUEST_TEMP_PATH,
                bytes.as_bytes(),
                rivet_core::repo_delete::REQUEST_MODE,
            )
            .map_err(|detail| {
                MachineError::Refused(format!(
                    "REFUSED the delete request could not be written to {path}, so NOTHING HAS \
                     BEEN DELIVERED and nothing will be destroyed. A full or read-only /state \
                     is the usual cause. Detail: {detail}",
                    path = rivet_core::repo_delete::REQUEST_TEMP_PATH,
                ))
            })?;

        self.store
            .rename(
                rivet_core::repo_delete::REQUEST_TEMP_PATH,
                rivet_core::repo_delete::REQUEST_PATH,
            )
            .map_err(|detail| {
                MachineError::Refused(format!(
                    "REFUSED the delete request was written but could not be moved into place \
                     at {path}, so NOTHING HAS BEEN DELIVERED - the supervisor reads that one \
                     exact path and nothing else. Detail: {detail}",
                    path = rivet_core::repo_delete::REQUEST_PATH,
                ))
            })?;

        let _ = self
            .store
            .flush_directory(rivet_core::state::STATE_MOUNT_POINT);
        Ok(())
    }
}

/// What a build with no board says, in one place.
///
/// `crate::storage::refusal`'s shape and its argument: a Windows host has
/// filesystems this build cannot see, and *"there is no `/state`"* is a true
/// statement about it that would nonetheless read as a fault. Naming the
/// platform is what makes it legible as *"you are not on the machine"*.
///
/// **Compiled where it is needed and where it is checked**, on
/// `crate::boot::Detached`'s own reasoning: on a platform that is not Linux this
/// is the answer, and under `cfg(test)` it is a contract with the tests below.
/// An unconditional `pub` would need a `dead_code` allow, which is a place for a
/// second unused thing to hide.
#[cfg(any(test, not(target_os = "linux")))]
pub fn refusal(platform: &str) -> MachineError {
    MachineError::Unsafe(format!(
        "REFUSED this is a {platform} build with no Rivet machine under it, so an install \
         cannot be requested and NOTHING HAS BEEN DELIVERED. `rivet apply install` writes a \
         request onto {state} and the supervisor on that machine performs it (the maintainer's 2026-08-24 install-request ruling); \
         there is neither here. {ask} Read the result back with `rivet get install`.",
         state = deploy::REQUEST_PATH,
         ask = rivet_core::control::ASK_A_BOARD,
    ))
}

/// The refusal a build with no board gives, as a [`Deployer`].
#[cfg(any(test, not(target_os = "linux")))]
pub struct NoMachine {
    platform: &'static str,
}

#[cfg(any(test, not(target_os = "linux")))]
impl NoMachine {
    pub const fn new(platform: &'static str) -> Self {
        Self { platform }
    }
}

#[cfg(any(test, not(target_os = "linux")))]
impl Deployer for NoMachine {
    fn can_request_install(&mut self, _digest: &artifact::Digest) -> Result<Held, MachineError> {
        Err(refusal(self.platform))
    }

    fn request_install(&mut self, _digest: &artifact::Digest) -> Result<Held, MachineError> {
        Err(refusal(self.platform))
    }

    fn can_request_uninstall(&mut self, _name: &str) -> Result<(), MachineError> {
        Err(refusal(self.platform))
    }

    fn request_uninstall(&mut self, _name: &str, _purge: bool) -> Result<(), MachineError> {
        Err(refusal(self.platform))
    }

    fn can_request_library_uninstall(
        &mut self,
        _selector: &uninstall_library::Selector,
    ) -> Result<uninstall_library::Removal, MachineError> {
        Err(refusal(self.platform))
    }

    fn request_library_uninstall(
        &mut self,
        _selector: &uninstall_library::Selector,
    ) -> Result<uninstall_library::Removal, MachineError> {
        Err(refusal(self.platform))
    }

    fn can_request_repo_delete(&mut self, _name: &str) -> Result<(), MachineError> {
        Err(refusal(self.platform))
    }

    fn request_repo_delete(&mut self, _name: &str, _id: &str) -> Result<(), MachineError> {
        Err(refusal(self.platform))
    }
}

// ---------------------------------------------------------------------------
// The store, over `std::fs`.
// ---------------------------------------------------------------------------

/// [`RequestStore`] over the real filesystem.
///
/// Stateless, on `crate::boot::SyscallMedium`'s reasoning: every fact it deals
/// in is read at the point of use.
///
/// **Not gated to Linux**, unlike the boot medium, and the difference is worth a
/// sentence. The medium needs `mount(2)` and a device tree; this needs
/// `write`, `rename` and `fsync`, which every platform has. What a Windows host
/// lacks is `/state`, and the interlock above refuses on exactly that — a
/// platform `cfg` would be a second, weaker statement of the same fact. The
/// **mode** is the one part that is genuinely Unix, and it is applied under a
/// `cfg` rather than the whole type.
pub struct Filesystem;

impl RequestStore for Filesystem {
    fn directory_exists(&mut self, path: &str) -> bool {
        std::path::Path::new(path).is_dir()
    }

    fn read(&mut self, path: &str) -> Result<Option<String>, String> {
        match std::fs::read_to_string(path) {
            Ok(contents) => Ok(Some(contents)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    fn write_private(&mut self, path: &str, bytes: &[u8], mode: u32) -> Result<(), String> {
        use std::io::Write as _;

        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        // **The mode is set at creation**, not afterwards, so there is no window
        // in which the file exists readable by anybody. A `set_permissions` call
        // after the fact would be a race that only ever matters on the machine
        // where it matters.
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(mode);
        }
        #[cfg(not(unix))]
        let _ = mode;

        let mut file = options.open(path).map_err(|e| e.to_string())?;
        file.write_all(bytes).map_err(|e| e.to_string())?;
        // Before the rename, so the rename publishes bytes that are on the card
        // rather than a name pointing at a file whose contents are still in the
        // page cache.
        file.sync_all().map_err(|e| e.to_string())
    }

    fn rename(&mut self, from: &str, to: &str) -> Result<(), String> {
        std::fs::rename(from, to).map_err(|e| e.to_string())
    }

    fn flush_directory(&mut self, path: &str) -> Result<(), String> {
        // Opening a directory read-only and `fsync`ing it is how a rename is
        // made durable on Linux. It is not portable, and it does not need to be:
        // the caller ignores the answer, and on a host where this cannot work
        // there is no `/state` for it to have been about.
        std::fs::File::open(path)
            .and_then(|d| d.sync_all())
            .map_err(|e| e.to_string())
    }

    fn read_head(&mut self, path: &str, len: usize) -> Result<Option<Vec<u8>>, String> {
        use std::io::Read as _;
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.to_string()),
        };
        let mut head = Vec::with_capacity(len);
        file.take(len as u64)
            .read_to_end(&mut head)
            .map_err(|e| e.to_string())?;
        Ok(Some(head))
    }

    fn list_directory(&mut self, path: &str) -> Result<Option<Vec<String>>, String> {
        let entries = match std::fs::read_dir(path) {
            Ok(entries) => entries,
            // **`NotFound` is the ordinary answer and is not an error.** Every
            // digest but a handful is one this machine does not hold, and
            // reporting that as a failing card would make the usual case look
            // like a fault.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.to_string()),
        };
        let mut names = Vec::new();
        for entry in entries {
            // **One bad entry refuses the whole listing.** The caller's next
            // decision is *which single file is the artifact*, and a listing
            // that silently dropped an entry it could not read could turn a
            // directory holding two into one that looks like it holds one.
            let entry = entry.map_err(|e| e.to_string())?;
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
        Ok(Some(names))
    }

    fn list_directories(&mut self, path: &str) -> Result<Option<Vec<String>>, String> {
        let entries = match std::fs::read_dir(path) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.to_string()),
        };
        let mut names = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| e.to_string())?;
            // `file_type()` does not follow a link, so a link is passed over
            // exactly as the supervisor's kernel port passes it over — the
            // two must agree about what is an application and what is a build.
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if !kind.is_dir() {
                continue;
            }
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
        names.sort();
        Ok(Some(names))
    }

    fn exists(&mut self, path: &str) -> Result<bool, String> {
        match std::fs::metadata(path) {
            Ok(_) => Ok(true),
            // The two answers that mean "looked, and nothing is there" — the
            // same two the supervisor's own check reads as `false`.
            Err(e)
                if e.kind() == std::io::ErrorKind::NotFound
                    || e.kind() == std::io::ErrorKind::NotADirectory =>
            {
                Ok(false)
            }
            Err(e) => Err(e.to_string()),
        }
    }
}

/// What this process can honestly say about who invoked it.
///
/// **The only place in Rivet that reads the environment to decide anything**,
/// and it is one variable. `rivet_core::deploy::Principal::from_environment`
/// holds every decision; this is the seam that fetches the bytes, so that all of
/// those decisions are reachable from a test on a host with no environment at
/// all.
///
/// # What it can and cannot see, and why that is not a shortcoming to fix here
///
/// * **Its own uid is root on every invocation** (`ADR-005` §5.1 makes `rivet`
///   a login shell for `root`), so it distinguishes nothing and is not consulted.
/// * **Over `ADR-035`'s control plane it can see the key**, because
///   `cmd/rivet-mcpd` pins the peer before spawning this and sets
///   `deploy::PEER_KEY_ENV`. That key IS the caller's identity rather than a
///   pointer to one, so there is nothing further to resolve.
/// * **Reached any other way it can see nothing**, and it says so rather than
///   guessing: `Origin::NoSession` is *the environment named no caller*, which
///   is not the same claim as *somebody was at the console*.
/// * **Nothing here is verified**, and everything here is forgeable by anything
///   already running as uid 0 — which is the same thing that could write
///   `rivet_core::deploy::REQUEST_PATH` by hand. `ADR-028` §4.5.
pub fn this_process() -> deploy::Principal {
    let peer = std::env::var(deploy::PEER_KEY_ENV).ok();
    deploy::Principal::from_environment(peer.as_deref())
}

/// The channel this machine has.
///
/// The one place the store is chosen, so `crate::boot` stays two forwarding
/// lines.
pub fn on_this_machine() -> Requests<Filesystem> {
    Requests::new(Filesystem, this_process())
}

/// A [`RequestStore`] in a map, for tests anywhere in this crate.
///
/// Public and `cfg(test)`: `crate::main`'s whole-binary tests drive
/// `rivet apply install` end to end and must not write to a real `/state` —
/// the development host under WSL has none, and the one machine that does is the
/// maintainer's board (`AGENTS.md` §9).
#[cfg(test)]
pub struct Memory {
    pub files: std::collections::BTreeMap<String, String>,
    pub directories: Vec<String>,
    /// Every `(from, to)` renamed, in order.
    pub renames: Vec<(String, String)>,
    /// Directories flushed, in order.
    pub flushed: Vec<String>,
    /// Modes files were created with, by path.
    pub modes: std::collections::BTreeMap<String, u32>,
    /// An error every write answers with, for the refusal path.
    pub write_fails: Option<String>,
    /// An error every read answers with — a card that is failing rather than a
    /// request that is absent.
    pub read_fails: Option<String>,
    /// What the artifact store holds, as a directory listing per path.
    ///
    /// **A map and not a set of digests**, because the interesting refusals are
    /// about the *contents* of an object directory: one that is empty, and one
    /// that holds two files.
    pub listings: std::collections::BTreeMap<String, Vec<String>>,
    /// An error every listing answers with — a card that will not say what it
    /// holds, which is a different answer from *it holds nothing*.
    pub list_fails: Option<String>,
    /// Files read as bytes, by path — an artifact in the store and the headers
    /// retained beside what the machine runs.
    pub heads: std::collections::BTreeMap<String, Vec<u8>>,
}

#[cfg(test)]
impl Memory {
    /// A healthy Rivet machine with no request waiting.
    pub fn rivet() -> Self {
        Self {
            files: std::collections::BTreeMap::new(),
            directories: vec![rivet_core::state::STATE_MOUNT_POINT.to_string()],
            renames: Vec::new(),
            flushed: Vec::new(),
            modes: std::collections::BTreeMap::new(),
            write_fails: None,
            read_fails: None,
            listings: std::collections::BTreeMap::new(),
            list_fails: None,
            heads: std::collections::BTreeMap::new(),
        }
    }

    /// A host that is not a Rivet machine: no `/state`.
    pub fn bare() -> Self {
        Self {
            directories: Vec::new(),
            ..Self::rivet()
        }
    }

    /// What is at [`rivet_core::deploy::REQUEST_PATH`] now.
    pub fn request(&self) -> Option<&String> {
        self.files.get(deploy::REQUEST_PATH)
    }

    /// **Put an artifact in this machine's store**, which is what makes a
    /// digest installable at all since `ADR-052` §7.
    pub fn holding(mut self, digest: &artifact::Digest, name: &str) -> Self {
        let store = artifact::Store::on_this_machine();
        self.listings
            .insert(store.object_dir(digest), vec![name.to_string()]);
        self
    }

    /// What is at [`rivet_core::uninstall_library::REQUEST_PATH`] now.
    pub fn library_request(&self) -> Option<&String> {
        self.files.get(uninstall_library::REQUEST_PATH)
    }

    /// **Install a library build under `/state/lib`**, providing `class`, with
    /// its manifest rendered by the manifest's own writer. Hand-composed.
    pub fn with_library(
        mut self,
        reference: &rivet_core::package::library::Reference,
        class: Option<&str>,
    ) -> Self {
        let volume = rivet_core::service::BUILT_IN_VOLUME;
        self.listings
            .entry(rivet_core::package::install::lib_root(volume))
            .or_default()
            .push(reference.to_string());
        let manifest = rivet_core::package::library::Manifest::new(vec!["libx.so.1".to_string()])
            .unwrap_or_else(|e| unreachable!("a fixture soname: {e:?}"))
            .with_addon(class.map(ToString::to_string), None)
            .unwrap_or_else(|e| unreachable!("a fixture class: {e:?}"));
        self.files.insert(
            format!(
                "{}/{}",
                rivet_core::package::install::library_dir(volume, reference),
                rivet_core::package::library::MANIFEST_NAME
            ),
            manifest.render(),
        );
        self
    }

    /// **Install an application under `/state/app`** declaring `library` and
    /// accepting `accepts`, with its manifest rendered by the manifest's own
    /// writer. Hand-composed.
    pub fn with_application(
        mut self,
        name: &str,
        library: &[&rivet_core::package::library::Reference],
        accepts: &[&str],
    ) -> Self {
        use rivet_core::package::application::{Manifest, ProgramKind};
        let volume = rivet_core::service::BUILT_IN_VOLUME;
        self.listings
            .entry(rivet_core::service::app_root(volume))
            .or_default()
            .push(name.to_string());
        let manifest = Manifest::new(ProgramKind::Daemon)
            .with_library(library.iter().map(|r| (*r).clone()).collect())
            .unwrap_or_else(|e| unreachable!("a fixture closure: {e:?}"))
            .with_accepts_classes(accepts.iter().map(ToString::to_string).collect())
            .unwrap_or_else(|e| unreachable!("fixture classes: {e:?}"));
        let service = rivet_core::service::ServiceName::new(name)
            .unwrap_or_else(|| unreachable!("{name} is a service name"));
        self.files.insert(
            rivet_core::service::manifest_path(volume, &service),
            manifest.render(),
        );
        self
    }
}

#[cfg(test)]
impl RequestStore for Memory {
    fn directory_exists(&mut self, path: &str) -> bool {
        self.directories.iter().any(|d| d == path)
    }

    fn read(&mut self, path: &str) -> Result<Option<String>, String> {
        if let Some(detail) = &self.read_fails {
            return Err(detail.clone());
        }
        Ok(self.files.get(path).cloned())
    }

    fn write_private(&mut self, path: &str, bytes: &[u8], mode: u32) -> Result<(), String> {
        if let Some(detail) = &self.write_fails {
            return Err(detail.clone());
        }
        let text = String::from_utf8(bytes.to_vec()).map_err(|e| e.to_string())?;
        self.files.insert(path.to_string(), text);
        self.modes.insert(path.to_string(), mode);
        Ok(())
    }

    fn rename(&mut self, from: &str, to: &str) -> Result<(), String> {
        self.renames.push((from.to_string(), to.to_string()));
        let Some(contents) = self.files.remove(from) else {
            return Err(format!("{from}: no such file"));
        };
        if let Some(mode) = self.modes.remove(from) {
            self.modes.insert(to.to_string(), mode);
        }
        self.files.insert(to.to_string(), contents);
        Ok(())
    }

    fn flush_directory(&mut self, path: &str) -> Result<(), String> {
        self.flushed.push(path.to_string());
        Ok(())
    }

    fn list_directory(&mut self, path: &str) -> Result<Option<Vec<String>>, String> {
        if let Some(detail) = &self.list_fails {
            return Err(detail.clone());
        }
        Ok(self.listings.get(path).cloned())
    }

    fn read_head(&mut self, path: &str, len: usize) -> Result<Option<Vec<u8>>, String> {
        Ok(self
            .heads
            .get(path)
            .map(|bytes| bytes[..bytes.len().min(len)].to_vec()))
    }

    /// The same listings as [`RequestStore::list_directory`]: every name a test
    /// puts in [`Memory::listings`] for a tree it means as directories is one.
    fn list_directories(&mut self, path: &str) -> Result<Option<Vec<String>>, String> {
        self.list_directory(path)
    }

    fn exists(&mut self, path: &str) -> Result<bool, String> {
        Ok(self.files.contains_key(path)
            || self.heads.contains_key(path)
            || self.listings.contains_key(path))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// A pinned key, spelled out once, for every test that needs a real one.
    const A_KEY: &str = "3f7c1a9e04b2d85f6a3c9017e2b48d5f1c6a09b3e7d248fa5601c93b8ed72a4f";

    /// A caller over `ADR-035`'s control plane, which is the ordinary way an
    /// install is asked for.
    fn over_the_control_plane() -> deploy::Principal {
        deploy::Principal::from_environment(Some(A_KEY))
    }

    /// The digest every install below asks for.
    const A_DIGEST: &str = "8c4e9b21f0a37d5e6b18c92470fd3a85e1c7b0946d2af38e5710cb639ad84f2c";

    /// A second one, for the tests that need two.
    const ANOTHER: &str = "1d2e3f405162738495a6b7c8d9eafb0c1d2e3f405162738495a6b7c8d9eafb0c";

    /// What the store holds it under.
    const ARTIFACT: &str = "photo-index-0.1.0-aarch64-application.rvt";

    fn digest(text: &str) -> artifact::Digest {
        artifact::Digest::new(text).unwrap_or_else(|| unreachable!("{text} is a digest"))
    }

    fn a_digest() -> artifact::Digest {
        digest(A_DIGEST)
    }

    fn channel(store: Memory) -> Requests<Memory> {
        Requests::new(store, over_the_control_plane())
    }

    /// A machine holding the one artifact every test below installs.
    fn holding() -> Requests<Memory> {
        channel(Memory::rivet().holding(&a_digest(), ARTIFACT))
    }

    /// **The whole of the ruling, from this side.** A confirmed install leaves
    /// one file on `/state` naming exactly the digest that was asked for, in the
    /// form `rivet_core::deploy::parse_request` reads.
    #[test]
    fn a_confirmed_install_leaves_one_request_naming_the_digest() {
        let mut c = holding();
        assert_eq!(
            c.request_install(&a_digest()),
            Ok(Held {
                name: ARTIFACT.to_string()
            })
        );

        let store = &c.store;
        let Some(request) = store.request() else {
            unreachable!("nothing was written to {}", deploy::REQUEST_PATH)
        };
        assert_eq!(
            deploy::parse_request(request),
            Ok(deploy::Request {
                digest: a_digest(),
                principal: over_the_control_plane(),
            }),
            "the supervisor's own parser could not read what this wrote"
        );
    }

    /// **A digest this machine does not hold is refused, and nothing is
    /// written** — `ADR-052` §7. The refusal says how to get the artifact here,
    /// because an operator holding a board and no route to it is stuck.
    #[test]
    fn a_digest_the_store_does_not_hold_is_refused_and_writes_nothing() {
        let mut c = channel(Memory::rivet());
        let refusal = c
            .request_install(&a_digest())
            .expect_err("an empty store");
        assert!(refusal.why().contains("does not hold"), "{}", refusal.why());
        assert!(refusal.why().contains("ADR-052 §7"), "{}", refusal.why());
        assert!(refusal.why().contains("cargo rivet deploy"), "{}", refusal.why());
        assert!(c.store.files.is_empty(), "{:?}", c.store.files);
    }

    /// **A store that cannot be listed is UNKNOWN, not empty.** A failing card
    /// and a machine that was never sent the artifact are opposite things to do
    /// next.
    #[test]
    fn a_store_that_cannot_be_read_is_not_reported_as_one_that_holds_nothing() {
        let mut store = Memory::rivet();
        store.list_fails = Some("Input/output error".to_string());
        let mut c = channel(store);
        let refusal = c
            .request_install(&a_digest())
            .expect_err("a card that will not answer");
        assert!(refusal.why().contains("UNKNOWN"), "{}", refusal.why());
        assert!(
            !refusal.why().contains("does not hold"),
            "an unreadable store was reported as an empty one: {}",
            refusal.why()
        );
        assert!(c.store.files.is_empty());
    }

    /// **An object directory that does not hold exactly one artifact is
    /// refused**, because *"install whichever the filesystem listed first"* is
    /// not a rule anybody could rely on.
    #[test]
    fn an_object_that_is_not_exactly_one_artifact_is_refused_rather_than_chosen() {
        for entries in [Vec::new(), vec!["a.rvt".to_string(), "b.rvt".to_string()]] {
            let mut store = Memory::rivet();
            store.listings.insert(
                artifact::Store::on_this_machine().object_dir(&a_digest()),
                entries.clone(),
            );
            let mut c = channel(store);
            let refusal = c
                .request_install(&a_digest())
                .expect_err("{entries:?} must be refused");
            assert!(
                refusal.why().contains("exactly one artifact"),
                "{entries:?}: {}",
                refusal.why()
            );
            assert!(c.store.files.is_empty());
        }
    }

    /// **The request says who asked**, and the key it names is the one the
    /// control plane pinned the caller by — which is the whole of the chain of
    /// custody `ADR-028` §4.5 claims.
    #[test]
    fn a_request_names_the_caller_that_asked_for_it() {
        let mut c = holding();
        c.request_install(&a_digest()).expect("a healthy machine");
        let expected = format!("from=pinned-{A_KEY} -- {A_DIGEST}\n");
        assert_eq!(c.store.request().map(String::as_str), Some(expected.as_str()));
    }

    /// **An invocation nothing named a caller for is not an error and does not
    /// claim to be a console.** The install proceeds, and nothing in the record
    /// says anybody was standing at the board.
    #[test]
    fn an_invocation_with_no_named_caller_is_delivered_and_says_so_plainly() {
        let mut c = Requests::new(
            Memory::rivet().holding(&a_digest(), ARTIFACT),
            deploy::Principal::no_session(),
        );
        assert!(c.request_install(&a_digest()).is_ok());
        assert_eq!(
            c.store.request().map(String::as_str),
            Some(format!("from=no-session -- {A_DIGEST}\n").as_str())
        );
    }

    /// **This host is not a Rivet board**, so the principal it reads is the one
    /// that says nothing — asserted rather than assumed, because a developer
    /// running the suite with the variable set would otherwise see a different
    /// answer from CI and neither would be wrong.
    #[test]
    fn the_principal_of_this_process_is_read_and_never_invented() {
        let principal = this_process();
        let expected =
            deploy::Principal::from_environment(std::env::var(deploy::PEER_KEY_ENV).ok().as_deref());
        assert_eq!(principal, expected);
        // Whatever the environment said, it is renderable and parses back —
        // which is the property that matters on the one machine that has both.
        let bytes = deploy::render_request(&a_digest(), &principal);
        assert_eq!(
            deploy::parse_request(&bytes).map(|r| r.digest),
            Ok(a_digest()),
            "this host's own environment produced a request the supervisor would refuse: {bytes:?}"
        );
    }

    /// **Written elsewhere and renamed into place**, so a supervisor that looks
    /// at the wrong moment sees a whole request or none — never a truncated path
    /// naming a file nobody asked for.
    #[test]
    fn a_request_is_renamed_into_place_rather_than_written_there() {
        let mut c = holding();
        c.request_install(&a_digest())
            .expect("a healthy machine");
        assert_eq!(
            c.store.renames,
            vec![(
                deploy::REQUEST_TEMP_PATH.to_string(),
                deploy::REQUEST_PATH.to_string()
            )]
        );
        assert!(
            !c.store.files.contains_key(deploy::REQUEST_TEMP_PATH),
            "the temporary file was left behind"
        );
        assert_eq!(
            c.store.modes.get(deploy::REQUEST_PATH),
            Some(&deploy::REQUEST_MODE)
        );
        assert_eq!(
            c.store.flushed,
            vec![rivet_core::state::STATE_MOUNT_POINT.to_string()]
        );
    }

    /// **A second request does not silently replace the first.** The operator
    /// who asked for the first was told it was delivered; overwriting it would
    /// make that a lie, and they would watch a payload they did not name
    /// install.
    #[test]
    fn a_request_already_waiting_is_refused_rather_than_overwritten() {
        let mut c = channel(
            Memory::rivet()
                .holding(&a_digest(), ARTIFACT)
                .holding(&digest(ANOTHER), "photo-index-0.2.0-aarch64-application.rvt"),
        );
        c.request_install(&a_digest()).expect("the first");

        let refusal = c
            .request_install(&digest(ANOTHER))
            .expect_err("a second request while one waits");
        assert!(matches!(refusal, MachineError::Refused(_)), "{refusal:?}");
        assert!(refusal.why().contains(A_DIGEST), "{}", refusal.why());
        assert!(
            refusal.why().contains("NOTHING HAS BEEN DELIVERED"),
            "{}",
            refusal.why()
        );
        // ...and the first one is untouched.
        let untouched = format!("from=pinned-{A_KEY} -- {A_DIGEST}\n");
        assert_eq!(c.store.request().map(String::as_str), Some(untouched.as_str()));
    }

    /// The plan path refuses for the same reason and in the same words, so an
    /// operator who saw the plan refused and typed `confirm` anyway is not told
    /// a second, different story.
    #[test]
    fn the_plan_path_and_the_confirmed_path_give_one_answer() {
        // A host with no /state, and a machine whose request is already taken —
        // the two things the interlock refuses.
        let mut no_state = channel(Memory::bare());
        let mut occupied = holding();
        occupied.request_install(&a_digest()).expect("the first");
        // ...and a third: a machine whose store does not hold what was asked
        // for, which is `ADR-052` §7's own refusal and must behave the same way.
        let mut empty_store = channel(Memory::rivet());

        for c in [&mut no_state, &mut occupied, &mut empty_store] {
            let planned = c.can_request_install(&a_digest());
            let confirmed = c.request_install(&a_digest());
            assert!(planned.is_err(), "the interlock let this through");
            assert_eq!(planned, confirmed);
        }
    }

    /// **A machine with no `/state` refuses, and nothing is written.** There is
    /// nowhere to put a request and nothing that would ever read one.
    #[test]
    fn a_machine_with_no_state_refuses_and_writes_nothing() {
        let mut c = channel(Memory::bare());
        let refusal = c
            .request_install(&a_digest())
            .expect_err("there is no /state");
        assert!(matches!(refusal, MachineError::Refused(_)), "{refusal:?}");
        assert!(c.store.files.is_empty(), "{:?}", c.store.files);
        assert!(c.store.renames.is_empty());
    }

    /// **A card that cannot be read is a refusal, not a go-ahead.** Reading
    /// "could not be read" as "there is nothing there" would overwrite somebody
    /// else's pending install on exactly the machine least able to afford it.
    #[test]
    fn a_request_that_cannot_be_read_stops_this_one_rather_than_replacing_it() {
        let mut store = Memory::rivet().holding(&a_digest(), ARTIFACT);
        store.read_fails = Some("Input/output error".to_string());
        let mut c = channel(store);
        let refusal = c
            .request_install(&a_digest())
            .expect_err("a card that will not answer");
        assert!(refusal.why().contains("NOTHING HAS BEEN DELIVERED"), "{}", refusal.why());
        assert!(c.store.files.is_empty());
    }

    /// A write that fails is a refusal that **says nothing was delivered**, and
    /// never an `Ok` — `AGENTS.md` §3's rule about stubs applied to the failure
    /// path of a real adapter.
    #[test]
    fn a_write_that_fails_is_never_reported_as_delivered() {
        let mut store = Memory::rivet().holding(&a_digest(), ARTIFACT);
        store.write_fails = Some("No space left on device".to_string());
        let mut c = channel(store);
        let refusal = c
            .request_install(&a_digest())
            .expect_err("a full card");
        assert!(
            refusal.why().contains("NOTHING HAS BEEN DELIVERED"),
            "{}",
            refusal.why()
        );
        assert!(c.store.renames.is_empty(), "a failed write was published");
    }

    /// **Nothing here looks INSIDE the payload**, which is `ADR-029` §11's
    /// single-verifier rule where it costs something: an object that is in the
    /// store but is corrupt, unsigned, for another architecture or not a `.rvt`
    /// at all is delivered, and the installer refuses it in the journal where
    /// `rivet get install` can report which check refused it.
    ///
    /// **The half that DID change on 2026-09-12** is that a payload which is not
    /// there at all is now refused here — that is the store answering *I do not
    /// have these bytes*, which is a fact about the machine and not a verdict on
    /// an artifact. The two are kept apart deliberately, and this test is the
    /// pair to `a_digest_the_store_does_not_hold_is_refused_and_writes_nothing`.
    #[test]
    fn an_object_this_machine_holds_is_delivered_without_being_second_guessed() {
        let mut c = channel(
            Memory::rivet().holding(&a_digest(), "not-even-a-composed-name.rvt"),
        );
        assert_eq!(
            c.request_install(&a_digest()),
            Ok(Held {
                name: "not-even-a-composed-name.rvt".to_string()
            })
        );
        assert!(c.store.request().is_some());
    }

    // -- PLAN-Feature-Set-2026-09-12 §3: what this machine already runs ----

    /// A 4 KiB "header". Its contents are never parsed here, only compared.
    fn header(fill: u8) -> Vec<u8> {
        vec![fill; rivet_core::package::format::HEADER_BYTES]
    }

    /// A machine holding [`ARTIFACT`] under [`A_DIGEST`], whose first 4 KiB are
    /// `head`, followed by the rest of an artifact.
    fn holding_an_artifact_headed(head: &[u8]) -> Memory {
        let mut store = Memory::rivet().holding(&a_digest(), ARTIFACT);
        let mut bytes = head.to_vec();
        bytes.extend_from_slice(b"...the members...");
        store.heads.insert(
            artifact::Store::on_this_machine().object_path(&a_digest(), ARTIFACT),
            bytes,
        );
        store
    }

    /// **The defect, from the channel.** `rivet-01` was sent the system update
    /// it was running and the request was delivered, so the installer wrote a
    /// second copy of it over the rollback. The build the running slot holds is
    /// refused here, **before a request is written**, in the words a host tool
    /// stops on — and the plan path says the same.
    #[test]
    fn the_build_the_running_slot_holds_is_refused_before_a_request_is_written() {
        let mut store = holding_an_artifact_headed(&header(0x85));
        store.heads.insert(
            deploy::slot_header_path(rivet_core::state::Slot::B),
            header(0x85),
        );
        let mut c = channel(store).running_slot(Some(rivet_core::state::Slot::B));

        let planned = c.can_request_install(&a_digest());
        let refusal = c.request_install(&a_digest()).expect_err("the running build");
        assert_eq!(planned, Err(refusal.clone()), "the plan and the confirmation disagree");
        assert!(matches!(refusal, MachineError::Refused(_)), "{refusal:?}");
        assert!(refusal.why().contains(deploy::ALREADY_RUNNING), "{}", refusal.why());
        assert!(refusal.why().contains("slot b"), "{}", refusal.why());
        assert!(c.store.files.is_empty(), "a request was written: {:?}", c.store.files);
        assert!(c.store.renames.is_empty());
    }

    /// **And the application `current` selects.**
    #[test]
    fn the_application_current_selects_is_refused_before_a_request_is_written() {
        let mut store = holding_an_artifact_headed(&header(0x11));
        store.listings.insert(
            rivet_core::service::app_root(rivet_core::service::BUILT_IN_VOLUME),
            vec!["tempmon-rivet".to_string(), "photo-index".to_string()],
        );
        let photo = rivet_core::service::ServiceName::new("photo-index").expect("a name");
        let tempmon = rivet_core::service::ServiceName::new("tempmon-rivet").expect("a name");
        let volume = rivet_core::service::BUILT_IN_VOLUME;
        store.heads.insert(
            deploy::running_application_header_path(volume, &tempmon),
            header(0x22),
        );
        store.heads.insert(
            deploy::running_application_header_path(volume, &photo),
            header(0x11),
        );
        let mut c = channel(store);
        let refusal = c.request_install(&a_digest()).expect_err("the running application");
        assert!(refusal.why().contains(deploy::ALREADY_RUNNING), "{}", refusal.why());
        assert!(refusal.why().contains("photo-index"), "{}", refusal.why());
        assert!(c.store.files.is_empty(), "{:?}", c.store.files);
    }

    /// **Not running is delivered as it always was**: a different build in the
    /// running slot, the same build in the slot that is NOT running, a retained
    /// file longer than a header, and a machine with no records at all.
    #[test]
    fn anything_not_known_to_be_running_is_delivered() {
        // A different build in the running slot.
        let mut store = holding_an_artifact_headed(&header(0x86));
        store.heads.insert(deploy::slot_header_path(rivet_core::state::Slot::B), header(0x85));
        let mut c = channel(store).running_slot(Some(rivet_core::state::Slot::B));
        assert!(c.request_install(&a_digest()).is_ok(), "a new build was refused");

        // This build, in the slot that is not running.
        let mut store = holding_an_artifact_headed(&header(0x85));
        store.heads.insert(deploy::slot_header_path(rivet_core::state::Slot::A), header(0x85));
        let mut c = channel(store).running_slot(Some(rivet_core::state::Slot::B));
        assert!(c.request_install(&a_digest()).is_ok(), "the inactive slot's build was called running");

        // A retained file that merely BEGINS with this header.
        let mut store = holding_an_artifact_headed(&header(0x85));
        let mut longer = header(0x85);
        longer.push(0);
        store.heads.insert(deploy::slot_header_path(rivet_core::state::Slot::B), longer);
        let mut c = channel(store).running_slot(Some(rivet_core::state::Slot::B));
        assert!(c.request_install(&a_digest()).is_ok(), "a longer record compared equal");

        // No slot reading and no records: today's behaviour, unchanged.
        let mut c = channel(holding_an_artifact_headed(&header(0x85)));
        assert!(c.request_install(&a_digest()).is_ok());
    }

    // -- PLAN-Feature-Set-2026-09-12 §3: the trial window ---------------------

    /// A system update, named the way `cargo rivet package` names one — the
    /// only name this check ever acts on.
    const UPDATE_ARTIFACT: &str = "rivet-0.1.86-aarch64-rpi4-update.rvt";

    fn holding_update() -> Requests<Memory> {
        channel(Memory::rivet().holding(&a_digest(), UPDATE_ARTIFACT))
    }

    /// **The defect, from the channel.** On `rivet-01`, `apply install`
    /// answered `requested: true` and exit 0 while the trial was undecided, and
    /// only `get install` said the installer had refused it. Refused here
    /// instead, before a request is written, for the same reason
    /// `the_build_the_running_slot_holds_is_refused_before_a_request_is_written`
    /// exists for *already running*: the plan path and the confirmation must
    /// agree.
    #[test]
    fn a_system_update_during_an_uncommitted_trial_is_refused_before_a_request_is_written() {
        let mut c = holding_update()
            .running_slot(Some(rivet_core::state::Slot::A))
            .default_slot(Some(Ok(rivet_core::state::Slot::B)));

        let planned = c.can_request_install(&a_digest());
        let refusal = c
            .request_install(&a_digest())
            .expect_err("an uncommitted trial");
        assert_eq!(planned, Err(refusal.clone()), "the plan and the confirmation disagree");
        assert!(matches!(refusal, MachineError::Refused(_)), "{refusal:?}");
        assert!(refusal.why().contains(deploy::UNCOMMITTED_TRIAL), "{}", refusal.why());
        assert!(refusal.why().contains("slot a"), "{}", refusal.why());
        assert!(refusal.why().contains("slot b"), "{}", refusal.why());
        assert!(
            refusal.why().contains("NOTHING HAS BEEN DELIVERED"),
            "{}",
            refusal.why()
        );
        assert!(c.store.files.is_empty(), "a request was written: {:?}", c.store.files);
        assert!(c.store.renames.is_empty());
    }

    /// **A committed boot is not refused**, however it was entered — the
    /// default and the running slot are the same, which is what committed
    /// means (`SlotRead::committed`).
    #[test]
    fn a_committed_boot_is_not_refused_for_the_trial() {
        let mut c = holding_update()
            .running_slot(Some(rivet_core::state::Slot::A))
            .default_slot(Some(Ok(rivet_core::state::Slot::A)));
        assert!(c.request_install(&a_digest()).is_ok());
    }

    /// **Only a system update touches a slot.** An application named exactly
    /// like the one above but for the wrong kind of artifact is delivered
    /// during an uncommitted trial without this check ever firing — the
    /// installer's own check has nothing to say about it either, for the same
    /// reason.
    #[test]
    fn an_application_is_never_refused_for_the_trial_window() {
        let mut c = holding() // ARTIFACT is an application.
            .running_slot(Some(rivet_core::state::Slot::A))
            .default_slot(Some(Ok(rivet_core::state::Slot::B)));
        assert!(c.request_install(&a_digest()).is_ok());
    }

    /// **Everything unreadable is left to the installer, never refused here** —
    /// `not_already_running`'s rule applied to the trial: an unmeasured default,
    /// one that could not be read, and no running slot at all.
    #[test]
    fn an_unmeasured_trial_is_left_to_the_installer() {
        // No default slot known.
        let mut c = holding_update().running_slot(Some(rivet_core::state::Slot::A));
        assert!(c.request_install(&a_digest()).is_ok(), "no default slot answered");

        // The default could not be read at all.
        let mut c = holding_update()
            .running_slot(Some(rivet_core::state::Slot::A))
            .default_slot(Some(Err("autoboot.txt would not parse".to_string())));
        assert!(c.request_install(&a_digest()).is_ok(), "an unreadable default answered");

        // No running slot known.
        let mut c = holding_update().default_slot(Some(Ok(rivet_core::state::Slot::B)));
        assert!(c.request_install(&a_digest()).is_ok(), "no running slot answered");
    }

    /// A build with no board refuses with `E_UNSAFE` and names the platform —
    /// *stop*, rather than *look at the hardware*, which is `ADR-006` §4.1's own
    /// test for whether two codes are worth having.
    #[test]
    fn a_build_with_no_board_tells_an_agent_to_stop_and_names_the_host() {
        let mut c = NoMachine::new("windows");
        for refusal in [
            c.can_request_install(&a_digest()).expect_err("no board"),
            c.request_install(&a_digest()).expect_err("no board"),
        ] {
            assert!(matches!(refusal, MachineError::Unsafe(_)), "{refusal:?}");
            assert!(refusal.why().contains("windows"), "{}", refusal.why());
            assert!(
                refusal.why().contains("NOTHING HAS BEEN DELIVERED"),
                "{}",
                refusal.why()
            );
            assert!(
                refusal.why().contains("rivet get install"),
                "{}",
                refusal.why()
            );
        }
    }

    /// **The real store is not pointed at a real `/state` by any test in this
    /// module**, and this is the assertion that says so out loud. `AGENTS.md`
    /// §9: nothing here may touch the maintainer's machine, and the development
    /// host under WSL is a Linux box on which `Filesystem` would happily create
    /// `/state` if something asked it to.
    #[test]
    fn the_real_store_reports_this_host_as_having_no_state_and_is_not_asked_to_write() {
        let mut store = Filesystem;
        // Reading a directory that is not there is the whole interaction, and it
        // answers without creating anything.
        let exists = store.directory_exists(rivet_core::state::STATE_MOUNT_POINT);
        assert!(
            !exists || std::path::Path::new(rivet_core::state::STATE_MOUNT_POINT).is_dir(),
            "the store invented a directory"
        );
    }

    // -- the library uninstall channel ---------------------------------------

    fn library(product: &str, version: &str, byte: u8) -> rivet_core::package::library::Reference {
        rivet_core::package::library::Reference::compose(product, version, &[byte; 32])
            .unwrap_or_else(|e| unreachable!("a fixture reference: {e:?}"))
    }

    fn selector(text: &str) -> uninstall_library::Selector {
        uninstall_library::Selector::parse(text)
            .unwrap_or_else(|e| unreachable!("a fixture selector: {}", e.why()))
    }

    /// **The two halves of the channel, meeting.** The exact bytes this writer
    /// leaves on the card are what `rivet_core::uninstall_library::parse_request`
    /// — the reader `cmd/rivet-sysd`'s watch calls — reads back as the selector
    /// and the principal that were asked for; and they arrive by a rename from
    /// the temporary name, at `0600`.
    #[test]
    fn a_confirmed_library_uninstall_leaves_one_request_the_supervisors_reader_parses() {
        let pack = library("aura-background-pack", "0.2.0", 0xaa);
        let mut c = channel(
            Memory::rivet()
                .with_library(&pack, Some("aura-background-pack"))
                .with_application("aura-renderer", &[], &["aura-background-pack"]),
        );

        let removal = c
            .request_library_uninstall(&selector("aura-background-pack"))
            .expect("delivered");
        assert_eq!(removal.builds, vec![pack]);
        assert_eq!(removal.acceptors.len(), 1, "{:?}", removal.acceptors);

        let Some(written) = c.store.library_request() else {
            unreachable!("nothing was written to {}", uninstall_library::REQUEST_PATH)
        };
        assert_eq!(
            uninstall_library::parse_request(written),
            Ok(uninstall_library::Request {
                selector: selector("aura-background-pack"),
                principal: over_the_control_plane(),
            }),
            "the supervisor's own reader could not read what this wrote"
        );
        assert_eq!(
            c.store.renames,
            vec![(
                uninstall_library::REQUEST_TEMP_PATH.to_string(),
                uninstall_library::REQUEST_PATH.to_string()
            )]
        );
        assert_eq!(
            c.store.modes.get(uninstall_library::REQUEST_PATH),
            Some(&uninstall_library::REQUEST_MODE)
        );
    }

    /// **The guard, on the plan path and the confirmed one**: an application
    /// that declares a selected build refuses the whole request, as
    /// `MachineError::Unsafe`, and nothing is written.
    #[test]
    fn a_build_named_in_a_declared_closure_is_refused_and_nothing_is_written() {
        let pack = library("libgreet", "1.0.0", 0xaa);
        let mut c = channel(
            Memory::rivet()
                .with_library(&pack, None)
                .with_application("greeter", &[&pack], &[]),
        );
        for refusal in [
            c.can_request_library_uninstall(&selector("libgreet"))
                .expect_err("declared"),
            c.request_library_uninstall(&selector("libgreet"))
                .expect_err("declared"),
        ] {
            assert!(matches!(refusal, MachineError::Unsafe(_)), "{refusal:?}");
            assert!(refusal.why().contains("`greeter` declares"), "{}", refusal.why());
        }
        assert!(c.store.files.keys().all(|p| !p.starts_with(uninstall_library::REQUEST_PATH)));
    }

    /// **An application whose manifest cannot be read refuses**, and nothing is
    /// written: a version directory with no manifest beside it.
    #[test]
    fn an_application_with_no_readable_manifest_refuses_and_nothing_is_written() {
        let pack = library("libgreet", "1.0.0", 0xaa);
        let mut store = Memory::rivet().with_library(&pack, None);
        let volume = rivet_core::service::BUILT_IN_VOLUME;
        store
            .listings
            .insert(rivet_core::service::app_root(volume), vec!["half".to_string()]);
        store.listings.insert(
            rivet_core::service::app_dir(
                volume,
                &rivet_core::service::ServiceName::new("half")
                    .unwrap_or_else(|| unreachable!("a name")),
            ),
            vec!["0.1.0".to_string()],
        );
        let mut c = channel(store);
        let refusal = c
            .request_library_uninstall(&selector("libgreet"))
            .expect_err("a hole");
        assert!(matches!(refusal, MachineError::Refused(_)), "{refusal:?}");
        assert!(refusal.why().contains("`half`"), "{}", refusal.why());
        assert!(c.store.library_request().is_none());
    }

    /// **A second request while one waits is refused, not silently swapped**,
    /// and the refusal names the one that is waiting.
    #[test]
    fn a_second_library_uninstall_while_one_waits_is_refused() {
        let pack = library("libgreet", "1.0.0", 0xaa);
        let other = library("libfarewell", "1.0.0", 0xbb);
        let mut c = channel(
            Memory::rivet()
                .with_library(&pack, None)
                .with_library(&other, None),
        );
        c.request_library_uninstall(&selector("libgreet"))
            .expect("the first is delivered");
        let refusal = c
            .request_library_uninstall(&selector("libfarewell"))
            .expect_err("one is waiting");
        assert!(refusal.why().contains("`libgreet`"), "{}", refusal.why());
        assert_eq!(
            uninstall_library::parse_request(c.store.library_request().expect("still there"))
                .map(|r| r.selector),
            Ok(selector("libgreet")),
            "the waiting request was replaced"
        );
    }

    /// Nothing matching is refused before anything is written, and the refusal
    /// names what is installed.
    #[test]
    fn nothing_matching_is_refused_and_nothing_is_written() {
        let pack = library("libgreet", "1.0.0", 0xaa);
        let mut c = channel(Memory::rivet().with_library(&pack, None));
        let refusal = c
            .request_library_uninstall(&selector("libgreeter"))
            .expect_err("nothing matches");
        assert!(refusal.why().contains(pack.as_str()), "{}", refusal.why());
        assert!(c.store.library_request().is_none());
    }

    /// **The real store lists directories and nothing else**, so this channel
    /// sees the tree the supervisor's discovery sees. Written into a directory
    /// this test makes under the host's temporary directory.
    #[test]
    fn the_real_store_lists_directories_only() {
        let root = std::env::temp_dir().join(format!(
            "rivet-cli-list-directories-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("an-application")).expect("a directory");
        std::fs::write(root.join("settings"), "x").expect("a file");

        let mut store = Filesystem;
        let path = root.to_string_lossy().into_owned();
        let listed = store.list_directories(&path);
        let missing = store.list_directories(&format!("{path}/absent"));
        let file_exists = store.exists(&format!("{path}/settings"));
        let absent_exists = store.exists(&format!("{path}/absent"));
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(listed, Ok(Some(vec!["an-application".to_string()])));
        assert_eq!(missing, Ok(None));
        assert_eq!(file_exists, Ok(true));
        assert_eq!(absent_exists, Ok(false));
    }
}
