//! **The system service definitions, compiled into this binary.**
//!
//! `ADR-017` §4.1 ruled where a definition physically lives, and for a system
//! service the answer is this file:
//!
//! > **A service definition ships inside the signed artifact that provides the
//! > service. It is never a loose file.**
//!
//! Not a data file under `/system` either: that filesystem is read-only `erofs`
//! (`ADR-018` §3), so changing a service file means rebuilding the image
//! regardless — a file would buy nothing a constant does not, while costing a
//! second schema and a second parser.
//!
//! # What is here and what is in the document
//!
//! `ADR-017` §4.1's split, which `rivet-config`'s `services` module states
//! from the other side:
//!
//! | Question | Authority |
//! | :-- | :-- |
//! | *What is `rivet-mcpd`?* — program, arguments, restart policy, how readiness is known | **this file** |
//! | *Should `rivet-mcpd` run? On which address? With what limits?* | the desired-state document, `ADR-007` §7.2 |
//!
//! Only the first column is settled here. `enabled` is a *starting* value that
//! [`crate::document`] overwrites from the stored document when the machine has
//! one — see [`system_services`].
//!
//! # What has been run, and where
//!
//! `AGENTS.md` §6. Every program named here is Rivet's own Rust binary,
//! installed by `cmd/rivet-image` at the path this file spells, and started by
//! this supervisor under QEMU. `rivet-mcpd` is the one that has additionally
//! answered a workstation **from a board** (`PLAN.md` §73.1, §102.2).
//!
//! What that does **not** say is that every definition here has been exercised
//! on hardware: `rivet-bluetoothd` ships disabled and needs an adapter, and
//! `rivet-timed`'s `clock_settime` has never been observed succeeding on a Pi.
//! Each definition states its own contract and which of its rows are
//! observations rather than assumptions.

use rivet_core::deploy;
use rivet_store::ConfigStore;

use crate::kernel::Program;
use crate::service::{IMAGE_DEFINED, Readiness, RestartPolicy, ServiceDef};

/// Where `cmd/rivet-image` installs `cmd/rivet-netd`.
///
/// **The fifth spelling of a `/system/bin` path in this workspace**, and it is
/// a duplicate for exactly the reason `SUPERVISOR_PATH`, `INIT_PATH`,
/// `CLI_PATH` and `NETD_PATH` in `cmd/rivet-image` already are: a binary
/// crate has no library target, so nothing can `use` a constant out of it.
/// The fix — the paths belong in `rivet-core`,
/// which every one of these crates already depends on — is a standing debt tracked since 2026-08-23, and the tripwire below
/// (`the_netd_path_matches_the_one_the_image_installs`) holds the line
/// meanwhile by reading the other crate's source, which is the same device
/// `cmd/rivet-image`'s `boot.rs` uses against `cmd/rivet-initd`.
pub const NETD_PATH: &str = "/system/bin/rivet-netd";

/// `ADR-017` §2.1's name for it. Also the cgroup name and the notify socket
/// name (`ADR-017` §3.1.3), which is why [`ServiceDef::validate`] refuses a
/// name with a path separator in it.
///
/// **Read out of `rivet-core` rather than spelled**, on [`LAUNCHER_NAME`]'s
/// argument: `rivet_core::service::IMAGE_SERVICE_NAMES` is what an installed
/// application's name is refused against (`PLAN-Feature-Set-2026-09-12` §3),
/// and two spellings of this name is the failure `ADR-011` names.
pub const NETD_NAME: &str = rivet_core::service::NETD_SERVICE_NAME;

/// Where `cmd/rivet-image` installs `cmd/rivet-timed`. **Read out of
/// `rivet-core`**, which names it in `IMAGE_SERVICE_HOMES`.
///
/// `cmd/rivet-image` still spells it, and the tripwire
/// `the_timed_path_matches_the_image_if_the_image_installs_it` below reads
/// that crate's source and asserts the two agree.
pub const TIMED_PATH: &str = rivet_core::service::TIMED_PATH;

/// `ADR-021` §7's name for it. Also the cgroup name and the notify socket name.
///
/// **Read out of `rivet-core`**, [`NETD_NAME`]'s reason.
pub const TIMED_NAME: &str = rivet_core::service::TIMED_SERVICE_NAME;

/// **The services that keep a machine reachable**, and which a `SIGHUP` reload
/// therefore may not stop. `ADR-032` §3.2.1, ruled 2026-08-26.
///
/// `ADR-018` §8's argument is the whole of it and this project has now made it
/// three times: **a machine that is reachable is one somebody can repair.**
/// `crate::document::load` refuses to stop on an unreadable document for that
/// reason; `crate::supervise::Supervisor::set_application_enabled` refuses
/// `Origin::Image` for that reason; and a reload that honoured
/// `[services.rivet-mcpd] enabled = false` on a headless board would produce the
/// one failure `PLAN.md` §28.2 names as unrecoverable — a machine that is up and
/// cannot be reached, with **no next reset to roll anything back.**
///
/// # What is in it, and the test that decides
///
/// Not "system services": `rivet-timed` is `Origin::Image` and stopping it
/// costs a clock. The test is **whether stopping this service takes away the
/// means of undoing that**, and exactly two do:
///
/// * [`MCPD_NAME`] — the control plane, **which is the administration surface**
///   (`ADR-035` §3). Without it there is no way in at all: `cargo rivet run`
///   answers a workstation from a board over TLS, and it is the only thing that
///   does.
/// * [`NETD_NAME`] — the interface bring-up. Without it there is no address for
///   the control plane to listen on.
///
/// **Membership is a claim about the present, and it has moved once.** This set
/// was ruled on 2026-08-26 against a different administration surface, and kept
/// naming it for three days after `ADR-035` phase 3 made the claim false. A
/// stale member is worse than a missing one, because withholding is how this set
/// works: the supervisor would be defending a channel against the operator.
/// `the_reachability_set_is_the_control_plane_and_the_interface` is what pins it
/// now.
///
/// # It withholds rather than refuses, and the difference is the point
///
/// A document naming one of these is **not** an error and is not discarded —
/// `ADR-007` §3.6, the system never accepts an input it will discard. The value
/// is kept, reported at `Severity::Warn` naming what was withheld and what to do
/// instead, and **applied at the next boot**, when `crate::main` reads the
/// document with a console attached and a firmware rollback still underneath.
/// That is the same shape `set_application_enabled` already has, said out loud.
pub const REACHABILITY_SET: &[&str] = &[MCPD_NAME, NETD_NAME];

/// **The supervisor's own `oom_score_adj`: the kernel never chooses it** when
/// the machine runs out of memory. `ADR-017` §3.
///
/// The supervisor is what restarts everything else, so a machine that lost it
/// to the OOM killer would lose every service's restart with it; `-1000` is the
/// kernel's own value for "never".
pub const SUPERVISOR_OOM_SCORE_ADJ: i32 = -1000;

/// **The control plane's: chosen almost last**, after every application and
/// every other image service. `ADR-017` §3.
///
/// Not `-1000`, because a control plane that had grown without bound is a
/// fault the machine should still be able to survive; but it is the way in, and
/// a machine whose memory ran out should be one an operator can still reach.
pub const MCPD_OOM_SCORE_ADJ: i32 = -900;

/// **The `oom_score_adj` the supervisor gives `service` at its spawn**:
/// [`MCPD_OOM_SCORE_ADJ`] for the control plane, `0` for everything else.
///
/// Every service is given one, not only the control plane, because a child
/// inherits its parent's value and the parent is the supervisor at
/// [`SUPERVISOR_OOM_SCORE_ADJ`]. Without the reset every application would be
/// as immune to the OOM killer as the supervisor is, which is the opposite of
/// the point.
#[must_use]
pub fn oom_score_adj(service: &str) -> i32 {
    if service == MCPD_NAME { MCPD_OOM_SCORE_ADJ } else { 0 }
}

/// The time servers a Rivet image ships, **by address**.
///
/// # Why addresses and not names
///
/// The whole argument is `cmd/rivet-timed/src/args.rs`'s, because that is
/// where the parser enforces it; the one-line form is that
/// **`/etc/resolv.conf` is not in a Rivet image** (`rivet_core::etc` leaves
/// it out deliberately) and **musl with no `resolv.conf` sends every query to
/// `127.0.0.1`** (`ADR-008` §5.1.1, read out of the linked `libc.a`), where
/// nothing on a Rivet board listens. A hostname here would be a service that
/// fails identically for ever on a perfectly healthy network.
///
/// # Which addresses, and what is known about them
///
/// **Anycast addresses from two providers**, because anycast is the shape that
/// survives being pinned: one address is many servers, chosen by routing, so a
/// pinned anycast address does not go stale the way a pinned pool member does.
/// Pinning members of `pool.ntp.org` would be both brittle and against its
/// operators' stated wishes, which is why the pool is not used at all.
///
/// | Address | Published as | Provider |
/// | :-- | :-- | :-- |
/// | `162.159.200.1` | `time.cloudflare.com` | Cloudflare |
/// | `162.159.200.123` | `time.cloudflare.com` | Cloudflare |
/// | `216.239.35.0` | `time.google.com` | Google |
/// | `216.239.35.4` | `time.google.com` | Google |
///
/// **Where those four values came from, and what has been read rather than
/// looked up** — `AGENTS.md` §5 and §7 apply to the table and it must be read
/// with them.
///
/// The addresses themselves are **documentation-derived**: they are what the two
/// providers publish for their public NTP service, and nothing here resolved a
/// name to obtain them.
///
/// **All four have been contacted.** `rivet-timed --once` was run against each
/// of them on 2026-08-23, from the development host under WSL — not from a
/// board and not through `bcmgenet`:
///
/// | Address | Answered | Round trip | Stratum |
/// | :-- | :-- | --: | --: |
/// | `162.159.200.1` | yes | 20 ms | 3 |
/// | `162.159.200.123` | yes | 25 ms | 3 |
/// | `216.239.35.0` | yes | 54 ms | 1 |
/// | `216.239.35.4` | yes | 48 ms | 1 |
///
/// **All four agreed with each other to within 5 ms**, which is the result worth
/// keeping: two providers' networks independently corroborating one offset is
/// what says the answers are real rather than merely present.
///
/// **What is still unverified**: no *Rivet* machine has sent one of these
/// packets — the run above was a host binary on a development machine, and the
/// board reaches the network through a different NIC, a different route and
/// `ADR-004`'s bring-up. A stale address here is a service that reports
/// `unreachable` rather than one that reports a wrong time;
/// `cmd/rivet-timed`'s summary record names the failure kind for exactly this
/// reason.
///
/// # Two providers, and the cost of mixing them
///
/// Two prefixes from two organisations, so that one provider's outage is not
/// this machine's outage. The cost is that both providers are documented to
/// **smear leap seconds** — spreading the extra second over a window rather
/// than repeating it — and **nothing here has verified that their smear windows
/// agree**. If they do not, the two disagree by up to one second for about a
/// day, once every few years. With `rivet-timed`'s noise floor at one second
/// that is at most one extra step, recorded like any other; it is written down
/// because it is the only known way these four addresses can disagree about
/// what time it is. And because `rivet-timed` steps the clock only when two
/// different servers agree, a smear disagreement also withholds a step whenever
/// the second server to answer is the other provider's, so the clock keeps its
/// time until two servers agree again.
///
/// # This is a definition key today and `ADR-021` §7 says it should not be
///
/// §7 lists *"servers, poll interval, enabled"* as **configuration**, and
/// `ADR-017` §4.1's third clause agrees in the abstract — it is a policy a
/// machine's owner is entitled to set. It is here because `ADR-007`'s schema has
/// no `[time]` table to put it in, and inventing a compiled-in default inside
/// `cmd/rivet-timed` would have been a *second* place that answers the same
/// question (`ADR-011`). One authority, in the wrong place, beats two.
pub const TIME_SERVERS: &[&str] = &[
    "162.159.200.1",
    "162.159.200.123",
    "216.239.35.0",
    "216.239.35.4",
];

/// Where `cmd/rivet-image` installs `cmd/rivet-mdnsd`. **Read out of
/// `rivet-core`**, on [`PROXYD_PATH`]'s footing, so it needs no tripwire.
pub const MDNSD_PATH: &str = rivet_core::service::MDNSD_PATH;

/// `PLAN.md` §23.1 item 1's name for it.
///
/// **Read out of `rivet-core`**, [`NETD_NAME`]'s reason.
pub const MDNSD_NAME: &str = rivet_core::service::MDNSD_SERVICE_NAME;

/// Where `cmd/rivet-image` installs `cmd/rivet-mcpd`.
///
/// Spelled here as well as in `cmd/rivet-image`, for [`NETD_PATH`]'s reason: a
/// binary crate has no library target, so nothing can `use` a constant out of
/// it. The fix is one copy in `rivet-core`, as [`MDNSD_PATH`] has.
pub const MCPD_PATH: &str = "/system/bin/rivet-mcpd";

/// `ADR-035` phase 1's name for it.
///
/// **Read out of `rivet-core`**, [`NETD_NAME`]'s reason.
pub const MCPD_NAME: &str = rivet_core::service::MCPD_SERVICE_NAME;

pub const MCPD_IDENTITY_DIR: &str = rivet_core::state::SECRETS;

/// The pin file inside it.
pub const MCPD_PEERS_PATH: &str = "/state/secrets/peers";

/// `rivet-mdnsd` — **the responder that makes a machine reachable by name.**
///
/// `PLAN.md` §23, ruled 2026-08-25. Built 2026-08-26, once the two things it
/// was blocked on cleared on the same day: `simple-mdns` was adopted after
/// `ADR-009` §3.2's survey (§39), and the `hostname` key it needs was ruled and
/// built (§40).
///
/// # It starts `false`, ON THE MERITS
///
/// [`system_services`] lists the three kinds of starting value; this is the
/// first service to hold the third, and the distinction is load-bearing rather
/// than pedantic. A `false` about the **build** flips the day a binary ships,
/// with nobody deciding anything. **This one flips when an operator asks**, and
/// nothing about a later image should change it.
///
/// `PLAN.md` §23.1 item 2 applies `ADR-017` §6.2's bootstrap test without
/// flinching: *"a machine that cannot announce itself is still reachable by
/// address, so mDNS is useful rather than bootstrap"*. §20.2 is explicit that
/// usefulness is the criterion that would enable everything. **The document
/// turns it on.**
///
/// # `on-failure`, not `always`
///
/// The reason is what a failure here means. Every way this program refuses is a
/// **statement about the machine** rather than a transient fault: no name, no
/// address, a DHCP interface whose address is not knowable yet, two addressed
/// interfaces. Restarting it for ever would fill the journal with one
/// unchanging sentence, and `ADR-017` §3.5's give-up threshold would eventually
/// stop it anyway — having said the same thing many times instead of once.
///
/// The contrast is with a service whose clean exit means *it has stopped
/// serving*, which is [`mcpd`]'s case and takes `always`.
///
/// **A responder that dies after announcing is a different case**, and
/// `on-failure` covers it: that is a non-zero exit and is worth retrying.
///
/// # Its own uid and a ruleset, on [`dnsd`]'s terms
///
/// It parses datagrams from any device on the link, so it runs as
/// `app_identity(MDNSD_NAME)` under
/// `rivet_core::confinement::image_service_policy`, with the home an
/// application has. Nothing it does needs root: UDP 5353 is above 1024 and
/// joining a multicast group takes no capability, so it opens its own socket
/// and is handed none.
///
/// # It reads a projection, never the document or the leases
///
/// `--projection` names `rivet_core::mdns::path`, the file
/// `crate::runtime::deliver_mdns_projection` writes from the same inputs
/// [`crate::mdns::Inputs`] compares: the machine's name and each interface's
/// address, a DHCP one's from its lease. The document is not granted to any
/// confined program and the lease directory is root's. **The hostname it
/// announces is the kernel's**, taken by `uname(2)`; `cmd/rivet-mdnsd`'s
/// `announce` module carries why, and refuses when the projection names
/// another.
fn mdnsd() -> ServiceDef {
    let volume = rivet_core::service::BUILT_IN_VOLUME;
    let name = rivet_core::service::ServiceName::new(MDNSD_NAME);
    let mut program = homed_program(MDNSD_PATH, volume, name.as_ref(), &[]);
    program.args = vec![
        "--projection".to_string(),
        name.as_ref()
            .map(|n| rivet_core::mdns::path(volume, n))
            .unwrap_or_default(),
    ];
    ServiceDef {
        name: MDNSD_NAME.to_string(),
        program,
        // The home an application has: `provide_image_homes`, `/etc/passwd`
        // and the projection's delivery all key on it.
        volume: Some(volume.to_string()),
        // **The operator's decision, and only theirs.** See above.
        enabled: false,
        restart: RestartPolicy::OnFailure,
        // No readiness signal: it opens its socket, reads its projection and
        // answers. `ADR-017` §4's `none` said honestly.
        readiness: Readiness::OnStart,
        consequence: "this machine does not answer to <hostname>.local, so anything holding \
                      only its name cannot find it (ADR-005 §3's mDNS row, PLAN.md §23) - \
                      including rivet-fleet, which refuses to reach a member it has no \
                      address for and names this service as the thing that would fix it"
            .to_string(),
        ..IMAGE_DEFINED
    }
}

/// `rivet-mcpd` — **the MCP control plane** (`ADR-035` phase 1).
///
/// Built 2026-08-28 and proved on hardware the same day (`PLAN.md` §73.1): a
/// card carrying it booted, committed, and served `initialize`, `tools/list`
/// and `tools/call`. Every one of those runs was started by hand, because until
/// this definition existed nothing on a Rivet machine started it.
///
/// # It starts `true`, because it is THE way in
///
/// `ADR-023` §4.1's ruling, arriving at the program it is now about: the only
/// means of reaching a freshly flashed, headless Pi has to be on by default.
/// Since `ADR-035` phase 3 that means is this control plane, and `ADR-017`
/// §6.2's bootstrap test therefore answers *bootstrap* rather than *useful* —
/// a machine that cannot be reached at all is not recoverable. It is
/// [`netd`]'s kind of `true`, one layer up: `netd` gets the address, this
/// answers on it.
///
/// # The flip was three phases late, and only a CARD could have shown it
///
/// This definition once said in its own words *"it flips to `true` in phase
/// 2"*. Phases 2 **and** 3 both completed and were proven on hardware, and the
/// value never moved. **That is `PLAN.md` §94.7 exactly** — a decision whose
/// trigger for being revisited is somebody happening to remember — and this
/// instance cost more than the audit's other examples, because while it stood
/// **a card built from these defaults booted with no control plane at all.**
///
/// A running board could never have shown it, because a board runs the
/// *document*, and the document has enabled this since phase 2. Only a freshly
/// flashed card would, and every card in that window was flashed by somebody
/// who then deployed through a control plane the document had already turned
/// on. `timed_is_enabled_exactly_when_the_image_installs_it` asserts the whole
/// enabled set for this reason, and it is the only place that difference is
/// visible.
///
/// # `always`, not `on-failure`
///
/// **A server that exits cleanly has not finished; it has stopped serving.**
/// So a clean exit here is something unexpected and restarting is the honest
/// response. [`mdnsd`]'s opposite reasoning does not transfer — its failures
/// are *statements about the machine* that repeat unchanged, where this one's
/// clean exit is simply the end of service.
///
/// `ADR-017` §3.5's give-up threshold still applies, and that is deliberate: a
/// listener that cannot bind fails identically every time, and §3.5's answer to
/// that is to stop and report rather than to keep writing to the card.
///
/// # uid 0, and this is the one that would be tempting to get wrong
///
/// It hands each connection the stdin and stdout of `/system/bin/rivet mcp`,
/// and that program installs payloads, arms trial boots and restarts the
/// machine. **A control plane that dropped privilege could not control**, so
/// there is no unprivileged identity to give it — see `IMAGE_DEFINED`.
///
/// What makes that safe over a network is `ADR-035` §7's mutual authentication:
/// TLS with pinned Ed25519 keys on both sides, so the listener is not an
/// unauthenticated surface even though it is not loopback.
///
/// # No readiness
///
/// `ADR-017` §4.2's `none`. The `listening` value needs §3.2's pre-created
/// socket, which this crate does not have, so the supervisor reports `running`
/// and never `running, ready` — §3.3's distinction kept rather than papered
/// over.
fn mcpd() -> ServiceDef {
    ServiceDef {
        name: MCPD_NAME.to_string(),
        program: Program {
            path: MCPD_PATH.to_string(),
            args: vec![
                // **A path, not an address.** `ADR-017` §4.1 keeps a service's
                // argument vector the image's, so the supervisor may not read
                // the operator's chosen address and pass it — the program reads
                // it, exactly as `netd` and `mdnsd` read theirs. The constant
                // that used to be here is gone: `rivet_config::control_plane`
                // is where the address lives now, which is what `MCPD_LISTEN`'s
                // own documentation said would happen the day TLS landed.
                "--document".to_string(),
                ConfigStore::at_default().machine_path(),
                "--identity".to_string(),
                MCPD_IDENTITY_DIR.to_string(),
                "--peers".to_string(),
                MCPD_PEERS_PATH.to_string(),
            ],
            env: Vec::new(),
            // uid 0: see above. It spawns the CLI that installs, arms and
            // reboots.
            identity: None,
            confinement: None,
            groups: Vec::new(),
            capabilities: Vec::new(),
        },
        // **The operator's, as `enabled` always is** — but its *starting* value
        // is the roadmap's, and since 2026-08-29 the roadmap says `true`. See
        // above for why that flip is three phases late.
        enabled: true,
        restart: RestartPolicy::Always,
        readiness: Readiness::OnStart,
        consequence: "there is no control plane, and since ADR-035 phase 3 that is the whole \
                      administration surface — so a headless board cannot be reached at all, \
                      by anything. This service is in REACHABILITY_SET for that reason"
            .to_string(),
        ..IMAGE_DEFINED
    }
}

/// **The services Rivet defines in the image**, in the order a boot converges
/// them.
///
/// `enabled` here is the *definition's* value rather than the operator's.
/// [`crate::document::apply_desired_state`] replaces it with the stored
/// document's wherever the document names the service — `ADR-017` §6 makes
/// `enabled` the entire control surface, so a machine that ignored it would be
/// running services its own configuration says are off.
///
/// # The three kinds of starting value, and keeping them distinct matters
///
/// * **`true` because the machine would otherwise be unrecoverable.**
///   [`mcpd`] and [`netd`]. `ADR-023` §4.1 is the ruling: the only means of
///   reaching a freshly flashed, headless Pi has to be on by default, or the
///   board is up and nobody can talk to it. That is [`REACHABILITY_SET`]'s
///   argument stated as a default rather than as a withholding.
/// * **`false` about the build.** Nothing is in this state today, and the
///   category is kept because it is the one that flips without anybody
///   deciding: a definition enabled while the image installs no binary is a
///   permanent spawn refusal on every boot of every board.
///   `timed_is_enabled_exactly_when_the_image_installs_it` is the assertion
///   that holds both directions.
/// * **`false` on the merits.** [`mdnsd`] and [`bluetoothd`]. `ADR-017` §6.2's
///   bootstrap test says a machine without either is still reachable,
///   updatable and administrable, so both are *useful* rather than
///   *bootstrap*. **The document turns them on**, and nothing about a later
///   image should change these values.
///
/// A card runs these defaults; a board runs its document. That difference is
/// why a stale starting value can survive for days on machines that all behave
/// correctly — `PLAN.md` §94.7, and
/// `timed_is_enabled_exactly_when_the_image_installs_it` asserts the whole
/// enabled set for exactly that reason.
pub fn system_services() -> Vec<ServiceDef> {
    vec![
        netd(),
        timed(),
        mdnsd(),
        mcpd(),
        bluetoothd(),
        // **Before the launcher, and the order is load-bearing.** `ADR-058`
        // §4.1 makes the launcher `displayd`'s first client, and a client that
        // starts first finds no socket. The supervisor starts this set in
        // order, so the compositor is listed ahead of the thing that connects
        // to it — the same reason `netd` is ahead of `mdnsd`.
        displayd(),
        launcher(),
        proxyd(),
        dnsd(),
    ]
}

/// Where `cmd/rivet-image` installs `cmd/rivet-dnsd`. `ADR-047`. **Read out of
/// `rivet-core`**, on [`PROXYD_PATH`]'s footing, so it needs no tripwire.
pub const DNSD_PATH: &str = rivet_core::service::DNSD_PATH;

/// The name a document turns it on by. **Read out of `rivet-core`**,
/// [`NETD_NAME`]'s reason.
pub const DNSD_NAME: &str = rivet_core::service::DNSD_SERVICE_NAME;

/// `rivet-dnsd` — **the machine answers for the fleet's names and forwards the
/// rest.** `ADR-047`.
///
/// # `enabled: false`, on the merits
///
/// [`mdnsd`]'s kind of `false`. A machine that answers no names is reachable,
/// updatable and administrable in every way, so this is *useful* rather than
/// *bootstrap* (`ADR-017` §6). It is also the one service whose failure is
/// felt by OTHER machines — every device pointed at it loses name resolution
/// when it stops — so it runs only where a document has asked for it, and no
/// update turns it on for a machine that did not.
///
/// # Two sockets on 53, bound by the supervisor
///
/// UDP for ordinary queries and TCP because a truncated answer is retried over
/// TCP to the same server (`ADR-047` §3). Both on every address, and not
/// `proxied`: a resolver is asked directly by the devices on the network.
///
/// # Its own uid and a ruleset, on [`proxyd`]'s terms
///
/// This is the one service here that parses datagrams from any device on the
/// network and answers from the internet, so it runs as `app_identity
/// (DNSD_NAME)` under `rivet_core::confinement::image_service_policy`, with
/// the home an application has. A defect in its query path then reaches only
/// what that ruleset grants: its own home, read-only views, and the sockets
/// it was handed.
///
/// # It reads a projection, never the machine document
///
/// `--document` names `rivet_core::name_service::path`, the file
/// `crate::runtime::deliver_name_service_document` writes: `version`, this
/// machine's hostname and the `[dns]` table, rendered by the document's own
/// serialiser. The document itself under `/state/config` is not granted to
/// any confined program (`ADR-047` §6).
///
/// # The sockets are bound by the supervisor
///
/// A process that is not root cannot bind port 53, so the handoff of both
/// sockets is what makes the identity possible. The program binds nothing.
///
/// # `on-failure`
///
/// Every way it exits before serving is a statement about the machine — no
/// `[dns]` table, no hostname, no handed socket — which repeats unchanged, so
/// `always` would only restate it. A server that dies while serving exits
/// non-zero and is restarted.
fn dnsd() -> ServiceDef {
    let volume = rivet_core::service::BUILT_IN_VOLUME;
    let name = rivet_core::service::ServiceName::new(DNSD_NAME);
    let mut program = homed_program(DNSD_PATH, volume, name.as_ref(), &[]);
    program.args = vec![
        "--document".to_string(),
        name.as_ref()
            .map(|n| rivet_core::name_service::path(volume, n))
            .unwrap_or_default(),
    ];
    ServiceDef {
        name: DNSD_NAME.to_string(),
        program,
        enabled: false,
        restart: RestartPolicy::OnFailure,
        readiness: Readiness::OnStart,
        consequence: "this machine answers no DNS queries, so every device pointed at it as a \
                      resolver can resolve neither the fleet's home.arpa names nor anything \
                      else until it is pointed elsewhere (ADR-047)"
            .to_string(),
        // The home an application has: `provide_image_homes`, `/etc/passwd`
        // and the projection's delivery all key on it.
        volume: Some(volume.to_string()),
        listen: vec![
            rivet_core::package::application::Socket::Udp(53),
            rivet_core::package::application::Socket::Tcp(53),
        ],
        ..IMAGE_DEFINED
    }
}

/// **The program of an image service that runs under its own identity** —
/// `rivet_core::service::IMAGE_SERVICE_HOMES`, `ADR-060`.
///
/// Its uid is the one `app_identity` derives from its name, its ruleset is
/// `rivet_core::confinement::image_service_policy`, and its environment is the
/// one an application is handed — `RIVET_APP_DATA`, `RIVET_APP_SETTINGS`,
/// `RIVET_APP_TMP` and `TMPDIR`, from the same `rivet_core` functions
/// `crate::applications::definition` uses. No arguments; the caller adds any.
///
/// The ruleset is composed with this process's pid because this process is the
/// supervisor — the same argument `crate::applications` makes. `devices` are
/// the classes the definition declares, which the ruleset reads exactly as an
/// application's declares them; the caller puts the same list in the
/// definition's `devices`, so the nodes it expands to are granted beside it.
fn homed_program(
    path: &str,
    volume: &str,
    name: Option<&rivet_core::service::ServiceName>,
    devices: &[rivet_core::package::application::Device],
) -> Program {
    let env = name
        .map(|n| {
            let scratch = rivet_core::service::tmp_dir(volume, n);
            vec![
                (
                    rivet_core::service::APP_DATA_ENV.to_string(),
                    rivet_core::service::data_dir(volume, n),
                ),
                (
                    rivet_core::service::APP_SETTINGS_ENV.to_string(),
                    rivet_core::settings::path(volume, n),
                ),
                (rivet_core::service::APP_TMP_ENV.to_string(), scratch.clone()),
                (rivet_core::service::TMPDIR_ENV.to_string(), scratch),
            ]
        })
        .unwrap_or_default();
    Program {
        path: path.to_string(),
        args: Vec::new(),
        env,
        identity: name.map(rivet_core::service::app_identity),
        confinement: name.map(|n| {
            rivet_core::confinement::image_service_policy(
                path,
                volume,
                n,
                Some(std::process::id()),
                devices,
            )
        }),
        groups: Vec::new(),
        capabilities: Vec::new(),
    }
}

/// Where `cmd/rivet-image` installs `cmd/rivet-proxyd`. `ADR-060`.
///
/// **Read out of `rivet-core`**, which both crates depend on, so this path is
/// spelled once and needs no tripwire.
pub const PROXYD_PATH: &str = rivet_core::service::PROXYD_PATH;

/// The name a document turns it on by, and the name of its home under
/// `/state/app`. **Read out of `rivet-core`**, [`NETD_NAME`]'s reason.
pub const PROXYD_NAME: &str = rivet_core::service::PROXYD_SERVICE_NAME;

/// `rivet-proxyd` — **the machine's front door, and part of the operating
/// system.** `ADR-060`.
///
/// # An image service that runs in user space
///
/// Every other definition in this file is uid 0 with no confinement, and
/// `IMAGE_DEFINED` says why that is right for them: each does root's work.
/// This one does not — it is handed a listening socket, reads its certificate
/// and forwards bytes — so it keeps the identity and the ruleset it had as a
/// carried application, and moving it into the image changes who ships it and
/// nothing about what it may reach:
///
/// * the uid is `app_identity(PROXYD_NAME)`, the one it ran as before, so the
///   certificate and key already in its data directory stay readable;
/// * the ruleset is `rivet_core::confinement::image_service_policy`, the
///   application rows with its program file in place of `current`;
/// * `volume` is the built-in volume, which is what gives it the home an
///   application has — `crate::applications::provide_image_homes` makes it
///   at boot, and `/etc/passwd` names the account.
///
/// # The environment an application is handed
///
/// `RIVET_APP_DATA`, `RIVET_APP_SETTINGS`, `RIVET_APP_TMP` and `TMPDIR`, from
/// the same `rivet_core` functions `crate::applications::definition` uses, so
/// the proxy's own code does not know which way it was delivered. No
/// `RIVET_APP_DIR`: there is no installed version directory to name.
///
/// # What was the manifest's, restated
///
/// `restart = "on-failure"`, `listen = ["tcp:443"]` on every address, and the
/// one setting it understands — `cmd/rivet-proxyd/Cargo.toml` still carries
/// the manifest, and `the_proxy_definition_matches_the_manifest_it_replaces`
/// reads it so the two cannot drift. `ready = "listening"` there is
/// [`Readiness::OnStart`] here: the socket is bound by the supervisor before
/// the spawn, so a listening readiness would say nothing the start does not.
///
/// # `enabled: false`, and the role turns it on
///
/// `ADR-050` §2's tier 2, on [`launcher`]'s terms: a fleet server's and a
/// fleet member's documents enable it (`ADR-050` §5), and a stand-alone
/// machine that serves nothing leaves it off.
fn proxyd() -> ServiceDef {
    let volume = rivet_core::service::BUILT_IN_VOLUME;
    let name = rivet_core::service::ServiceName::new(PROXYD_NAME);
    ServiceDef {
        name: PROXYD_NAME.to_string(),
        // **Its own uid, the one it ran as when it was carried.** Not root:
        // nothing it does needs root, and the files it already owns are owned
        // by this uid.
        program: homed_program(PROXYD_PATH, volume, name.as_ref(), &[]),
        enabled: false,
        restart: RestartPolicy::OnFailure,
        readiness: Readiness::OnStart,
        consequence: "nothing on this machine answers on 443, so no application behind the \
                      proxy can be reached by a browser and the operator's certificate is \
                      checked by nobody. The control plane and the fleet portal on 4443 are \
                      unaffected (ADR-060)"
            .to_string(),
        // The home an application has, which is what `provide_image_homes`,
        // `/etc/passwd`, the settings delivery and the routing table all key on.
        volume: Some(volume.to_string()),
        // **Every address**, and no `proxied`: it is the proxy, not behind it
        // (`ADR-046` §7).
        listen: vec![rivet_core::package::application::Socket::Tcp(443)],
        ..IMAGE_DEFINED
    }
}

/// Where `cmd/rivet-image` installs `cmd/rivet-displayd`. `ADR-058` §4.
///
/// **Read out of `rivet-core`**, on [`PROXYD_PATH`]'s terms, because
/// `rivet_core::service::IMAGE_SERVICE_HOMES` names it too.
/// `cmd/rivet-image` spells it again, and
/// `the_displayd_path_matches_the_one_the_image_installs` is the tripwire.
pub const DISPLAYD_PATH: &str = rivet_core::service::DISPLAYD_PATH;

/// The name a document turns it on by, and the name it provides its Wayland
/// socket under.
///
/// **Read out of `rivet-core`**, on [`LAUNCHER_NAME`]'s argument exactly: an
/// application writes `connects = ["rivet-displayd:wayland"]` and the
/// supervisor has to find the provider by the same name.
pub const DISPLAYD_NAME: &str = rivet_core::compositor::PROVIDER;

/// `rivet-displayd` — **the machine has a compositor.** `ADR-058` §4, ruled
/// 2026-09-20.
///
/// # It is the program that may not fail
///
/// The maintainer, on a draft that listed the launcher's being unable to fail as
/// a cost, ruled that system programs often cannot be allowed to fail
/// (.rulings/ R-310). So the screen was split off from the menu, and this is the
/// half that holds it. `RestartPolicy::Always`, like the launcher — but where a launcher
/// restart costs a redrawn menu, a restart here costs every client its surface,
/// so the program is written to `ADR-017` §3 to make it not happen.
///
/// # `enabled: false`, like every tier 2 service
///
/// `ADR-050` §2's rule and [`launcher`]'s reasoning: a headless machine leaves
/// the launcher off and a kiosk machine adds it, and a compositor on a headless
/// machine is the same wasted program as a menu on one. The document decides.
///
/// # It provides a socket and connects to nothing
///
/// The reverse of the launcher's shape, and that is `ADR-058` §4's split in
/// one line: mechanism provides, policy consumes. The supervisor binds the
/// Wayland socket, hands over the descriptor, and chowns it to whichever
/// application declared `connects = ["rivet-displayd:wayland"]` — the ordinary
/// `provides` path that `rivet-bluetoothd` established, with no special case.
///
/// # Its own uid and a ruleset, on [`proxyd`]'s terms
///
/// Every client's bytes reach this program — surfaces, dma-bufs, protocol
/// messages — so it runs as `app_identity(DISPLAYD_NAME)` under
/// `rivet_core::confinement::image_service_policy`, with the home an
/// application has. Nothing it does needs root:
///
/// * **DRM master** is the kernel's to give the first process that opens a
///   primary node while none holds it, with no capability asked; this program
///   opens the node once and holds it for life, and never drops master to take
///   it back, which is the one act that needs `CAP_SYS_ADMIN`.
/// * **The card and render nodes** are the `display` class's arbiter set,
///   chowned to its uid by the supervisor before the spawn and appended to the
///   ruleset (`ADR-015` §6).
/// * **Mesa's device lookup** — the sysfs tree, `/dev/dri` listed, and the GBM
///   backend — is granted because it declares `display`, by the function that
///   grants it to an application making the same declaration.
/// * **The kernel's uevents**, for hot-plug, are open to an unprivileged
///   socket: the kernel's uevent netlink family lets any process join its
///   multicast group.
/// * **The capture file** is in its own data directory.
/// * **Both sockets** are bound by the supervisor and handed over.
///
/// # `display` and not `input`
///
/// It reads no input device: its seat carries no keyboard and no pointer, and
/// the gesture vocabulary reaches the launcher, which declares `input`. Under
/// its own uid a declared class is a chown of every node the class expands to,
/// so declaring `input` here would hand it every keyboard on the machine for
/// nothing.
fn displayd() -> ServiceDef {
    let volume = rivet_core::service::BUILT_IN_VOLUME;
    let name = rivet_core::service::ServiceName::new(DISPLAYD_NAME);
    let devices = vec![rivet_core::package::application::Device::Display];
    ServiceDef {
        name: DISPLAYD_NAME.to_string(),
        // No arguments, on [`launcher`]'s argument: it is handed its socket
        // rather than opening a path, and it enumerates the DRM device rather
        // than being told which one.
        program: homed_program(DISPLAYD_PATH, volume, name.as_ref(), &devices),
        // The home an application has, which `provide_image_homes` and
        // `/etc/passwd` key on, and where the capture file is written.
        volume: Some(volume.to_string()),
        enabled: false,
        restart: RestartPolicy::Always,
        // **`OnStart` and it is a lie this one may grow out of.** A client that
        // connects before the socket is accepting gets `ECONNREFUSED`, and the
        // launcher is started immediately after this. The socket is BOUND by
        // the supervisor before either process exists, so a connect queues
        // rather than fails — which is why `OnStart` is honest today and why
        // the fix, if it is ever needed, is a readiness notification rather
        // than a sleep.
        readiness: Readiness::OnStart,
        consequence: "this machine has a GPU and a screen and nothing driving them: no \
                      application can be handed a surface, so a kiosk shows nothing at all. \
                      Nothing else is affected - the control plane, the network and every \
                      headless service are untouched (ADR-058 4.1)"
            .to_string(),
        // **Two sockets, both bound by the supervisor and handed over**, folded
        // into `listen` exactly as an application's `provides` is.
        //
        // `wayland` is the protocol a client draws through. `capture` is
        // `ADR-058` §5's screenshot — a separate endpoint on purpose, because
        // the caller is an OPERATOR over the control plane rather than a
        // client, and an answer that routed through a client would be
        // unavailable exactly when a client is wedged.
        listen: vec![
            rivet_core::package::application::Socket::Unix(
                rivet_core::compositor::ENDPOINT.to_string(),
            ),
            rivet_core::package::application::Socket::Unix(
                rivet_core::compositor::CAPTURE_ENDPOINT.to_string(),
            ),
        ],
        // **`wayland` is SHARED and `capture` is not**, and the difference is
        // the whole of `.rulings/` R-206's endpoint half.
        //
        // A compositor exists so that SEVERAL applications hold surfaces and it
        // arbitrates between them; an endpoint only one application may reach
        // makes that impossible by construction — which is what `rivet-01`
        // reported the moment a second application wanted a surface. So this
        // one takes as many consumers as declare it, through a supplementary
        // group rather than a chown (`Endpoint::shared`).
        //
        // `capture` stays one-to-one deliberately. Its caller is an OPERATOR
        // over the control plane, there is one of those on a machine, and
        // widening it would hand every application on the board a way to
        // photograph the screen.
        provides: [
            format!("socket:{}:shared", rivet_core::compositor::ENDPOINT),
            format!("socket:{}", rivet_core::compositor::CAPTURE_ENDPOINT),
        ]
        .iter()
        .filter_map(|spec| {
            rivet_core::package::application::Endpoint::parse("provides", spec).ok()
        })
        .collect(),
        // **Nothing.** It is the provider; it asks the machine for no service.
        connects: Vec::new(),
        devices,
        ..IMAGE_DEFINED
    }
}

/// Where `cmd/rivet-image` installs `cmd/rivet-launcher`. `ADR-050` §2.
///
/// The ninth spelling of a `/system/bin` path in this workspace, on
/// [`BLUETOOTHD_PATH`]'s terms exactly, and
/// `the_launcher_path_matches_the_one_the_image_installs` is the tripwire that
/// keeps the two agreeing.
pub const LAUNCHER_PATH: &str = "/system/bin/rivet-launcher";

/// The name a document turns it on by, and the name it provides its own input
/// endpoint under.
///
/// **Read out of `rivet-core` rather than spelled**, on [`BLUETOOTHD_NAME`]'s
/// argument: `rivet_core::input::PROVIDER` is already the name the agent's
/// press endpoint is filed under, and two spellings of a service that provides
/// an endpoint is the failure `ADR-011` names.
pub const LAUNCHER_NAME: &str = rivet_core::input::PROVIDER;

/// `rivet-launcher` — **the machine offers a menu.** `ADR-050` §2, ruled
/// 2026-09-07.
///
/// # Tier 2, which is the whole of the decision
///
/// `ADR-050` §2's axis is *what decides whether it runs*, and for this it is
/// **the document**, exactly as for [`mdnsd`] and [`bluetoothd`]. The
/// maintainer's own rule is the specification: provisioning a headless machine
/// leaves the launcher off and provisioning a kiosk adds it (.rulings/ R-304) —
/// which is `enabled`, a mechanism this project already had, and is why §2.1
/// says nothing new had to be built.
///
/// # It was an application until this ADR, and what the move costs
///
/// `cmd/rivet-launcher`'s own header records the 2026-09-01 ruling that the
/// launcher makes more sense as an application (.rulings/ R-305), and
/// `deploy/launcher/Cargo.toml` is the manifest that shipped it as one.
/// `ADR-050` §2 supersedes the placement on
/// two grounds: the launcher is versioned with the operating system, and
/// whether a machine offers a menu is a property of the machine rather than of
/// a program somebody installed. What it buys is `SPEC-05` §1 for a kiosk
/// card — a menu on the first boot rather than after a deploy.
///
/// **What it costs is this function**, and it is stated because it is the kind
/// of thing that drifts. Every line below restates something
/// `deploy/launcher/Cargo.toml` declares, and an image service has no manifest
/// for `crate::applications` to read it out of. The two are checked against
/// each other by `the_launcher_definition_matches_the_manifest_it_replaces`,
/// which reads that file rather than trusting this comment.
///
/// # `connects`, and it is the line that decides whether anything is drawn
///
/// `.rulings/` R-138: an application declaring the kiosk endpoint is the
/// display **ARBITER** and is granted `/dev/dri/card*`; one that does not is a
/// lessee with a render node and nothing to modeset. `Supervisor::display_role`
/// reads exactly this field, so deleting the line would leave the same binary
/// on the same card drawing nothing, silently. That failure is why
/// `deploy/launcher/Cargo.toml` argues at length for being tracked, and it is
/// why the line is here rather than implied.
///
/// **This is the first image service to declare `connects`**, and
/// `crate::service::IMAGE_DEFINED` says why the field is normally empty: a
/// system service runs as uid 0 and reaches any socket already, so the grant is
/// a no-op. That is still true of the *grant* — what is not a no-op is the
/// arbitration, which is decided from this field and not from a uid.
///
/// # `devices`, which grants nothing and is not decoration
///
/// `Supervisor::converge` skips the device grant for a definition with no
/// `identity`, and this has none: an image service is uid 0, which already
/// opens a `0600 root:root` devtmpfs node. The classes are declared anyway
/// because `crate::devices`' conflict check and the journal line that reports
/// what holds the screen both read this field, and a machine whose launcher
/// held the display while claiming no device class would be lying in its own
/// records.
///
/// # `always`, and no readiness
///
/// `restart = "always"` is the manifest's, and it is right for the same reason
/// there: a launcher that exits has left the machine with a screen nobody
/// drives, and every way it can exit is worth another attempt. `ready = none`
/// likewise — it draws when it draws, and there is no signal for it.
fn launcher() -> ServiceDef {
    ServiceDef {
        name: LAUNCHER_NAME.to_string(),
        program: Program {
            // No arguments and no environment, on [`bluetoothd`]'s argument:
            // it is handed its input listener rather than opening a path
            // (`ADR-036` §5), and it asks the supervisor for the offered
            // list over the kiosk socket rather than reading the document.
            args: Vec::new(),
            env: Vec::new(),
            path: LAUNCHER_PATH.to_string(),
            // uid 0, like every other image service. **This is a real change
            // from the application form**, where `ADR-015` §5's per-name uid
            // applied, and it is the honest consequence of tier 2 rather than a
            // shortcut: an image service has no package to derive an identity
            // from, and `crate::service::IMAGE_DEFINED` records that the whole
            // set runs as root.
            identity: None,
            // `None`, like every other image service, and
            // `bluetoothd`'s comment carries the reason in full: `ADR-042`'s
            // ruleset is computed from an application MANIFEST and an image
            // service has none, so a policy written here would be this file's
            // guess at what the program touches.
            confinement: None,
            groups: Vec::new(),
            capabilities: Vec::new(),
        },
        // **The operator's decision, and only theirs.** `ADR-050` §2 — the
        // document turns it on, and *"headless leaves launcher off"* is this
        // value.
        enabled: false,
        restart: RestartPolicy::Always,
        readiness: Readiness::OnStart,
        consequence: "this machine has a screen and nothing driving it: no menu, and no way to \
                      choose a kiosk application from the console. Nothing else is affected — \
                      an application with `auto-run` still takes the display, and the control \
                      plane is untouched (ADR-050 §2)"
            .to_string(),
        // **The agent's press**, and it is bound by the supervisor and handed
        // over rather than opened. Folded into `listen` exactly as an
        // application's `provides` is, so the socket is bound by the one path
        // that already does that.
        listen: vec![rivet_core::package::application::Socket::Unix(
            rivet_core::input::ENDPOINT.to_string(),
        )],
        provides: rivet_core::package::application::Endpoint::parse(
            "provides",
            &format!("socket:{}", rivet_core::input::ENDPOINT),
        )
        .into_iter()
        .collect(),
        // **Two now, and the second one is the screen** — `ADR-058` §4.
        //
        // `rivet-sysd:kiosk` is unchanged and still says what this program is
        // for: it selects applications. What it no longer means is *this
        // program drives the display* — `Supervisor::display_role` reads the
        // compositor's `provides` for that since 2026-09-20, and the comment
        // that used to sit here calling this line "what makes this the display
        // arbiter" was true under `.rulings/` R-138 and is not now.
        //
        // `rivet-displayd:wayland` is how the launcher reaches the screen at
        // all. **Declaring it is what makes a missing compositor a NAMED
        // failure**: an endpoint nobody provides is something the supervisor
        // can report, where a launcher that simply found no socket would draw
        // nothing and say nothing.
        connects: [
            format!(
                "{}:{}",
                rivet_core::kiosk::PROVIDER,
                rivet_core::kiosk::ENDPOINT
            ),
            format!(
                "{}:{}",
                rivet_core::compositor::PROVIDER,
                rivet_core::compositor::ENDPOINT
            ),
            // **The capture socket** — `ADR-058` §5. `apply input capture`
            // arrives here because this program provides the input endpoint,
            // and this is how it is forwarded to the one program that can see
            // the whole screen.
            format!(
                "{}:{}",
                rivet_core::compositor::PROVIDER,
                rivet_core::compositor::CAPTURE_ENDPOINT
            ),
        ]
        .iter()
        .filter_map(|spec| {
            rivet_core::package::application::Endpoint::parse("connects", spec).ok()
        })
        .collect(),
        // **No `Display` any more, and that is the point of the whole ADR.**
        // A Wayland client needs no DRM node, no render node, no framebuffer
        // and none of `ADR-058` §1's five confinement grants. It draws into
        // a shared-memory buffer the compositor handed it.
        //
        // `Input` stays: the launcher reads `/dev/input/event*` itself for the
        // `ADR-015` gesture vocabulary. Moving that to the compositor as
        // `wl_keyboard` is a separate change and would cost the keymap dataset
        // `cmd/rivet-displayd` deliberately does not ship.
        devices: vec![rivet_core::package::application::Device::Input],
        ..IMAGE_DEFINED
    }
}

/// Where `cmd/rivet-image` installs `cmd/rivet-bluetoothd`. **Read out of
/// `rivet-core`**, which names it in `IMAGE_SERVICE_HOMES`.
pub const BLUETOOTHD_PATH: &str = rivet_core::service::BLUETOOTHD_PATH;

/// `ADR-017` §2.1's name for it, and the same string `rivet_core::bluetooth`
/// gives an application to write in its manifest.
///
/// **Read out of `rivet-core` rather than spelled**, unlike the five above, and
/// the difference is that this one is half of a contract: an application writes
/// `connects = ["rivet-bluetoothd:advertisements"]` and the supervisor has to
/// find the provider by the same name. Two spellings is the failure `ADR-011`
/// names, and `rivet_core::kiosk::PROVIDER` already sets this precedent for the
/// one endpoint the supervisor provides itself.
pub const BLUETOOTHD_NAME: &str = rivet_core::bluetooth::PROVIDER;

/// `rivet-bluetoothd` — **a Rivet reads BLE advertisements.** `ADR-041`,
/// ruled 2026-09-05 (`.rulings/` R-144).
///
/// # It is the FIRST image service that provides an endpoint
///
/// `crate::service::IMAGE_DEFINED` sets `provides` empty and says why: *"an
/// image service runs as uid 0 and reaches any socket already, so a grant would
/// be a no-op dressed as a permission."* That sentence is about a **system
/// service consuming**, and it stays true. This is the other direction — a
/// system service **providing** to an application, which runs as its own uid
/// and cannot open a `0600 root:root` socket without the grant.
///
/// It needed no new mechanism, which is `ADR-041` §6.2's whole argument.
/// `provides` is folded into `listen` here exactly as `crate::applications`
/// folds a manifest's, so the supervisor binds the socket, hands over the
/// descriptor, and chowns the file to the one consumer that declared
/// `connects`. **The one thing that is not reused is
/// `rivet_core::kiosk`'s special case**, and deliberately: that endpoint is
/// provided by the supervisor *process* and had to be skipped in
/// `crate::endpoints::resolve`. This one is provided by a service in the set,
/// so the ordinary path resolves it.
///
/// # `enabled: false`, on the MERITS
///
/// The sixth starting value in this file, and it belongs with `rivet-mdnsd`'s
/// rather than with `rivet-timed`'s. Three reasons, and each is sufficient:
///
/// 1. **`ADR-017` §6.2's bootstrap test.** A machine that is not reading
///    advertisements is reachable, updatable and administrable in every way; so
///    this is *useful*, not *bootstrap*.
/// 2. **It takes an exclusive claim on a shared part.** The adapter is the
///    Bluetooth half of the CYW43455, which is also the wireless client, and
///    what continuous LE scanning costs that pair **has not been measured**
///    (R-144.1 item 5). Enabling it by default would make every board pay an
///    unmeasured cost for a capability most of them have no consumer for.
/// 3. **With no consumer it does nothing.** An advertisement reaches an
///    application that declares `connects`, and a machine with no such
///    application would be holding a radio for nobody.
///
/// **What that costs, stated because it is the thing that will surprise
/// somebody.** An application declaring
/// `connects = ["rivet-bluetoothd:advertisements"]` on a machine where this is
/// disabled is **NOT STARTED** — `crate::endpoints::Refusal::NoSuchProvider`,
/// which now says *"no such service is enabled"* rather than *"no such
/// application is installed"* precisely because of this service. That is the
/// same shape `AGENTS.md` §9.1 records `aura-bridge` dying in, and the refusal
/// naming the document key is what makes it a five-second diagnosis instead.
///
/// # `on-failure`, and what a failure means here
///
/// Two kinds of exit, and the policy is right for both:
///
/// * **A statement about the machine**: the socket was not handed over (a
///   definition fault), or the kernel has no Bluetooth subsystem (an image
///   fault). Neither is fixed by restarting, and `ADR-017` §3.5's give-up
///   threshold ends the loop after five attempts having said the same thing
///   five times.
/// * **The controller went away, or would not be taken.** The daemon holds no
///   capability once its controller is bound, so it cannot bind another; it
///   exits non-zero and this policy brings it back holding `CAP_NET_ADMIN`
///   again. A scan that ran for `Limits::success_window_ms` wipes the give-up
///   count, so an adapter unplugged and plugged back in, however often, is
///   restarted every time; only a controller lost five times inside that
///   window each is given up on.
///
/// **What it does NOT exit for is having no adapter**, which is the state every
/// Rivet built before `ADR-041` is in. It blocks on the management socket and
/// waits for one — `cmd/rivet-bluetoothd/src/run.rs` carries that argument, and
/// it is the reason this is not `restart = never` with a note about hardware.
///
/// # Its own uid, keeping `CAP_NET_ADMIN` and `CAP_SETPCAP` until it drops both
///
/// It runs as `app_identity(BLUETOOTHD_NAME)` under
/// `rivet_core::confinement::image_service_policy`, with the home an
/// application has. Its own uid fits because nothing it hands a consumer is a
/// file it owns: the advertisement stream is the listening socket this
/// supervisor binds, chowns to the consumer and passes as a descriptor, and an
/// `accept` on a handed descriptor needs no ownership of anything.
///
/// Binding `HCI_CHANNEL_USER` checks `CAP_NET_ADMIN`, so it keeps that across
/// the uid change, and `CAP_SETPCAP`, which is what lets it empty its own
/// bounding set. Once the user channel is bound the daemon drops both:
/// permitted, effective, inheritable and ambient emptied, the bounding set
/// emptied, `NoNewPrivs` already set. The list here is the ceiling for the
/// moments before that bind, not what it scans with.
///
/// # No readiness
///
/// `ADR-017` §4.2's `none` rather than `listening`, and here that is a real
/// choice rather than a limitation: the socket is bound by the **supervisor**
/// before the spawn, so `listening` would report ready the instant the process
/// started and say nothing about whether it had found an adapter. `OnStart` is
/// the honest value, and the journal's `SCANNING` line is what a reader wants.
fn bluetoothd() -> ServiceDef {
    let volume = rivet_core::service::BUILT_IN_VOLUME;
    let name = rivet_core::service::ServiceName::new(BLUETOOTHD_NAME);
    // **No arguments, and that is a design statement.** It is not told which
    // adapter to use, because the kernel is the authority for that and it asks
    // (`AGENTS.md` §5); it is not told where its socket is, because it is
    // handed one; and it reads no document, because `ADR-041` §6 argues there
    // is nothing in tier 1 an operator would set that `enabled` does not
    // already say.
    let mut program = homed_program(BLUETOOTHD_PATH, volume, name.as_ref(), &[]);
    // The bind's capability and the one that lets it give that up. The
    // ruleset does not mediate `AF_BLUETOOTH`; the uid and the drop are what
    // bound it.
    program.capabilities = vec![
        crate::kernel::Capability::NET_ADMIN,
        crate::kernel::Capability::SETPCAP,
    ];
    ServiceDef {
        name: BLUETOOTHD_NAME.to_string(),
        program,
        // The home an application has: `provide_image_homes` and
        // `/etc/passwd` key on it.
        volume: Some(volume.to_string()),
        // **The operator's decision, and only theirs.** See above for why the
        // starting value is `false`.
        enabled: false,
        restart: RestartPolicy::OnFailure,
        readiness: Readiness::OnStart,
        consequence: "no application on this machine can read a BLE advertisement, so anything \
                      that reads a Bluetooth sensor has nothing to read. Nothing else is \
                      affected: this service holds the Bluetooth adapter and touches no other \
                      part of the machine (ADR-041)"
            .to_string(),
        // **`provides` folded into `listen`, exactly as an application's is.**
        // `crate::applications` does the same to a manifest's, and doing it
        // here rather than teaching the supervisor a second path is what makes
        // this an ordinary provider rather than a special case.
        listen: vec![rivet_core::package::application::Socket::Unix(
            rivet_core::bluetooth::ENDPOINT.to_string(),
        )],
        // An `Endpoint` is PARSED rather than constructed, because the parser
        // is what an application's manifest goes through and a second way of
        // building one is a second set of rules about what a name may be
        // (`ADR-011`). The argument is a constant, so a refusal here would be a
        // defect in `rivet_core::bluetooth::ENDPOINT` and not input — and
        // `into_iter().collect()` makes the degradation an **empty
        // `provides`**, which is the honest one: the service still starts, and
        // `crate::endpoints` refuses its consumer by name rather than a machine
        // silently having a socket nobody may open.
        // `the_endpoint_this_service_provides_is_the_one_an_application_names`
        // below is what stops that from ever being the shipped state.
        provides: rivet_core::package::application::Endpoint::parse(
            "provides",
            &format!("socket:{}", rivet_core::bluetooth::ENDPOINT),
        )
        .into_iter()
        .collect(),
        ..IMAGE_DEFINED
    }
}

/// **Whether stopping `name` would take away the means of undoing that** —
/// [`REACHABILITY_SET`], and every service [`network_services`] derives.
///
/// A DHCP client or a supplicant is the address the control plane listens on,
/// exactly as `rivet-netd` is; their names are made from the document, so they
/// are recognised by prefix rather than listed.
#[must_use]
pub fn keeps_the_machine_reachable(name: &str) -> bool {
    REACHABILITY_SET.contains(&name) || rivet_core::service::is_network_mechanism_service(name)
}

/// **The network's long-lived mechanisms, derived from the document's
/// `[network]` table** — one DHCP client per interface it addresses by DHCP,
/// one supplicant per interface it gives an `ssid`. `ADR-004` §8, `ADR-017`
/// §3.
///
/// # Derived, not listed and not applications
///
/// What runs is a function of the document the way an image service's
/// `enabled` is, so they are made here, at boot, from the same document every
/// other definition is converged against. They ship in the image, run from
/// `/system`, and are named [`rivet_core::service::dhcp_service_name`] and
/// [`rivet_core::service::supplicant_service_name`]. A `[network]` table
/// changed on a running machine reaches them at the next boot, as it reaches
/// `rivet-netd`.
///
/// # Why they are services and not `rivet-netd`'s children
///
/// `rivet-netd` runs once and exits, and the supervisor then sweeps its cgroup:
/// a client it had started was signalled with it, and its lease never renewed.
/// As services they have cgroups of their own, their output captured for the
/// life of the process, and a restart when they die. `always`, because a
/// client or supplicant that exits has stopped holding the interface's address
/// or association, whatever its status.
///
/// # uid 0, reduced
///
/// They run as `rivet-netd` does: uid 0, bounding set [`NETWORK_CAPABILITIES`],
/// `NoNewPrivs`. The client needs the raw socket and port 68; the hook it
/// execs writes the root-owned lease directory and signals this process. The
/// supplicant drives `nl80211` and reads EAPOL frames. Their own identities are
/// a later step.
#[must_use]
pub fn network_services(document: &rivet_config::Document) -> Vec<ServiceDef> {
    let mut out = Vec::new();
    for interface in document.network() {
        let name = interface.name().as_str();
        if interface.ssid().is_some() {
            out.push(network_mechanism(
                rivet_core::service::supplicant_service_name(name),
                rivet_adapter_net::supplicant::MECHANISM,
                rivet_adapter_net::supplicant::arguments(name),
                "this interface is not associated with its wireless network, so it has no link \
                 and no address, and anything reaching the machine through it cannot (ADR-004 \
                 §4)",
            ));
        }
        if matches!(interface.addressing(), rivet_config::Addressing::Dhcp) {
            out.push(network_mechanism(
                rivet_core::service::dhcp_service_name(name),
                rivet_adapter_net::dhcp::MECHANISM,
                rivet_adapter_net::dhcp::arguments(name),
                "this interface takes no lease and renews none, so it has no address from DHCP \
                 or loses the one it had when the lease runs out, and nothing can reach the \
                 machine through it (ADR-004 §8)",
            ));
        }
    }
    out
}

/// One of [`network_services`]' definitions.
fn network_mechanism(
    name: String,
    path: &str,
    args: Vec<String>,
    consequence: &str,
) -> ServiceDef {
    ServiceDef {
        name,
        program: Program {
            path: path.to_string(),
            args,
            env: Vec::new(),
            identity: None,
            confinement: None,
            groups: Vec::new(),
            capabilities: NETWORK_CAPABILITIES.to_vec(),
        },
        enabled: true,
        restart: RestartPolicy::Always,
        readiness: Readiness::OnStart,
        consequence: consequence.to_string(),
        ..IMAGE_DEFINED
    }
}

/// **The capabilities the network's own programs keep, and the whole of
/// root they hold**: `rivet-netd`, and [`network_services`]. Each stays uid 0 with its bounding set
/// reduced to these and `NoNewPrivs` set (`crate::kernel::Kernel::spawn`).
///
/// * `CAP_NET_ADMIN` — netlink writes of links, addresses and routes, and a
///   supplicant driving `nl80211`.
/// * `CAP_NET_RAW` — the packet socket a DHCP client broadcasts its first
///   DISCOVER on, and a supplicant reads EAPOL frames from.
/// * `CAP_NET_BIND_SERVICE` — a DHCP client's port 68.
pub const NETWORK_CAPABILITIES: [crate::kernel::Capability; 3] = [
    crate::kernel::Capability::NET_ADMIN,
    crate::kernel::Capability::NET_RAW,
    crate::kernel::Capability::NET_BIND_SERVICE,
];

/// `rivet-netd` — **converge the interfaces the document names, then exit.**
///
/// # It runs once, and `on-failure` is what says so
///
/// `cmd/rivet-netd` is not a daemon and its own module documentation is blunt
/// about it: it "converges the interfaces the configuration document names,
/// prints what it did, and exits". A supervisor that restarted it on a clean
/// exit would loop for ever over a machine that is already configured, and
/// `ADR-017` §3.5's hazard is precisely that — a restart loop writes to an SD
/// card until the card dies.
///
/// **The existing three-value vocabulary already expresses this and no fourth
/// value is added.** `ADR-017` §3.5's `on-failure`, with
/// [`crate::supervise::State::Done`], is exactly *runs once, exits 0, stays
/// exited*:
///
/// | It exits | `wants_restart` | State | Next convergence pass |
/// | :-- | :-- | :-- | :-- |
/// | 0 — every interface configured | false | `Done` | nothing; `Done` is not `Idle` |
/// | 1 — an interface refused | true | `Idle` → `Backoff` | started again, with backoff |
/// | 2 — no usable document | true | `Idle` → `Backoff` | started again, with backoff |
/// | 3 — no netlink socket | true | `Idle` → `Backoff` | started again, with backoff |
///
/// `State::Done` exists for this and its own documentation says why it is
/// distinct from `Idle`: convergence has no memory, so an `Idle` enabled
/// service is started on the very next pass.
///
/// # And it is run again when what it applies changes
///
/// A DHCP interface's address arrives after this program has exited, through
/// a lease its supervised client records ([`network_services`]), and a
/// supplicant the supervisor restarts has forgotten its network. The
/// supervisor runs this program again for both
/// (`crate::supervise::Supervisor::follow_leases` and the supplicant's start),
/// which is how every netlink write stays in one program while the
/// mechanisms that feed it run for the life of the machine.
///
/// **`never` would be the wrong reading of "runs once".** It would leave a
/// machine whose interface was not yet present at boot permanently
/// unconfigured, and `ADR-017` §3.2 is explicit that a service which needs
/// something not yet ready **retries** — the obligation the ADR places on
/// service authors, met here by the policy rather than by code inside `netd`.
///
/// **And a repeated failure still ends.** Four backoffs and the fifth attempt
/// marks it `Failed` (`Limits::give_up_after`), which is the honest outcome for
/// a document asking for `method = "dhcp"` on a card built with no `udhcpc` on
/// it at all (`rivet_adapter_net::dhcp::DhcpUnavailable::MechanismNotBuilt`):
/// `netd` exits 1 every time, deterministically. It says so five times over
/// about four seconds and then stops — a broken service rather than a broken
/// machine, which is §3.5's own phrasing.
///
/// # The document, and why the path is an argument
///
/// `ADR-007` §3.3 makes the stored document the statement of what the machine
/// should be, and `ADR-017` §3.1.9 has PID 1 seed it from the boot partition on
/// a first boot. `cmd/rivet-netd` takes `--document <path>`: a path says
/// *where to read the statement from*, not what the statement is, which is why
/// it is an argument and an address never could be (`ADR-011` §3).
///
/// The value is [`ConfigStore::machine_path`] rather than a literal, so the
/// supervisor cannot point `netd` at a file the store does not write.
fn netd() -> ServiceDef {
    ServiceDef {
        name: NETD_NAME.to_string(),
        program: Program {
            path: NETD_PATH.to_string(),
            args: vec![
                "--document".to_string(),
                ConfigStore::at_default().machine_path(),
            ],
            env: Vec::new(),
            // **uid 0, holding only the network's three capabilities.** It
            // reads root-owned files — the document, a wireless key — and
            // writes netlink, so it keeps the uid and loses the rest of root:
            // `NoNewPrivs`, and a bounding set of [`NETWORK_CAPABILITIES`].
            identity: None,
            confinement: None,
            groups: Vec::new(),
            capabilities: NETWORK_CAPABILITIES.to_vec(),
        },
        enabled: true,
        restart: RestartPolicy::OnFailure,
        // `ADR-017` §4.2's `none`: this program has no readiness signal and
        // will not grow one — it exits. Saying `none` is honest rather than
        // lax; it records that a spawn is not a readiness.
        readiness: Readiness::OnStart,
        consequence: "this machine has no address, route or resolver from Rivet, so nothing \
                      can reach it over the network (ADR-004 §2)"
            .to_string(),
        ..IMAGE_DEFINED
    }
}

/// `rivet-timed` — **the wall clock, corrected against something outside this
/// machine.** `ADR-021` §7.
///
/// > *"It is a service under `ADR-017` §2 — supervised, in the desired-state
/// > document, configurable (servers, poll interval, enabled). It is **not**
/// > part of `rivet-netd`. It needs the network, but under `ADR-017` §3.2's
/// > convergence model that is not a dependency to declare: it retries until the
/// > network works, like everything else."*
///
/// # It stays up, and `always` is what says so
///
/// The same shape as [`mcpd`] and the opposite of [`netd`]. A clean exit
/// from a time client is not a completion — it is a machine that has stopped
/// correcting its clock, silently, for as long as it stays up. `ADR-017` §3.5's
/// give-up threshold still applies, which is the right answer for a binary that
/// exits 2 on an argument list this file got wrong: it says so five times and
/// stops, rather than writing to the card for ever.
///
/// **`on-failure` would be the wrong reading.** `rivet-timed --once` exits 0
/// having taken one sample, and this definition does not pass `--once`; the
/// service form never returns at all. So a clean exit here means something
/// unexpected happened, and restarting is the honest response.
///
/// # There is no ordering against `rivet-netd`, and §3.2.1 is why
///
/// This is the **fourth** apparent ordering `ADR-017` §3.2.1 has been asked to
/// settle, and it is the same kind as its second: *the network must be up before
/// a time server can be reached*. It is a **state of the world**, not a
/// happens-before. `rivet-timed` polls every sixty-four seconds until
/// something answers and every seventeen minutes afterwards, so the network
/// arriving late costs at most a minute — and a declared `After=network` would
/// be wrong the moment the link went down and came back.
///
/// # Why it is enabled, and the rule that decides
///
/// **A definition is enabled exactly when the image installs its binary.**
/// Enabled without the binary is a permanent spawn refusal on every boot of
/// every board — the supervisor spawns a path that is not on the card, gets
/// `ENOENT`, and refuses loudly and permanently via
/// `Errno::is_a_program_this_image_cannot_run`, about a service the operator
/// never asked for. Installed without enabled is a binary nothing can start
/// without a document edit. `cmd/rivet-image` installs `rivet-timed` at
/// [`TIMED_PATH`], so this is `true`, and
/// `timed_is_enabled_exactly_when_the_image_installs_it` reads the other
/// crate's source and asserts both directions rather than the value.
///
/// **What that means for a Rivet board, said plainly rather than left to be
/// discovered:** every board runs an SNTP client that sends UDP to
/// [`TIME_SERVERS`] and calls `clock_settime`. `ADR-021` §4 is what wants that —
/// a machine whose clock is never checked against anything outside itself has
/// timestamps as wrong as its last boot plus however long it was off — and
/// `ADR-017` §6 makes turning it off the document's job and one key:
///
/// ```toml
/// [services.rivet-timed]
/// enabled = false
/// ```
///
/// # Its own uid and a ruleset, keeping `CAP_SYS_TIME` and nothing else
///
/// It parses replies from servers on the internet, so it runs as
/// `app_identity(TIMED_NAME)` under
/// `rivet_core::confinement::image_service_policy`, with the home an
/// application has, on [`mdnsd`]'s terms. What it touches:
///
/// * **One UDP socket per exchange, bound to an ephemeral port** (`rsntp`'s
///   default `0.0.0.0:0`), so no port below 1024 and no
///   `CAP_NET_BIND_SERVICE`. Landlock does not mediate UDP.
/// * **No name resolution**: `--server` takes addresses and its parser
///   refuses a hostname, so nothing reads `/etc/resolv.conf`.
/// * **`/proc/sys/kernel/random/boot_id`**, which the platform rows grant to
///   every confined program.
/// * **Nothing on `/state`.** The stored clock floor is written by this
///   supervisor alone (`crate::clock`), and its records go to stdout, which
///   the supervisor captures.
/// * **`clock_settime(CLOCK_REALTIME)`**, which needs `CAP_SYS_TIME`. That is
///   the one capability it keeps — permitted, effective and ambient, with the
///   bounding set reduced to it ([`Program::capabilities`]).
///
/// # The contract this definition must satisfy
///
/// 1. The binary is at [`TIMED_PATH`], executable, and statically linked against
///    musl for `aarch64` (`ADR-003` §4). `cmd/rivet-image` installs it at that
///    path, `0755`, root-owned, in both `/system` slots, from a release build
///    for `aarch64-unknown-linux-musl`. Asserted by that crate's own test and
///    by the tripwire below.
/// 2. It accepts `--server <address>`, repeated. Read from
///    `cmd/rivet-timed/src/args.rs`, which is in this workspace, so this row
///    is checkable rather than assumed; the tripwire below reads it.
/// 3. **The process holds `CAP_SYS_TIME`**, kept across the uid change by
///    [`crate::kernel::Kernel::spawn`]. Without it every step is an `EPERM`
///    and `rivet-timed` says so at `error` on every correction it tries.
/// 4. At least two addresses in [`TIME_SERVERS`] are reachable from the
///    machine and answer SNTP, because a step needs two servers to agree.
fn timed() -> ServiceDef {
    let volume = rivet_core::service::BUILT_IN_VOLUME;
    let name = rivet_core::service::ServiceName::new(TIMED_NAME);
    let mut program = homed_program(TIMED_PATH, volume, name.as_ref(), &[]);
    let mut args = Vec::with_capacity(TIME_SERVERS.len() * 2);
    for server in TIME_SERVERS {
        args.push("--server".to_string());
        args.push((*server).to_string());
    }
    program.args = args;
    // The one piece of root it keeps: setting the wall clock.
    program.capabilities = vec![crate::kernel::Capability::SYS_TIME];
    ServiceDef {
        name: TIMED_NAME.to_string(),
        program,
        // The home an application has: `provide_image_homes` and
        // `/etc/passwd` key on it.
        volume: Some(volume.to_string()),
        // **Not a judgement about clocks**, which `ADR-021` §4 already made.
        // It is the image's fact — the binary is installed — and one document
        // key turns it off (`ADR-017` §6).
        enabled: true,
        restart: RestartPolicy::Always,
        // `ADR-017` §4.2's `none`. `rivet-timed` *could* send `READY=1` — it is
        // Rivet's own Rust binary — and it deliberately does not, because there
        // is no answer to what "ready" would mean for it. Ready when the process
        // is up? That is `none` with extra steps. Ready when the clock is right?
        // That can be seventeen minutes after start on a board whose network is
        // late, and `ADR-017` §3.3's readiness is not a health check.
        readiness: Readiness::OnStart,
        consequence: "this machine's wall clock is never checked against anything outside itself, \
                      so every timestamp stays as wrong as the last boot's plus however long the \
                      board was off, and certificate expiry cannot be trusted (ADR-021 §4)"
            .to_string(),
        ..IMAGE_DEFINED
    }
}

/// `rivet-deploy` — **the installer, defined but not in [`system_services`],
/// and the gap is a ruling rather than an omission.**
///
/// The maintainer's 2026-08-24 ruling made the installer its own supervised binary and
/// the maintainer's 2026-08-24 install-request ruling made it **one-shot**: started per install, exits when done.
/// This is that definition. `cmd/rivet-image` puts the binary on the card, so
/// the half of `ADR-029` §14.4 that was about packaging is closed.
///
/// # Why it takes a parameter, and why that keeps it out of `system_services`
///
/// An install is *of something*. `ADR-029` §9.2 step 1 begins with a payload,
/// `cmd/rivet-deploy` takes its path on `argv`, and there is no default and
/// no directory it searches — `ADR-011` §3's distinction, the same one that
/// makes `rivet-netd --document <path>` an argument rather than an address.
///
/// **So this function needs an argument that [`system_services`] has nowhere to
/// get.** That list is a compile-time constant read once, before the
/// `Supervisor` is built, and `ADR-029` §14.4 named the consequence in the same
/// paragraph as the ruling that created the binary:
///
/// > A system service's argv is `cmd/rivet-sysd`'s
/// > `definitions::system_services()`, a compile-time constant carrying no
/// > parameter, so **there is no way today for an operator to start this binary
/// > with a path through the supervisor at all**.
///
/// **The channel is ruled and this function has a caller.** The maintainer's
/// 2026-08-24 ruling took option A — a request file on `/state` — and [`crate::deploy_request`]
/// is the reading half. What made that possible is `ADR-017` §3.2.2's service
/// instance created at run time, which is what this function is built into.
///
/// **There is still no CLI-to-supervisor socket**, and option A exists so that
/// none is needed. That was checked rather than assumed, and it is the half of
/// §14.4's sentence that has not moved.
///
/// The argument for the parameter, above, is unchanged and is now load-bearing
/// rather than theoretical: the payload comes off the request file, one pass
/// before the convergence that spawns this.
///
/// **This is not a placeholder and it is not scaffolding** (`AGENTS.md` §3).
/// Every field is the answer, argued below, and the definition validates.
///
/// # `never`, and it is the one policy choice here
///
/// `ADR-017` §3.5's three values, and this is the opposite end from [`netd`],
/// which is the other run-once service. The maintainer's ruling is the argument:
///
/// > **one-shot** — nothing needs it resident, and **a crashed one-shot is a
/// > failed install rather than a broken machine**.
///
/// `on-failure` — [`netd`]'s answer — would retry a failed install four times
/// with backoff, and every one of the three failure exits is the wrong thing to
/// retry:
///
/// | Exit | Means | Retrying it |
/// | :-- | :-- | :-- |
/// | 1 | the payload is refused | re-reads and re-verifies the same bytes to reach the same verdict. `ADR-017` §3.5.1's argument exactly: the failure is a fact about the file, not about the moment |
/// | 2 | `argv` was not usable | identical every time |
/// | 3 | the machine, the card or the kernel refused | **the only one that might work later** — and it may have written part of a 640 MiB slot, so an automatic second attempt starts writing again over a partial one, unasked |
///
/// **And the asymmetry with `netd` is the whole reason they differ.** `netd`
/// converges *the machine's own desired state* — a thing the machine should be
/// doing anyway, so retrying is the supervisor doing its job. An install is an
/// **operator-initiated act on a payload the operator named**, and repeating
/// somebody's request without being asked is not convergence.
///
/// The cost is stated rather than hidden: a card that was momentarily busy is
/// not retried, and the operator has to ask again. `rivet get install` is how
/// they find out — the outcome token says which of the three it was.
///
/// # The supervisor does not check the payload first
///
/// It reads a path it was given, mounts what it needs itself (`ADR-029` §14.2
/// puts that in the binary's own seam), and writes to a block device. **A
/// `stat` here would be a second reader of the thing the installer is about to
/// read**, and the installer's refusal can say what is wrong with the file
/// while a `stat` can only say it is not there. `ADR-029` §11's argument
/// against two verifiers, one notch smaller.
///
/// # Nothing here has run **on a board**
///
/// `AGENTS.md` §6. [`crate::deploy_request`] constructs this definition, hands
/// it to a [`crate::supervise::Supervisor`], and the supervisor spawns it —
/// asserted end to end over a `crate::testkernel`, which is a fake kernel
/// rather than a machine. **No install has been started this way on a Raspberry
/// Pi**, and nothing but a board can settle whether it works.
///
/// What is asserted below is unchanged: that it validates, that it names the
/// path the image installs, and that its flags are the ones the binary accepts
/// — three things a test can settle and none of which is *"an install ran"*.
pub fn deploy(payload: &str) -> ServiceDef {
    ServiceDef {
        name: deploy::SERVICE_NAME.to_string(),
        program: Program {
            path: deploy::PROGRAM_PATH.to_string(),
            args: vec![payload.to_string()],
            env: Vec::new(),
            // uid 0: it mounts a FAT partition and writes a block device. See
            // `IMAGE_DEFINED`.
            identity: None,
            confinement: None,
            groups: Vec::new(),
            capabilities: Vec::new(),
        },
        // **True, and it is not a policy about installing.** A definition built
        // by a caller that was asked for an install is a definition of a thing
        // that should run; `enabled: false` here would mean the supervisor
        // constructed it and then declined to do it. `ADR-017` §6's control
        // surface is untouched, because this definition never reaches the
        // document's union — it does not exist until somebody asks for an
        // install.
        enabled: true,
        restart: RestartPolicy::Never,
        // `ADR-017` §4.2's `none`. It exits; a spawn is not a readiness and
        // saying so is honest rather than lax. What "ready" would even mean for
        // an installer is *finished*, which is `rivet get install`'s question
        // and not the supervisor's.
        readiness: Readiness::OnStart,
        consequence: "the payload an operator asked to install is not installed, so this machine \
                      keeps running exactly what it was running — the inactive slot is never the \
                      running one (ADR-018 §4), so nothing is lost except the install"
            .to_string(),
        ..IMAGE_DEFINED
    }
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    //
    // **The block every other test module in this crate carries, and the one
    // this one never got.** `cmd/rivet-sysd/Cargo.toml` denies `expect_used`
    // rather than warning on it — deliberately, because this crate is the
    // supervisor and an `expect` in it is a machine that stops supervising —
    // and a `deny` reaches test code too. `crate::etc`'s and
    // `crate::supervise`'s test modules lift it in exactly these words.
    //
    // Its absence here is why `cargo clippy --workspace --all-targets` exited
    // **101** from the day this crate tightened the lint until 2026-09-02.
    // `SPEC-04` §12.2 recorded the exit code; what it could not record is that
    // clippy was ALSO absent from the build container, so the three errors were
    // invisible to the loop that would have caught them.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// A test whose *name* asserts a count is a claim that goes stale exactly as
    /// `AGENTS.md` §7 describes, so this one names the record instead and the
    /// list below carries the count.
    #[test]
    fn the_image_defines_the_services_the_record_names_and_no_others() {
        let names: Vec<_> = system_services().iter().map(|s| s.name.clone()).collect();
        assert_eq!(
            names,
            [
                NETD_NAME,
                TIMED_NAME,
                MDNSD_NAME,
                MCPD_NAME,
                // Added 2026-09-05, `ADR-041`. The first that provides an
                // endpoint to an application.
                BLUETOOTHD_NAME,
                // Added 2026-09-20, `ADR-058` §4. Ahead of the launcher on
                // purpose: it is the launcher's compositor, and this set is
                // started in order.
                DISPLAYD_NAME,
                // Added 2026-09-07, `ADR-050` §2. The first that CONNECTS to
                // one — and the first that was an application before it was a
                // service.
                LAUNCHER_NAME,
                // `ADR-060`. The first image service that runs under its own
                // identity with a home under /state/app.
                PROXYD_NAME,
                // `ADR-047`. The name service, shipped disabled.
                DNSD_NAME,
            ]
        );
    }

    /// **`ADR-047`'s service answers on 53 over both transports, and no machine
    /// runs it that did not ask.** A starting value of `true` would turn on a
    /// resolver on every machine an update reaches.
    #[test]
    fn the_name_service_is_disabled_and_listens_on_53_over_udp_and_tcp() {
        let d = dnsd();
        assert!(!d.enabled, "no machine may run the name service without asking");
        assert_eq!(
            d.listen,
            [
                rivet_core::package::application::Socket::Udp(53),
                rivet_core::package::application::Socket::Tcp(53),
            ]
        );
        assert!(!d.proxied, "a resolver is asked directly, on every address");
        assert_eq!(d.program.path, "/system/bin/rivet-dnsd");
        assert_eq!(
            d.program.args,
            ["--document".to_string(), "/state/app/rivet-dnsd/dns.toml".to_string()],
            "it reads its projection, never the machine document"
        );
    }

    /// **The tripwire between the two lists of the image's services.**
    ///
    /// `rivet_core::service::IMAGE_SERVICE_NAMES` is what an application's name
    /// is refused against and what `Document::carried_services` excludes;
    /// [`system_services`] is what this supervisor actually defines. They are
    /// two lists only because this crate has no library target, and one that
    /// lagged the other is how `rivet-displayd` was once treated as an
    /// application a card had to carry. Membership, not order.
    #[test]
    fn the_image_service_names_are_exactly_the_services_the_image_defines() {
        let mut defined: Vec<String> = system_services().into_iter().map(|d| d.name).collect();
        let mut named: Vec<String> = rivet_core::service::IMAGE_SERVICE_NAMES
            .iter()
            .map(|n| (*n).to_string())
            .collect();
        defined.sort();
        named.sort();
        assert_eq!(
            named, defined,
            "rivet_core::service::IMAGE_SERVICE_NAMES and definitions::system_services disagree"
        );
    }

    /// **Nothing pinned this set until 2026-08-29**, and it went on naming a
    /// program that was no longer the administration surface for three days
    /// after `cargo rivet run` made the justification in its own documentation
    /// false. A constant that decides what a reload may not stop is a policy,
    /// and an unpinned policy drifts silently — `AGENTS.md` §7.
    ///
    /// Membership, not order, and every name must be a service the image
    /// actually defines: a typo here withholds nothing and reads as working.
    #[test]
    fn the_control_plane_is_chosen_after_everything_but_the_supervisor_and_nothing_else_is() {
        assert_eq!(SUPERVISOR_OOM_SCORE_ADJ, -1000);
        assert_eq!(oom_score_adj(MCPD_NAME), -900);
        for service in system_services() {
            if service.name != MCPD_NAME {
                assert_eq!(oom_score_adj(&service.name), 0, "{}", service.name);
            }
        }
        assert_eq!(oom_score_adj("photo-index"), 0);
    }

    #[test]
    fn the_reachability_set_is_the_control_plane_and_the_interface() {
        assert!(
            REACHABILITY_SET.contains(&MCPD_NAME),
            "the control plane is the administration surface (ADR-035 §3)"
        );
        assert!(
            REACHABILITY_SET.contains(&NETD_NAME),
            "without an address there is nothing for it to listen on"
        );
        assert_eq!(
            REACHABILITY_SET.len(),
            2,
            "a third member withholds something from the operator, and withholding is how \
             this set works"
        );

        let defined: Vec<_> = system_services().into_iter().map(|d| d.name).collect();
        for name in REACHABILITY_SET {
            assert!(
                defined.iter().any(|d| d == name),
                "{name} is withheld from reloads but no definition ships it"
            );
        }
    }

    /// Every one of these would otherwise fail at the first spawn, on a board,
    /// with a message about a cgroup or a working directory rather than about
    /// the definition.
    #[test]
    fn every_compiled_in_definition_validates() {
        for def in system_services() {
            assert_eq!(def.validate(), Ok(()), "{}", def.name);
        }
    }

    /// The whole of the run-once decision, pinned so that a later change to
    /// either policy has to be deliberate.
    ///
    /// **The name is the point.** This test used to be called
    /// `..._starts_disabled`, which asserted a roadmap position rather than a
    /// property — so the day the roadmap moved past it, the test went on
    /// passing and defended the stale value. `ADR-035` scheduled the flip
    /// for phase 2; phases 2 and 3 completed without it. **A test named after a
    /// phase outlives the phase.**
    #[test]
    fn the_control_plane_is_defined_and_starts_enabled() {
        let mcpd = system_services()
            .into_iter()
            .find(|d| d.name == MCPD_NAME)
            .expect("the supervisor defines rivet-mcpd");
        assert_eq!(mcpd.program.path, MCPD_PATH);
        assert!(
            mcpd.enabled,
            "since ADR-035 phase 3 this IS the way in, so a card that booted without it \
             would be unreachable (ADR-017 §6.2's bootstrap test, answered the other way \
             on 2026-08-29)"
        );
        assert_eq!(
            mcpd.restart,
            RestartPolicy::Always,
            "a server that exits cleanly has not finished, it has stopped serving"
        );
    }

    /// **It is told where to READ its address, not what the address is.**
    /// `ADR-017` §4.1: only `enabled` and a memory cap are the operator's, so
    /// a supervisor that rewrote this argument vector from the document would
    /// be letting configuration decide what a program is. The program reads the
    /// document itself — `netd` and `mdnsd` do the same.
    #[test]
    fn the_control_plane_is_told_where_to_read_its_address() {
        let mcpd = system_services()
            .into_iter()
            .find(|d| d.name == MCPD_NAME)
            .expect("the supervisor defines rivet-mcpd");
        assert!(
            mcpd.program.args.iter().any(|a| a == "--document"),
            "the definition must hand it the document: {:?}",
            mcpd.program.args
        );
        assert!(
            !mcpd.program.args.iter().any(|a| a.contains(':')),
            "no address may appear in the argument vector — it lives in the document now: {:?}",
            mcpd.program.args
        );
    }

    /// **It runs as root, deliberately.** The temptation is to give a network
    /// listener an unprivileged identity; this one execs the CLI that installs
    /// payloads and reboots the machine, so dropping privilege would produce a
    /// control plane that cannot control.
    #[test]
    fn the_control_plane_keeps_uid_zero_because_it_is_the_control_plane() {
        let mcpd = system_services()
            .into_iter()
            .find(|d| d.name == MCPD_NAME)
            .expect("the supervisor defines rivet-mcpd");
        assert!(mcpd.program.identity.is_none(), "uid 0 — see the definition");
    }

    /// The two shapes this file defines, asserted against each other so that a
    /// later edit has to choose one deliberately.
    #[test]
    fn netd_runs_once_and_the_control_plane_stays_up() {
        use crate::kernel::ExitStatus;

        let netd = netd();
        assert_eq!(netd.restart, RestartPolicy::OnFailure);
        assert!(
            !netd.restart.wants_restart(ExitStatus::Exited(0)),
            "a converge that succeeded is not a crash"
        );
        assert!(
            netd.restart.wants_restart(ExitStatus::Exited(1)),
            "ADR-017 §3.2 — a service whose requirements are not ready yet retries"
        );

        let mcpd = mcpd();
        assert_eq!(mcpd.restart, RestartPolicy::Always);
        assert!(
            mcpd.restart.wants_restart(ExitStatus::Exited(0)),
            "a server that exits cleanly has not finished; it has stopped serving"
        );
    }

    /// `ADR-011`: the stored document has one path, and it is the store's.
    /// A literal here would let the supervisor point `netd` at a file nothing
    /// writes, and the symptom would be `netd` exiting 2 on a machine that was
    /// provisioned perfectly.
    #[test]
    fn netd_is_pointed_at_the_path_the_store_actually_writes() {
        let netd = netd();
        let expected = ConfigStore::at_default().machine_path();
        assert_eq!(netd.program.args, ["--document", expected.as_str()]);
        assert_eq!(expected, "/state/config/machine.toml");
    }

    /// Every path any definition hands to a program is absolute, because
    /// `ServiceDef::validate` only checks the program itself.
    ///
    /// **Written against the argument vectors rather than against a list of
    /// this module's constants**, which is what the earlier form did: a test
    /// that enumerates constants stops covering an argument the moment the
    /// argument stops being a constant here. Reading the vectors keeps it true
    /// through that move and the next one.
    #[test]
    fn every_path_handed_to_a_program_is_absolute() {
        for def in system_services() {
            assert!(def.program.path.starts_with('/'), "{}", def.program.path);
            for arg in &def.program.args {
                if arg.contains('/') {
                    assert!(arg.starts_with('/'), "{arg} is a relative path");
                }
            }
        }
    }

    /// A service with nothing to lose does not need saying; one that is down
    /// and says nothing about what that costs is the failure `ADR-017` §3.1.6's
    /// `consequence_of` mechanism was introduced to prevent.
    #[test]
    fn every_definition_says_what_the_machine_loses_without_it() {
        for def in system_services() {
            assert!(
                def.consequence.len() > 20,
                "{} does not say what its absence costs",
                def.name
            );
            assert!(
                def.consequence.contains("ADR-"),
                "{} does not cite the ruling its consequence comes from",
                def.name
            );
        }
    }

    // -- rivet-timed, ADR-021 §7 -------------------------------------------

    /// `ADR-021` §7 makes it a service; `ADR-017` §3.5's vocabulary is what says
    /// it is the kind that stays up. Pinned so a later change is deliberate.
    #[test]
    fn timed_stays_up_because_a_time_client_that_exits_stops_correcting_the_clock() {
        use crate::kernel::ExitStatus;

        let t = timed();
        assert_eq!(t.restart, RestartPolicy::Always);
        assert!(
            t.restart.wants_restart(ExitStatus::Exited(0)),
            "the service form never returns; a clean exit is something unexpected"
        );
        assert_eq!(t.readiness, Readiness::OnStart);
    }

    /// **The pairing, not the value.** A definition that is enabled while the
    /// image installs nothing is the `ENOENT`-on-every-boot failure; a
    /// definition that is disabled while the image *does* install it is a
    /// binary nobody can run without editing a document.
    ///
    /// Reading the image's own source is what lets one test hold both
    /// directions — the same device the tripwire below uses, and the same
    /// caveat: it proves the source says the right thing, which is weaker than
    /// proving the card carries it.
    #[test]
    fn timed_is_enabled_exactly_when_the_image_installs_it() {
        let system_rs =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../rivet-image/src/system.rs");
        let Ok(body) = std::fs::read_to_string(&system_rs) else {
            eprintln!("cmd/rivet-image/src/system.rs was not readable; this test is blind");
            return;
        };
        let installed = body.contains("pub const TIMED_PATH: &str = \"");
        assert_eq!(
            timed().enabled,
            installed,
            "rivet-timed is enabled={} while cmd/rivet-image installs={}. Enabled without \
             the binary is a permanent spawn refusal on every boot of every board; installed \
             without enabled is a binary nothing can start without a document edit.",
            timed().enabled,
            installed
        );

        // And the whole enabled set, so this stays a statement about which
        // services a fresh card starts rather than about one flag.
        //
        // **This line is the only place a card's behaviour is visible.** A
        // *board* runs the document and only a *card* runs these defaults, so a
        // starting value can be wrong for days while every board in service
        // behaves correctly — which is exactly how `rivet-mcpd` came to be
        // three phases late (`PLAN.md` §94.7, and [`mcpd`]'s own note).
        let enabled: Vec<_> = system_services()
            .into_iter()
            .filter(|s| s.enabled)
            .map(|s| s.name)
            .collect();
        assert_eq!(enabled, [NETD_NAME, TIMED_NAME, MCPD_NAME]);
    }

    /// Every `--server` word is an **address**, never a name.
    ///
    /// `cmd/rivet-timed`'s parser refuses a hostname, so a name here would be
    /// a service that exits 2 five times and is marked failed — but the failure
    /// would name the argument rather than the reason, which is
    /// `/etc/resolv.conf` being absent from the image (`ADR-008` §5.1.1). This
    /// asserts the same rule on this side of the boundary, where the values
    /// actually live.
    #[test]
    fn every_time_server_is_an_address_and_none_is_a_hostname() {
        assert!(!TIME_SERVERS.is_empty(), "a time client with no servers");
        for server in TIME_SERVERS {
            assert!(
                server.parse::<std::net::IpAddr>().is_ok(),
                "{server} is not an IP address; musl with no /etc/resolv.conf resolves every name \
                 against 127.0.0.1, where nothing on a Rivet board listens (ADR-008 §5.1.1)"
            );
        }
        // The definition hands them over as `--server <addr>` pairs, in order.
        let expected: Vec<String> = TIME_SERVERS
            .iter()
            .flat_map(|s| ["--server".to_string(), (*s).to_string()])
            .collect();
        assert_eq!(timed().program.args, expected);
    }

    /// More than one provider, so one organisation's outage is not this
    /// machine's. Written against the first two octets rather than the whole
    /// address, because that is the property that matters: two addresses in one
    /// prefix fail together.
    #[test]
    fn the_time_servers_are_not_all_in_one_prefix() {
        let mut prefixes: Vec<String> = TIME_SERVERS
            .iter()
            .map(|s| s.split('.').take(2).collect::<Vec<_>>().join("."))
            .collect();
        prefixes.sort_unstable();
        prefixes.dedup();
        assert!(
            prefixes.len() >= 2,
            "every time server is in {prefixes:?}; one route failure would be a machine with no \
             time source at all"
        );
    }

    /// The tripwire for the gap this whole definition is waiting on.
    ///
    /// Guarded rather than unconditional, and the answer is **printed either
    /// way**, which is the same shape `cmd/rivet-sysd/src/kernel.rs`
    /// uses for its cgroup probe: a skipped assertion must be visible rather
    /// than silent (`AGENTS.md` §7). Today the image declares no `TIMED_PATH`
    /// and this prints so; the day it does, the two must agree or the image
    /// installs the binary at one path while the supervisor starts it from
    /// another.
    #[test]
    fn the_timed_path_matches_the_image_if_the_image_installs_it() {
        let system_rs =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../rivet-image/src/system.rs");
        let Ok(body) = std::fs::read_to_string(&system_rs) else {
            eprintln!("cmd/rivet-image/src/system.rs was not readable; this tripwire is blind");
            return;
        };
        let marker = "pub const TIMED_PATH: &str = \"";
        let Some(rest) = body.split(marker).nth(1) else {
            eprintln!(
                "cmd/rivet-image declares no TIMED_PATH: rivet-timed is NOT installed in any \
                 Rivet image, which is why `timed()` ships disabled"
            );
            return;
        };
        let installed = rest.split('"').next().unwrap_or("");
        assert_eq!(
            installed, TIMED_PATH,
            "cmd/rivet-image installs rivet-timed at {installed}, and this supervisor starts \
             it from {TIMED_PATH}"
        );
    }

    /// The flag this definition passes is the flag that binary takes. Read out
    /// of its source, because a binary crate has no library target to import
    /// from — the same device the `NETD_PATH` tripwire below uses.
    ///
    /// The failure it prevents is specific: `rivet-timed` refuses an unknown
    /// flag rather than ignoring it (`ADR-021` §4.2.1 — there is no `--slew`,
    /// and silently accepting one would let a machine be configured as though
    /// there were), so a renamed flag here is a service that exits 2 five times
    /// and is marked failed.
    #[test]
    fn the_server_flag_is_one_rivet_timed_actually_accepts() {
        let args_rs =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../rivet-timed/src/args.rs");
        let Ok(body) = std::fs::read_to_string(&args_rs) else {
            eprintln!("cmd/rivet-timed/src/args.rs was not readable; this tripwire is blind");
            return;
        };
        for flag in timed().program.args.iter().filter(|a| a.starts_with("--")) {
            assert!(
                body.contains(&format!("\"{flag}\"")),
                "cmd/rivet-timed's parser has no {flag}; it refuses unknown flags, so this \
                 service would exit 2 on every attempt"
            );
        }
    }

    // -- rivet-deploy, the maintainer's 2026-08-24 install-request ruling ---------------------------------------

    /// **It is deliberately not in the list**, and a test says so rather than a
    /// reader having to notice. A `deploy` that appeared in [`system_services`]
    /// would be started on every boot with no payload, exit 2, and — under any
    /// policy but `never` — say so five times.
    #[test]
    fn the_installer_is_defined_and_is_not_one_of_the_services_a_boot_starts() {
        let names: Vec<_> = system_services().iter().map(|s| s.name.clone()).collect();
        assert!(
            !names.contains(&deploy::SERVICE_NAME.to_string()),
            "the installer must not be started by a boot: {names:?}"
        );
        assert_eq!(super::deploy("/state/x.rvt").name, deploy::SERVICE_NAME);
    }

    /// Every reason the supervisor refuses a definition, checked against this
    /// one — because nothing constructs it yet, so the first time it is built
    /// for real is on a board.
    #[test]
    fn the_installer_definition_validates_for_any_payload_path() {
        for payload in [
            "/state/photo-index.rvt",
            "/state/rivet-1.2.3-aarch64-rpi4.update.rvt",
            "/system/x.rvt",
        ] {
            let def = super::deploy(payload);
            assert_eq!(def.validate(), Ok(()), "{payload}");
            assert_eq!(def.program.args, vec![payload.to_string()]);
        }
    }

    /// **`never`, and the asymmetry with `netd` is the point.** Both run once;
    /// only one of them is a convergence. See [`super::deploy`] for the
    /// argument, and note what this asserts: a REFUSED payload is not retried.
    #[test]
    fn a_failed_install_is_reported_rather_than_repeated() {
        use crate::kernel::ExitStatus;

        let def = super::deploy("/state/x.rvt");
        assert_eq!(def.restart, RestartPolicy::Never);
        for status in [
            ExitStatus::Exited(0), // installed
            ExitStatus::Exited(1), // the payload is refused
            ExitStatus::Exited(2), // argv was not usable
            ExitStatus::Exited(3), // the machine refused
        ] {
            assert!(
                !def.restart.wants_restart(status),
                "{status:?}: an install the operator asked for once must not be repeated \
                 unasked — it re-reads the payload and may write a slot a second time"
            );
        }
        // And it is the opposite of the other run-once service, which converges
        // the machine's own desired state and therefore should retry.
        assert_eq!(netd().restart, RestartPolicy::OnFailure);
    }

    /// **The path is `rivet-core`'s, so there is nothing for a tripwire to
    /// catch.**
    ///
    /// Every other `/system/bin` path in this file is a duplicate of one in
    /// `cmd/rivet-image`, guarded by a test that greps the other crate's
    /// source — a standing debt, tracked since 2026-08-23. Those greps exist because two crates
    /// have written two spellings of one path before, and each was internally
    /// consistent, so the disagreement was invisible. This one is not a
    /// duplicate:
    /// both crates read `rivet_core::deploy::PROGRAM_PATH`, so they cannot
    /// disagree and the grep would be checking a fact the type system already
    /// holds. Asserted anyway, because a *later* edit that reintroduced a
    /// literal here is exactly the failure the pattern keeps having.
    #[test]
    fn the_installers_path_is_rivet_cores_and_not_a_second_spelling() {
        assert_eq!(super::deploy("/x").program.path, deploy::PROGRAM_PATH);
        let this_file = include_str!("definitions.rs");
        assert!(
            !this_file.contains("\"/system/bin/rivet-deploy\""),
            "a literal path here would be the second spelling the path-duplication debt is about"
        );
    }

    /// The payload reaches the binary as the binary's parser expects it: a bare
    /// positional, not behind a flag it does not have. Read out of that crate's
    /// source, because a binary crate has no library target to import from —
    /// the same device the `NETD_PATH` tripwire below uses.
    #[test]
    fn the_payload_is_passed_the_way_the_installer_reads_it() {
        let def = super::deploy("/state/x.rvt");
        assert_eq!(
            def.program.args,
            vec!["/state/x.rvt".to_string()],
            "the installer takes the payload as a bare positional"
        );
        let args_rs =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../rivet-deploy/src/args.rs");
        let Ok(body) = std::fs::read_to_string(&args_rs) else {
            eprintln!("cmd/rivet-deploy/src/args.rs was not readable; this tripwire is blind");
            return;
        };
        for arg in &def.program.args {
            if arg.starts_with('-') {
                assert!(
                    body.contains(&format!("\"{arg}\"")),
                    "cmd/rivet-deploy's parser has no {arg}; it refuses an unknown option, so \
                     this service would exit 2 on every attempt"
                );
            }
        }
        // And the parser really does take a bare word as the payload, so a
        // positional is not a guess about its shape.
        assert!(
            body.contains("no payload named"),
            "cmd/rivet-deploy no longer refuses a missing positional payload; this \
             definition's shape is an assumption again"
        );
    }

    /// A service that is down and says nothing about what that costs is the
    /// failure `ADR-017` §3.1.6's mechanism exists to prevent — asserted for
    /// this definition too, since it is not in the list the other test walks.
    #[test]
    fn the_installer_says_what_a_machine_loses_when_it_fails() {
        let def = super::deploy("/state/x.rvt");
        assert!(def.consequence.len() > 20, "{}", def.consequence);
        assert!(def.consequence.contains("ADR-"), "{}", def.consequence);
        // The reassuring half has to be there: a failed install does not break
        // the machine, because the inactive slot is never the running one.
        assert!(
            def.consequence.contains("running"),
            "{}",
            def.consequence
        );
    }

    /// The tripwire for the standing path-duplication debt, and the same device
    /// `cmd/rivet-image`'s `boot.rs` already uses against `cmd/rivet-initd`:
    /// read the other crate's constant out of its source, because a binary
    /// crate has no library target to `use` it from.
    ///
    /// If it drifts, the image installs `rivet-netd` at one path and the
    /// supervisor starts it from another — a failure that names neither
    /// constant, on a board, over a link that was never brought up.
    #[test]
    fn the_netd_path_matches_the_one_the_image_installs() {
        let system_rs =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../rivet-image/src/system.rs");
        let Ok(body) = std::fs::read_to_string(&system_rs) else {
            // A checkout without the sibling crate is not this test's failure
            // to report, and `AGENTS.md` §7 forbids concluding absence from one
            // probe with no way to tell an error from a true negative.
            return;
        };
        let marker = "pub const NETD_PATH: &str = \"";
        let Some(rest) = body.split(marker).nth(1) else {
            unreachable!("cmd/rivet-image no longer declares NETD_PATH; this tripwire is blind")
        };
        let installed = rest.split('"').next().unwrap_or("");
        assert_eq!(
            installed, NETD_PATH,
            "cmd/rivet-image installs rivet-netd at {installed}, and this supervisor \
             starts it from {NETD_PATH}"
        );
    }

    /// **The peers file lives under `ADR-020` §5's secrets directory**, and a
    /// `const` cannot be built from another `const` by concatenation — so the
    /// agreement is asserted rather than expressed. `ADR-011`: two spellings of
    /// one path is the arrangement in which they come to disagree, and this is
    /// what notices.
    #[test]
    fn the_peers_file_is_under_the_secrets_directory_the_core_names() {
        assert!(
            MCPD_PEERS_PATH.starts_with(rivet_core::state::SECRETS),
            "{MCPD_PEERS_PATH} is not under {}",
            rivet_core::state::SECRETS
        );
        assert_eq!(MCPD_IDENTITY_DIR, rivet_core::state::SECRETS);
    }

    /// [`the_netd_path_matches_the_one_the_image_installs`]'s tripwire, for the
    /// eighth spelling of a `/system/bin` path.
    #[test]
    fn the_bluetoothd_path_matches_the_one_the_image_installs() {
        let system_rs =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../rivet-image/src/system.rs");
        let Ok(body) = std::fs::read_to_string(&system_rs) else {
            return;
        };
        let marker = "pub const BLUETOOTHD_PATH: &str = \"";
        let Some(rest) = body.split(marker).nth(1) else {
            unreachable!(
                "cmd/rivet-image no longer declares BLUETOOTHD_PATH; this tripwire is blind"
            )
        };
        let installed = rest.split('"').next().unwrap_or("");
        assert_eq!(
            installed, BLUETOOTHD_PATH,
            "cmd/rivet-image installs rivet-bluetoothd at {installed}, and this supervisor \
             starts it from {BLUETOOTHD_PATH}"
        );
    }

    /// The same tripwire for the ninth spelling — `ADR-050` §2's launcher.
    #[test]
    fn the_launcher_path_matches_the_one_the_image_installs() {
        let system_rs =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../rivet-image/src/system.rs");
        let Ok(body) = std::fs::read_to_string(&system_rs) else {
            return;
        };
        let marker = "pub const LAUNCHER_PATH: &str = \"";
        let Some(rest) = body.split(marker).nth(1) else {
            unreachable!("cmd/rivet-image no longer declares LAUNCHER_PATH; this tripwire is blind")
        };
        let installed = rest.split('"').next().unwrap_or("");
        assert_eq!(
            installed, LAUNCHER_PATH,
            "cmd/rivet-image installs rivet-launcher at {installed}, and this supervisor \
             starts it from {LAUNCHER_PATH}"
        );
    }

    /// The same tripwire for the tenth spelling — `ADR-058` §4's compositor.
    #[test]
    fn the_displayd_path_matches_the_one_the_image_installs() {
        let system_rs =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../rivet-image/src/system.rs");
        let Ok(body) = std::fs::read_to_string(&system_rs) else {
            return;
        };
        let marker = "pub const DISPLAYD_PATH: &str = \"";
        let Some(rest) = body.split(marker).nth(1) else {
            unreachable!("cmd/rivet-image no longer declares DISPLAYD_PATH; this tripwire is blind")
        };
        let installed = rest.split('"').next().unwrap_or("");
        assert_eq!(
            installed, DISPLAYD_PATH,
            "cmd/rivet-image installs rivet-displayd at {installed}, and this supervisor \
             starts it from {DISPLAYD_PATH}"
        );
    }

    /// **The compositor is started before its first client.** `ADR-058` §4
    /// makes the launcher `displayd`'s first client, and the supervisor starts
    /// this set in order — so a launcher ahead of the compositor is a launcher
    /// connecting to a socket nothing is accepting on yet.
    #[test]
    fn the_compositor_is_started_before_the_launcher_that_draws_through_it() {
        let services = system_services();
        let names: Vec<&str> = services.iter().map(|d| d.name.as_str()).collect();
        let compositor = names.iter().position(|n| *n == DISPLAYD_NAME);
        let client = names.iter().position(|n| *n == LAUNCHER_NAME);
        assert!(
            compositor < client,
            "rivet-displayd must come before rivet-launcher in {names:?}"
        );
    }

    /// **It provides and does not connect**, which is `ADR-058` §4's split
    /// stated as data: mechanism provides the surface, policy consumes it.
    #[test]
    fn the_compositor_provides_the_wayland_socket_and_asks_for_nothing() {
        let def = displayd();
        assert!(!def.enabled, "every card would come up driving the screen");
        assert_eq!(def.name, "rivet-displayd");
        assert_eq!(def.program.path, DISPLAYD_PATH);
        assert!(
            def.connects.is_empty(),
            "the compositor consumes no endpoint; it is the provider"
        );
        // **Two endpoints, and every one of them must be BOUND.** A `provides`
        // without a matching `listen` is a socket nothing creates — the
        // failure `bluetoothd`'s own test names — and a caller finds `ENOENT`
        // on a path the supervisor advertised.
        for endpoint in [
            rivet_core::compositor::ENDPOINT,
            rivet_core::compositor::CAPTURE_ENDPOINT,
        ] {
            assert!(
                def.provides
                    .iter()
                    .any(|e| e.owner() == "socket" && e.name() == endpoint),
                "{endpoint} is not provided: {:?}",
                def.provides
            );
            assert!(
                def.listen.iter().any(|s| matches!(s,
                    rivet_core::package::application::Socket::Unix(name) if name == endpoint)),
                "{endpoint} is provided and not bound: {:?}",
                def.listen
            );
        }
        assert_eq!(def.provides.len(), 2, "{:?}", def.provides);
        assert_eq!(def.listen.len(), 2, "{:?}", def.listen);
    }

    /// **It ships OFF**, which is the whole of `ADR-050` §2: *"headless
    /// leaves launcher off, kiosk adds launcher"* is the document's answer,
    /// and a starting value of `true` would make every card a kiosk.
    #[test]
    fn the_launcher_ships_disabled_because_the_document_decides_it() {
        let def = launcher();
        assert!(!def.enabled, "every card would come up holding the screen");
        assert_eq!(def.name, "rivet-launcher");
        assert_eq!(def.program.path, LAUNCHER_PATH);
    }

    /// **Both endpoints the launcher needs, and neither is optional.**
    ///
    /// `rivet-sysd:kiosk` is what lets it select an application. Since
    /// `ADR-058` §4 it no longer decides what device node the launcher gets —
    /// `Supervisor::display_role` reads the compositor's `provides` for that —
    /// and `rivet-displayd:wayland` is what lets it reach the screen at all.
    ///
    /// The endpoints are parsed rather than constructed and collected from an
    /// `Option`, so a refusal degrades to an EMPTY list rather than a panic —
    /// which is exactly the shipped state this asserts against. A count of two
    /// is therefore a claim that BOTH parsed.
    #[test]
    fn the_launcher_declares_the_kiosk_socket_and_the_compositor() {
        let def = launcher();
        assert_eq!(def.connects.len(), 3, "an endpoint did not parse: {:?}", def.connects);
        assert!(
            def.connects.iter().any(|e| e.owner() == rivet_core::compositor::PROVIDER
                && e.name() == rivet_core::compositor::CAPTURE_ENDPOINT),
            "the launcher cannot forward a capture request: {:?}",
            def.connects
        );
        assert!(
            def.connects.iter().any(|e| e.owner() == rivet_core::kiosk::PROVIDER
                && e.name() == rivet_core::kiosk::ENDPOINT),
            "the launcher cannot select an application: {:?}",
            def.connects
        );
        assert!(
            def.connects.iter().any(|e| e.owner() == rivet_core::compositor::PROVIDER
                && e.name() == rivet_core::compositor::ENDPOINT),
            "the launcher cannot reach a screen: {:?}",
            def.connects
        );
    }

    /// **It declares no display, and that is `ADR-058` §3 as an assertion.**
    ///
    /// The ADR's claim is that a program drawing through a handed surface
    /// reaches exactly one thing — the surface — and needs none of §1.1's five
    /// confinement grants. `Device::Display` is what grants them, so this test
    /// is the difference between that being true and being said.
    #[test]
    fn the_launcher_asks_for_no_display_device_at_all() {
        let def = launcher();
        assert!(
            !def.devices
                .contains(&rivet_core::package::application::Device::Display),
            "a Wayland client was granted the DRM nodes: {:?}",
            def.devices
        );
        assert_eq!(
            def.devices,
            vec![rivet_core::package::application::Device::Input],
            "it still reads evdev for the ADR-015 vocabulary"
        );
    }

    /// And the agent's press, on `bluetoothd`'s terms: a `provides` that is not
    /// also a `listen` is a socket nothing binds.
    #[test]
    fn the_launcher_provides_the_input_endpoint_and_binds_it() {
        let def = launcher();
        assert_eq!(def.name, rivet_core::input::PROVIDER);
        assert_eq!(def.provides.len(), 1, "the endpoint did not parse");
        assert_eq!(def.provides[0].owner(), "socket");
        assert_eq!(def.provides[0].name(), rivet_core::input::ENDPOINT);
        assert!(
            def.listen
                .iter()
                .any(|s| matches!(s,
                    rivet_core::package::application::Socket::Unix(name)
                        if name == rivet_core::input::ENDPOINT)),
            "the provided endpoint is not bound: {:?}",
            def.listen
        );
    }

    /// **Every declaration here restates one in
    /// `deploy/launcher/Cargo.toml`**, which is the manifest that shipped this
    /// program as an application, and an image service has no manifest for
    /// `crate::applications` to read. So the two are compared against each
    /// other rather than trusted to agree — this reads that file.
    ///
    /// It returns rather than failing when the file is absent, on the path
    /// tripwires' terms: a checkout that does not have it is not this test's to
    /// refuse.
    #[test]
    fn the_launcher_definition_matches_the_manifest_it_replaces() {
        let at = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../deploy/launcher/Cargo.toml");
        let Ok(body) = std::fs::read_to_string(&at) else {
            return;
        };
        let def = launcher();

        assert!(
            body.contains("restart = \"always\""),
            "the manifest no longer says restart = always; this definition does"
        );
        assert_eq!(def.restart, RestartPolicy::Always);
        assert!(
            body.contains("ready   = \"none\"") || body.contains("ready = \"none\""),
            "the manifest no longer says ready = none; this definition does"
        );
        assert_eq!(def.readiness, Readiness::OnStart);
        assert!(
            body.contains("connects = [\"rivet-sysd:kiosk\", \"rivet-displayd:wayland\", \"rivet-displayd:capture\"]"),
            "the manifest no longer declares both endpoints — the kiosk socket selects an \
             application and the compositor is how this program reaches a screen at all \
             (ADR-058 §4.1)"
        );
        assert!(
            body.contains("provides = [\"socket:input\"]"),
            "the manifest no longer provides socket:input"
        );
        assert!(
            body.contains("devices = [\"input\"]"),
            "the manifest declares a device class this definition does not — and `display` \
             coming back would re-grant a Wayland client the DRM nodes ADR-058 §3.2 removed"
        );
        assert_eq!(
            def.devices,
            vec![rivet_core::package::application::Device::Input]
        );
    }

    /// **The half of the contract an application writes.** An application says
    /// `connects = ["rivet-bluetoothd:advertisements"]`, and if either half of
    /// that pair drifts the supervisor refuses the application by name and the
    /// machine looks like it has no Bluetooth.
    ///
    /// `provides` is built through the manifest parser and collected from an
    /// `Option`, so a refusal would produce an EMPTY list rather than a panic —
    /// which is the honest degradation and exactly the state this asserts is
    /// not the shipped one.
    #[test]
    fn the_endpoint_this_service_provides_is_the_one_an_application_names() {
        let def = bluetoothd();
        assert_eq!(def.name, "rivet-bluetoothd");
        assert_eq!(def.provides.len(), 1, "the endpoint did not parse");
        assert_eq!(def.provides[0].owner(), "socket");
        assert_eq!(def.provides[0].name(), rivet_core::bluetooth::ENDPOINT);
        assert_eq!(def.name, rivet_core::bluetooth::PROVIDER);
    }

    /// A `provides` that is not also a `listen` is a socket nothing binds.
    /// `crate::applications` folds a manifest's the same way and this file has
    /// to do it by hand, which is precisely the kind of thing that gets left
    /// out.
    #[test]
    fn the_provided_endpoint_is_also_a_listening_socket() {
        let def = bluetoothd();
        let unix: Vec<&str> = def
            .listen
            .iter()
            .filter_map(|s| match s {
                rivet_core::package::application::Socket::Unix(name) => Some(name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(unix, vec![rivet_core::bluetooth::ENDPOINT]);
    }

    /// It ships OFF, and the three reasons are on [`bluetoothd`]. A change here
    /// makes every board hold its radio.
    #[test]
    fn the_ble_observer_ships_disabled_and_takes_no_arguments() {
        let def = bluetoothd();
        assert!(!def.enabled, "ADR-041 6 ships this disabled");
        assert!(def.program.args.is_empty(), "it asks the kernel instead");
        assert_eq!(def.restart, RestartPolicy::OnFailure);
    }

    // -- rivet-proxyd, ADR-060 ---------------------------------------------

    fn proxy_name() -> rivet_core::service::ServiceName {
        rivet_core::service::ServiceName::new(PROXYD_NAME).expect("the proxy's name is a name")
    }

    /// **It runs as the uid it had as an application, under a ruleset, and not
    /// as root** — `ADR-060` §2. The uid is what keeps the certificate already
    /// in its data directory readable across the move.
    #[test]
    fn the_proxy_runs_as_its_own_uid_under_a_ruleset() {
        let def = proxyd();
        assert_eq!(def.origin, crate::service::Origin::Image);
        assert_eq!(
            def.program.identity,
            Some(rivet_core::service::app_identity(&proxy_name()))
        );
        let policy = def
            .program
            .confinement
            .as_ref()
            .expect("an image service with an identity is confined");
        let program = policy
            .grants
            .iter()
            .find(|g| g.path == PROXYD_PATH)
            .expect("the ruleset grants the program it is about to exec");
        assert!(program.access.holds(rivet_core::confinement::Access::EXECUTE));
        assert_eq!(def.volume.as_deref(), Some(rivet_core::service::BUILT_IN_VOLUME));
        assert_eq!(def.validate(), Ok(()));
    }

    /// **The environment an application is handed**, from the same functions,
    /// so the proxy's own code cannot tell which way it was delivered.
    #[test]
    fn the_proxy_is_handed_its_data_settings_and_scratch_directories() {
        let def = proxyd();
        let volume = rivet_core::service::BUILT_IN_VOLUME;
        let name = proxy_name();
        let env: std::collections::BTreeMap<_, _> = def.program.env.iter().cloned().collect();
        assert_eq!(
            env.get(rivet_core::service::APP_DATA_ENV),
            Some(&rivet_core::service::data_dir(volume, &name))
        );
        assert_eq!(
            env.get(rivet_core::service::APP_SETTINGS_ENV),
            Some(&rivet_core::settings::path(volume, &name))
        );
        let scratch = rivet_core::service::tmp_dir(volume, &name);
        assert_eq!(env.get(rivet_core::service::APP_TMP_ENV), Some(&scratch));
        assert_eq!(env.get(rivet_core::service::TMPDIR_ENV), Some(&scratch));
        assert_eq!(
            env.get(rivet_core::service::APP_DIR_ENV),
            None,
            "there is no installed version directory to name"
        );
        assert_eq!(env.get(rivet_core::service::APP_DATA_ENV).map(String::as_str), Some("/state/app/rivet-proxyd/data"));
    }

    /// It ships DISABLED and the role turns it on (`ADR-050` §2's tier 2), on
    /// 443 on every address.
    #[test]
    fn the_proxy_ships_disabled_and_holds_443_on_every_address() {
        let def = proxyd();
        assert!(!def.enabled, "the document's role decides");
        assert!(!def.proxied, "it is the proxy, not behind it");
        assert_eq!(
            def.listen,
            vec![rivet_core::package::application::Socket::Tcp(443)]
        );
        assert_eq!(def.program.path, "/system/bin/rivet-proxyd");
    }

    /// **The definition says what the proxy's manifest said**, on
    /// `the_launcher_definition_matches_the_manifest_it_replaces`'s terms: the
    /// manifest stays in `cmd/rivet-proxyd/Cargo.toml`, and the restatement
    /// here is the kind of thing that drifts.
    #[test]
    fn the_proxy_definition_matches_the_manifest_it_replaces() {
        let at = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../rivet-proxyd/Cargo.toml");
        let Ok(body) = std::fs::read_to_string(&at) else {
            eprintln!("cmd/rivet-proxyd/Cargo.toml was not readable; this test is blind");
            return;
        };
        let def = proxyd();
        assert!(
            body.contains("restart = \"on-failure\""),
            "the manifest no longer says restart = on-failure; this definition does"
        );
        assert_eq!(def.restart, RestartPolicy::OnFailure);
        assert!(
            body.contains("listen  = [\"tcp:443\"]") || body.contains("listen = [\"tcp:443\"]"),
            "the manifest no longer listens on tcp:443; this definition does"
        );
        let home = rivet_core::service::image_service_home(PROXYD_NAME)
            .expect("the proxy is an image service with a home");
        assert_eq!(home.program, PROXYD_PATH);
        let declared = home
            .settings
            .iter()
            .map(|k| format!("\"{k}\""))
            .collect::<Vec<_>>()
            .join(", ");
        assert!(
            body.contains(&format!("settings = [{declared}]")),
            "the manifest's settings are not {declared}, which is what the image delivers"
        );
    }

    /// **The name service runs as its own uid under its own ruleset**, never
    /// as root — `ADR-047` §6. It parses datagrams from any device on the
    /// network, so a defect in its query path must not run with root's reach.
    #[test]
    fn the_name_service_runs_under_its_own_identity_and_ruleset() {
        let def = dnsd();
        let name = rivet_core::service::ServiceName::new(DNSD_NAME).expect("a valid name");
        let identity = def.program.identity.expect("the name service has an identity");
        assert_eq!(identity, rivet_core::service::app_identity(&name));
        assert_ne!(identity.uid, 0, "never root");
        assert_ne!(identity.gid, 0, "never root's group");
        assert_eq!(def.volume.as_deref(), Some(rivet_core::service::BUILT_IN_VOLUME));
        let policy = def
            .program
            .confinement
            .as_ref()
            .expect("an image service with an identity is confined");
        assert_eq!(
            policy,
            &rivet_core::confinement::image_service_policy(
                DNSD_PATH,
                rivet_core::service::BUILT_IN_VOLUME,
                &name,
                Some(std::process::id()),
                &[],
            ),
            "the ruleset is the image-service policy and nothing composed beside it"
        );
        assert_eq!(def.validate(), Ok(()));
    }

    /// **The compositor runs as its own uid under its own ruleset**, never as
    /// root — `ADR-058` §4. Every client's bytes reach it, so a defect in a
    /// protocol handler must not run with root's reach. The ruleset is the
    /// image-service policy over the definition's own `devices`, so the
    /// declaration that earns the card nodes and the one that earns Mesa's
    /// lookup are one field.
    #[test]
    fn the_compositor_runs_under_its_own_identity_and_ruleset() {
        let def = displayd();
        let name = rivet_core::service::ServiceName::new(DISPLAYD_NAME).expect("a valid name");
        let identity = def.program.identity.expect("the compositor has an identity");
        assert_eq!(identity, rivet_core::service::app_identity(&name));
        assert_ne!(identity.uid, 0, "never root");
        assert_ne!(identity.gid, 0, "never root's group");
        assert_eq!(def.volume.as_deref(), Some(rivet_core::service::BUILT_IN_VOLUME));
        assert_eq!(
            def.program.confinement.as_ref(),
            Some(&rivet_core::confinement::image_service_policy(
                DISPLAYD_PATH,
                rivet_core::service::BUILT_IN_VOLUME,
                &name,
                Some(std::process::id()),
                &def.devices,
            )),
            "the ruleset is the image-service policy over its own declaration"
        );
        assert_eq!(def.validate(), Ok(()));
    }

    /// **It declares the screen and nothing else.** Under its own uid a
    /// declared class is a chown of every node it expands to, and the
    /// compositor reads no input device — the launcher does, and declares it.
    #[test]
    fn the_compositor_declares_the_display_and_not_the_input_devices() {
        assert_eq!(
            displayd().devices,
            vec![rivet_core::package::application::Device::Display],
            "a class it does not read is every keyboard on the machine handed over for nothing"
        );
    }

    /// **The tripwire between `IMAGE_SERVICE_HOMES` and the definitions.**
    ///
    /// A name in the table is a promise that the service runs as its own uid,
    /// under its own ruleset, from a home on the built-in volume, executing the
    /// program the table names. A definition that kept uid 0 while its name
    /// sat in the table would be given an account, a home and a settings file
    /// and still run as root.
    #[test]
    fn every_image_service_with_a_home_is_defined_confined_under_its_own_uid() {
        let defs = system_services();
        for home in rivet_core::service::IMAGE_SERVICE_HOMES {
            let def = defs
                .iter()
                .find(|d| d.name == home.name)
                .unwrap_or_else(|| panic!("{} has a home and no definition", home.name));
            let name = rivet_core::service::ServiceName::new(home.name).expect("a valid name");
            assert_eq!(def.program.path, home.program, "{}", home.name);
            assert_eq!(
                def.program.identity,
                Some(rivet_core::service::app_identity(&name)),
                "{} does not run as its own uid",
                home.name
            );
            assert!(def.program.confinement.is_some(), "{} is not confined", home.name);
            assert_eq!(
                def.volume.as_deref(),
                Some(rivet_core::service::BUILT_IN_VOLUME),
                "{} has no home",
                home.name
            );
        }
        assert!(
            rivet_core::service::image_service_home(DNSD_NAME).is_some(),
            "the name service is one of them"
        );
        assert!(
            rivet_core::service::image_service_home(DISPLAYD_NAME).is_some(),
            "the compositor is one of them"
        );
        assert!(
            rivet_core::service::image_service_home(MDNSD_NAME).is_some(),
            "the mDNS responder is one of them"
        );
    }

    /// **The mDNS responder runs as its own uid under its own ruleset**, never
    /// as root — `ADR-017` §3. It parses datagrams from any device on the
    /// link, so a defect in that path must not run with root's reach. It
    /// reads its projection and nothing else, and opens its own socket: UDP
    /// 5353 needs no privilege, so nothing is handed to it.
    #[test]
    fn the_mdns_responder_runs_under_its_own_identity_and_reads_its_projection() {
        let def = mdnsd();
        let name = rivet_core::service::ServiceName::new(MDNSD_NAME).expect("a valid name");
        let identity = def.program.identity.expect("the responder has an identity");
        assert_eq!(identity, rivet_core::service::app_identity(&name));
        assert_ne!(identity.uid, 0, "never root");
        assert_ne!(identity.gid, 0, "never root's group");
        assert_eq!(def.volume.as_deref(), Some(rivet_core::service::BUILT_IN_VOLUME));
        assert_eq!(
            def.program.confinement.as_ref(),
            Some(&rivet_core::confinement::image_service_policy(
                MDNSD_PATH,
                rivet_core::service::BUILT_IN_VOLUME,
                &name,
                Some(std::process::id()),
                &[],
            )),
            "the ruleset is the image-service policy and nothing composed beside it"
        );
        assert_eq!(
            def.program.args,
            [
                "--projection".to_string(),
                "/state/app/rivet-mdnsd/mdns".to_string()
            ],
            "it reads its projection, never the machine document"
        );
        assert!(def.listen.is_empty(), "it opens its own socket");
        assert!(!def.enabled, "the document turns it on");
        assert_eq!(def.validate(), Ok(()));
    }

    /// **The time client runs as its own uid under its own ruleset, keeping
    /// `CAP_SYS_TIME` and nothing else** — `ADR-021` §7. It parses replies from
    /// servers on the internet, so a defect there must not run with root's
    /// reach; setting the wall clock is the one thing it does that needs
    /// privilege. Its UDP socket binds an ephemeral port, so nothing is
    /// handed to it and no other capability is kept.
    #[test]
    fn the_time_client_runs_under_its_own_identity_keeping_only_cap_sys_time() {
        let def = timed();
        let name = rivet_core::service::ServiceName::new(TIMED_NAME).expect("a valid name");
        let identity = def.program.identity.expect("the time client has an identity");
        assert_eq!(identity, rivet_core::service::app_identity(&name));
        assert_ne!(identity.uid, 0, "never root");
        assert_ne!(identity.gid, 0, "never root's group");
        assert_eq!(def.volume.as_deref(), Some(rivet_core::service::BUILT_IN_VOLUME));
        assert_eq!(
            def.program.confinement.as_ref(),
            Some(&rivet_core::confinement::image_service_policy(
                TIMED_PATH,
                rivet_core::service::BUILT_IN_VOLUME,
                &name,
                Some(std::process::id()),
                &[],
            )),
            "the ruleset is the image-service policy and nothing composed beside it"
        );
        assert_eq!(
            def.program.capabilities,
            [crate::kernel::Capability::SYS_TIME],
            "exactly CAP_SYS_TIME"
        );
        assert!(def.listen.is_empty(), "it opens its own socket on an ephemeral port");
        assert!(def.enabled, "the image installs it, so it is on");
        assert_eq!(def.validate(), Ok(()));
        assert!(
            rivet_core::service::image_service_home(TIMED_NAME).is_some(),
            "the time client is one of the homed image services"
        );
    }

    /// **Which services keep a capability, pinned.** A kept capability is a
    /// piece of root a program holds; one appearing on another definition is
    /// a decision, and this test makes it a visible one.
    #[test]
    fn which_image_services_keep_a_capability_is_pinned() {
        let keeping: Vec<_> = system_services()
            .into_iter()
            .filter(|d| !d.program.capabilities.is_empty())
            .map(|d| d.name)
            .collect();
        assert_eq!(keeping, [NETD_NAME, TIMED_NAME, BLUETOOTHD_NAME]);
    }

    /// **The BLE observer runs as its own uid, keeping `CAP_NET_ADMIN` and
    /// `CAP_SETPCAP` and nothing else** — `ADR-041` §6, `ADR-017` §3.
    /// `CAP_NET_ADMIN` is what binding the controller's user channel checks;
    /// `CAP_SETPCAP` is what lets the daemon empty its own bounding set once
    /// that bind is done. Both are dropped by the daemon itself, so the
    /// definition names the ceiling for the moments before the bind.
    #[test]
    fn the_ble_observer_runs_as_its_own_uid_keeping_only_what_the_bind_and_the_drop_need() {
        let def = bluetoothd();
        let name = rivet_core::service::ServiceName::new(BLUETOOTHD_NAME).expect("a valid name");
        let identity = def.program.identity.expect("the observer has an identity");
        assert_eq!(identity, rivet_core::service::app_identity(&name));
        assert_ne!(identity.uid, 0, "never root");
        assert_ne!(identity.gid, 0, "never root's group");
        assert_eq!(def.volume.as_deref(), Some(rivet_core::service::BUILT_IN_VOLUME));
        assert_eq!(
            def.program.confinement.as_ref(),
            Some(&rivet_core::confinement::image_service_policy(
                BLUETOOTHD_PATH,
                rivet_core::service::BUILT_IN_VOLUME,
                &name,
                Some(std::process::id()),
                &[],
            )),
            "the ruleset is the image-service policy and nothing composed beside it"
        );
        assert_eq!(
            def.program.capabilities,
            [
                crate::kernel::Capability::NET_ADMIN,
                crate::kernel::Capability::SETPCAP
            ],
            "exactly CAP_NET_ADMIN and CAP_SETPCAP"
        );
        assert_eq!(
            crate::kernel::capability_mask(&def.program.capabilities),
            0x1100,
            "bits 8 and 12"
        );
        assert_eq!(def.validate(), Ok(()));
        assert!(
            rivet_core::service::image_service_home(BLUETOOTHD_NAME).is_some(),
            "the observer is one of the homed image services"
        );
    }

    fn document(text: &str) -> rivet_config::Document {
        rivet_config::Document::parse(text).expect("a valid document")
    }

    /// **One supervised DHCP client per DHCP interface, and none for a
    /// static one** — R-253 point 1. Each is `udhcpc` on the image's path with
    /// the arguments `rivet_adapter_net::dhcp::arguments` gives, restarted
    /// whatever its exit, uid 0 with the network's three capabilities.
    #[test]
    fn each_dhcp_interface_gets_a_supervised_client_and_a_static_one_does_not() {
        let services = network_services(&document(
            "version = 1\n\
             [network.eth0]\nmethod = \"dhcp\"\n\
             [network.eth1]\nmethod = \"static\"\naddress = \"192.168.1.50/24\"\n",
        ));
        let names: Vec<_> = services.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["rivet-dhcp-eth0"]);
        let client = &services[0];
        assert_eq!(client.program.path, rivet_adapter_net::dhcp::MECHANISM);
        assert_eq!(client.program.args, rivet_adapter_net::dhcp::arguments("eth0"));
        assert_eq!(client.restart, RestartPolicy::Always);
        assert!(client.enabled);
        assert_eq!(client.origin, crate::service::Origin::Image);
        assert_eq!(client.program.identity, None);
        assert_eq!(client.program.capabilities, NETWORK_CAPABILITIES);
        assert_eq!(client.validate(), Ok(()));
    }

    /// **A wireless interface gets a supervised supplicant**, and a wireless
    /// DHCP interface gets both — the supplicant to join, the client to ask.
    #[test]
    fn a_wireless_interface_gets_a_supervised_supplicant() {
        let services = network_services(&document(
            "version = 1\n[network.wlan0]\nmethod = \"dhcp\"\nssid = \"home\"\n",
        ));
        let names: Vec<_> = services.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["rivet-wpa-wlan0", "rivet-dhcp-wlan0"]);
        let supplicant = &services[0];
        assert_eq!(supplicant.program.path, rivet_adapter_net::supplicant::MECHANISM);
        assert_eq!(
            supplicant.program.args,
            rivet_adapter_net::supplicant::arguments("wlan0")
        );
        assert_eq!(supplicant.restart, RestartPolicy::Always);
        assert_eq!(supplicant.program.capabilities, NETWORK_CAPABILITIES);
        assert_eq!(supplicant.validate(), Ok(()));
    }

    /// A machine with no `[network]` table, or a static one, derives nothing.
    #[test]
    fn a_machine_with_no_dhcp_and_no_wireless_derives_nothing() {
        assert!(network_services(&document("version = 1\n")).is_empty());
        assert!(
            network_services(&document(
                "version = 1\n[network.eth0]\nmethod = \"static\"\naddress = \"10.0.0.2/24\"\n"
            ))
            .is_empty()
        );
    }

    /// **The derived services keep the machine reachable**, like `rivet-netd`:
    /// a reload may not stop one, and an operator may not hold one stopped.
    #[test]
    fn the_networks_derived_services_keep_the_machine_reachable() {
        assert!(keeps_the_machine_reachable("rivet-dhcp-eth0"));
        assert!(keeps_the_machine_reachable("rivet-wpa-wlan0"));
        assert!(keeps_the_machine_reachable(NETD_NAME));
        assert!(keeps_the_machine_reachable(MCPD_NAME));
        assert!(!keeps_the_machine_reachable(TIMED_NAME));
    }

    /// **`rivet-netd` stays uid 0 and keeps the network's three
    /// capabilities and nothing else**: netlink writes, the raw socket a
    /// wireless join reads, and a port below 1024. No uid of its own and no
    /// ruleset, so the kernel reduces its bounding set and sets `NoNewPrivs`
    /// and changes nothing else.
    #[test]
    fn netd_is_uid_0_with_its_bounding_set_reduced_to_the_networks_three() {
        let d = netd();
        assert_eq!(d.program.identity, None, "it stays uid 0");
        assert_eq!(d.program.confinement, None, "a ruleset on uid 0 is refused");
        assert_eq!(
            d.program.capabilities,
            NETWORK_CAPABILITIES,
            "exactly CAP_NET_ADMIN, CAP_NET_RAW and CAP_NET_BIND_SERVICE"
        );
        assert_eq!(
            crate::kernel::capability_mask(&d.program.capabilities),
            0x3400,
            "bits 10, 12 and 13"
        );
        assert_eq!(d.validate(), Ok(()));
    }
}
