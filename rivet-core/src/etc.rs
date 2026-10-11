//! `/etc`, as data rather than as files: **the definition the one generator
//! renders.**
//!
//! `ADR-008` §5 is the framing: `/etc` "is not part of Rivet's design at all.
//! It exists solely because the C mechanisms of `ADR-001` expect files at paths
//! they hard-coded." `ADR-008` §5.1 lists those paths and says they are musl's,
//! compiled in as string literals. `ADR-007` §3.5 says the contents are
//! generated output and never authored, on a tmpfs.
//!
//! All of that is still true. What was missing is *who generates it*.
//!
//! # One generator, and it renders all of `/etc` — ruled 2026-08-23
//!
//! `/etc` has **one** generator — the runtime projection — and it renders all
//! of `/etc`. `cmd/rivet-image` produces nothing under `/etc`. The full
//! reasoning is `ADR-007` §3.5 and the short form is that nothing about a file
//! being a fact-about-the-image *requires* it to be made at build time: the
//! projection runs before anything that reads `/etc` starts, so it can render
//! everything, and one generator needs no argument about disjointness to
//! satisfy `ADR-011`.
//!
//! | Fact | Authority | Reaches `/etc` from |
//! | :-- | :-- | :-- |
//! | which binaries may be login shells | the **image's own tree** | [`LOGIN_SHELL_PATHS`], probed at runtime |
//! | which uid the image's files belong to | the **image** | [`SYSTEM_IDENTITIES`] |
//! | which uid each installed application runs as | its **name** — `crate::service::app_identity` | [`EtcInputs::applications`], listed at runtime |
//! | what `localhost` means | a property of the loopback interface | `render_hosts` |
//! | which humans have accounts, and their passwords | the **document** | no schema key yet — `ADR-008` §7.3, Stage 4 |
//! | which nameservers to use | the **document** (`[network.resolver]`), else the **DHCP lease** | [`EtcInputs::nameservers`] — the static half since 2026-09-01, the lease half since 2026-09-23 (`ADR-023`); `cmd/rivet-sysd`'s `leased_resolver` reads `/run/rivet/dhcp/*.lease` when the document is silent |
//! | what timezone this machine keeps | the **document** (`ADR-021`) | ~~no schema key yet~~ **DONE 2026-09-02** — the maintainer ruled the `timezone` key, and [`EtcInputs::local_time`] carries it to [`LOCALTIME`] |
//!
//! **What survives unchanged is the expensive half**: the nine musl paths, the
//! argument for shipping or not shipping each file, and the renderers. That
//! knowledge cost a sweep of the linked `libc.a` and found a ninth path
//! `ADR-008` §5.1 had only predicted.
//!
//! # Who calls this, and where the I/O is
//!
//! `cmd/rivet-sysd`'s `etc` module, before it converges any service —
//! ruled 2026-08-23, `ADR-017` §3.1.8. **Not PID 1**, and that is a ruling
//! rather than an omission: `ADR-017` §3.1.5's job list did not grow.
//!
//! Nothing here performs I/O. This module renders *bytes and modes*; mounting
//! the tmpfs, writing the files and remounting it read-only are the
//! supervisor's, behind its `Kernel` port, exactly as `crate::mount` and
//! `crate::state` render *plans* that `cmd/rivet-initd` performs.
//!
//! # Which files ship, and the rule that decides it
//!
//! > **Ship a file when its absence produces a *wrong answer*. Leave it out
//! > when its absence produces a clean failure.**
//!
//! That rule is `AGENTS.md` §5's *refuse rather than guess* pointed at musl's
//! fallbacks, and it decides all eight of `ADR-008` §5.1's paths without a
//! judgement call. Every fallback below was read out of **the exact `libc.a`
//! Rivet links** — `aarch64-unknown-linux-musl`, from the toolchain's
//! `self-contained` directory — rather than from musl's documentation, per
//! `AGENTS.md` §5.
//!
//! | Path | Absent means | Verdict |
//! | :-- | :-- | :-- |
//! | `/etc/shells` | musl substitutes a built-in list — see [`MUSL_DEFSHELLS`] | **ship** — wrong answer |
//! | `/etc/hosts` | `localhost` is looked up **over DNS** — see [`MUSL_HAS_NO_BUILTIN_LOCALHOST`] | **ship** — wrong answer |
//! | `/etc/passwd` | `getpwnam` finds nothing; uid 0 has no name | **ship** — the image's own files are uid 0 |
//! | `/etc/group` | `getgrgid` finds nothing; gid 5 has no name | **ship** — [`TTY_GID`] is already load-bearing |
//! | `/etc/shadow` | password auth falls back to `passwd`'s `x` | **ship** — see [`SHADOW`] |
//! | `/etc/resolv.conf` | musl uses `127.0.0.1`, where nothing listens | leave out — DHCP's, `ADR-023` |
//! | `/etc/localtime` | musl keeps **UTC** | ~~leave out — correct for an unconfigured machine~~ **ship, CONDITIONALLY, since 2026-09-02** |
//! | `/etc/services` | a service *name* fails to resolve; numeric ports are unaffected | leave out — nothing resolves one |
//!
//! **The `/etc/localtime` row changed on 2026-09-02 and the rule above did
//! not.** *Ship a file when its absence produces a wrong answer* decided that
//! row correctly for a machine nobody had told: UTC is not a wrong answer, it
//! is the honest answer for a machine with no opinion, and every Rivet built
//! before that date had none. `ADR-021` §9 gave the operator a way to have an
//! opinion, and the moment a document says `America/Chicago` the *absence* of
//! this file is a wrong answer — silently, with nothing anywhere saying so,
//! which is the sharpest instance of the rule in the table. So the file is
//! projected **when and only when the document names a zone and this image has
//! the tzfile for it**, and the machine says which of those failed when one
//! does. [`LOCALTIME`] and [`EtcInputs::local_time`] carry the detail.
//!
//! **`/etc/zoneinfo/` did NOT change with it**, and that is worth one sentence
//! because the two look like they should move together. It is one of the three
//! prefixes musl searches for a zone *by name*, and nothing on a Rivet asks by
//! name — `TZ` is unset, the database lives at [`ZONEINFO_DIR`] where the
//! projection reads it directly, and musl reads [`LOCALTIME`] and stops.
//!
//! # The evidence, and how it was taken
//!
//! `ar x` on `libc.a`, then `strings` and `objdump -s` on the individual
//! objects. Recorded here because `ADR-008` §5.1 states its own method and its
//! own limit, and this is a second, independent pass over a different artifact
//! — the linked library rather than the upstream sources.
//!
//! It **agrees with §5.1 on all eight paths and adds a ninth**, `/etc/zoneinfo/`
//! in `__tz.lo` (one of the prefixes musl searches for a named zone, alongside
//! `/usr/share/zoneinfo/` and `/share/zoneinfo/`). §5.1 predicted exactly this:
//! "treat an additional path found later as expected, not as a contradiction."
//! It is in [`MUSL_ETC_PATHS`] and it is not shipped, for the same reason
//! `/etc/localtime` is not.

use core::fmt::Write as _;

use crate::mount::MountFlag;

// ---------------------------------------------------------------------------
// Where /etc lives, as a plan for whoever performs it
// ---------------------------------------------------------------------------

/// `ADR-007` §3.5: `/etc` "exists, is generated, and lives on a tmpfs".
///
/// The directory itself ships in the image as an empty mount point
/// (`cmd/rivet-image`'s `system::ETC`), because the root filesystem is
/// read-only by format (`ADR-018` §3) and nothing can create it at runtime.
pub const ETC_MOUNT_POINT: &str = "/etc";

/// A tmpfs, per `ADR-007` §3.5. Nothing under `/etc` survives a reboot and
/// nothing should: it is a view of state held elsewhere (§3.6.2).
pub const ETC_FSTYPE: &str = "tmpfs";

/// `noexec,nosuid,nodev`, and each of the three is a statement rather than a
/// habit.
///
/// * **`noexec`** — nothing in `/etc` is a program. This is the opposite
///   judgement from `crate::mount`'s `/run`, which deliberately omits the flag
///   "because services are started from it"; nothing is ever started from
///   `/etc`, and `/etc/shells` naming a binary is a *reference* to one that
///   lives on `/system`.
/// * **`nosuid`** — nothing in Rivet is set-user-ID (`crate::state`).
/// * **`nodev`** — device nodes are the kernel's, on devtmpfs (`ADR-024` §3).
pub const ETC_FLAGS: &[MountFlag] = &[MountFlag::NoExec, MountFlag::NoSuid, MountFlag::NoDev];

/// The tmpfs options, and the size cap is the interesting half.
///
/// `mode=755` because `/etc` is looked up by unprivileged processes; a tmpfs
/// otherwise mounts `1777`, which is a world-writable `/etc` and exactly the
/// hand-edit `ADR-007` §3.6.1 item 1 exists to refuse.
///
/// **`size=1m`** rather than the tmpfs default of half of RAM. Every file
/// [`etc_projection`] renders is a few hundred bytes and the whole set is
/// asserted to fit far inside this — `the_whole_projection_fits_inside_the_tmpfs_cap`.
/// A default cap of "half the machine" is a cap that means nothing, and the
/// failure it permits is a generator bug reaching the OOM killer instead of
/// failing at the write it got wrong.
pub const ETC_TMPFS_DATA: Option<&str> = Some("mode=755,size=1m");

/// Where Rivet's own CLI goes when it is built — `ADR-008` §3, `ADR-016` §3.
///
/// ~~**Nothing is at this path today.** `cmd/rivet` is not in the workspace,
/// so the projection probes for it and finds nothing, and [`EtcInputs`]'s
/// `login_shell` is `None` in consequence.~~ **Struck 2026-08-23**, per
/// `AGENTS.md` §7 — `cmd/rivet` is in the workspace, `cmd/rivet-image`
/// installs it here, and a Raspberry Pi 4 has accepted a login against it
/// (`PLAN.md` §0). [`EtcInputs::login_shells`] documents the same
/// correction and this sentence was left behind by it, which is exactly the
/// failure §7 names.
///
/// The constant exists so that the probe, the test fixtures and
/// `cmd/rivet-image`'s `LOGIN_SHELLS` name one string rather than three
/// (`ADR-011`).
///
/// **Renamed from `LOGIN_SHELL_PATH` on 2026-08-24**, when the dual-login-shell ruling
/// ruled that this is no longer *the* login shell but *one of two*. The old
/// name said something that had stopped being true, and a constant whose name
/// makes a false claim is worse than one that is merely awkward to grep for.
pub const CLI_PATH: &str = "/system/bin/rivet";

/// Where `nu` goes — `ADR-005` §5.1's ruled interactive login shell.
///
/// Beside the Rust binaries in `/system/bin`, because
/// `ADR-008` §3 separates paths by **ownership and lifetime**: nushell is part
/// of the image, read-only, replaced wholesale by an update.
///
/// **It is not a system extension, and that is structural rather than a
/// preference** (`ADR-005` §5.6.2): this projection runs *before* anything
/// mounts an extension, so an extension-borne shell would be absent at exactly
/// the moment `/etc/shells` is written, and every login would be rejected
/// before authentication.
pub const NUSHELL_PATH: &str = "/system/bin/nu";

/// Every binary this machine is willing to call a login shell, **in preference
/// order**, ruled 2026-08-24.
///
/// Two things read this list and they read it differently, which is the whole
/// reason it is a list and not a string:
///
/// * **`/etc/passwd`'s seventh field** takes the **first entry that is actually
///   on the filesystem**. `nu` is first because §5.1 ruled it the interactive
///   login shell; `cmd/rivet` is second because it is the shell a machine
///   still has when `nu` was left out of the build.
/// * **`/etc/shells` names every entry that is present**, because
///   `getusershell()` is a membership test and an account
///   whose shell is unlisted is refused *before* authentication. That same rule
///   requires `cmd/rivet` to stay listed so that a future restricted account
///   naming it is not rejected at the gate.
///
/// **Order is the ruling; presence is the machine's.** Nothing here asserts
/// that either binary exists — `EtcInputs::login_shells` carries the answer to
/// that, and it is obtained by *asking the filesystem* (that same ruling). A card
/// built without `nu` therefore still projects a working `/etc/passwd` naming
/// `rivet`, which is the property that made this ruling cheap.
pub const LOGIN_SHELL_PATHS: &[&str] = &[NUSHELL_PATH, CLI_PATH];

// ---------------------------------------------------------------------------
// What musl does when a file is not there
// ---------------------------------------------------------------------------

/// The paths musl compiles in, from `ADR-008` §5.1 **plus one it did not have**.
///
/// Ordered as a sorted list rather than by importance, so that a diff against a
/// later sweep is readable.
///
/// `/etc/tcb/%s/shadow` is a format string rather than a path, so it is recorded
/// in the ADR and not here, where every entry has to be a real path something
/// can be asserted about. It sits beside `/etc/shadow` in `getspnam_r.lo`'s
/// read-only data and that object imports `snprintf` — **which shows the two are
/// alternatives and does not show which is tried first.** It does not matter to
/// Rivet either way: neither path exists on a board until this module puts one
/// there, and `getspnam_r` finding nothing is the outcome in both orders.
pub const MUSL_ETC_PATHS: &[&str] = &[
    "/etc/group",
    "/etc/hosts",
    "/etc/localtime",
    "/etc/passwd",
    "/etc/resolv.conf",
    "/etc/services",
    "/etc/shadow",
    "/etc/shells",
    // Found 2026-08-23 in the linked `libc.a`; not in `ADR-008` §5.1's table.
    "/etc/zoneinfo/",
];

/// What `getusershell()` returns when `/etc/shells` cannot be opened.
///
/// **This is the sharpest reason anything in this module exists**, and it is
/// read verbatim out of the library Rivet links:
///
/// ```text
/// $ objdump -s -j .rodata.defshells getusershell.lo
///  0000 2f62696e 2f73680a 2f62696e 2f637368  /bin/sh./bin/csh
///  0010 0a00                                 ..
/// ```
///
/// The object imports `fopen`, `fmemopen`, `getline` and `fclose` and nothing
/// else, which is musl's `setusershell()` opening `/etc/shells` and falling back
/// to a `fmemopen` over this buffer.
///
/// Measured on real hardware: a program that
/// admits a login calls `getusershell()` **before it looks at any credential**
/// and rejects an account whose shell is not listed, so a machine's `/etc/shells`
/// decides who may log in before anything about identity is considered. An
/// absent `/etc/shells` does not fail —
/// it silently declares that `/bin/sh` and `/bin/csh` are the machine's login
/// shells. Rivet ships neither today, so today the outcome is the same
/// rejection either way.
///
/// **It stops being the same the day a POSIX environment ships** (`ADR-005`
/// §5.4 keeps that possibility open as an add-on). Then an absent `/etc/shells`
/// would authorise `/bin/sh` as a login shell — the one thing `ADR-005` §5.4
/// specifically removed from the administration surface — without anything
/// having decided to. That is the wrong answer this file exists to prevent, and
/// it is invisible until it happens.
pub const MUSL_DEFSHELLS: &[&str] = &["/bin/sh", "/bin/csh"];

/// musl has **no** built-in meaning for `localhost`.
///
/// `strings` over the whole of `libc.a` finds the literal `localhost` **zero
/// times**, and `lookup_name.lo`'s `name_from_hosts` is the only function that
/// opens `/etc/hosts`. So without that file, `localhost` falls through
/// `name_from_numeric` and `name_from_hosts` to `name_from_dns` and is asked of
/// a nameserver.
///
/// That is a wrong answer in the strong sense: a name that is *definitionally*
/// this machine is resolved off it, over a network that may not exist, and
/// whatever comes back is believed.
pub const MUSL_HAS_NO_BUILTIN_LOCALHOST: bool = true;

/// What musl's resolver uses when `/etc/resolv.conf` is absent.
///
/// From `objdump -s -j .rodata.__get_resolv_conf.str1.8 resolvconf.lo`: the
/// object holds `/etc/resolv.conf`, the keyword set, and the literal
/// `127.0.0.1`.
///
/// This is why `/etc/resolv.conf` is **not** in the image. Its absence sends
/// queries to a loopback address where nothing listens, which fails cleanly and
/// quickly; a stale copy baked into a read-only filesystem would answer
/// confidently and wrongly, and could not be corrected on the running machine.
/// The nameserver set is the DHCP lease's or `net.dns`'s (`ADR-023`), and
/// `ADR-011` gives it exactly one authority.
pub const MUSL_FALLBACK_NAMESERVER: &str = "127.0.0.1";

// ---------------------------------------------------------------------------
// The identities the image itself defines
// ---------------------------------------------------------------------------

/// The conventional gid of the `tty` group, and `stage0_plan`'s devpts option.
///
/// **Not a convention borrowed for tidiness — a number this image already
/// depends on.** `crate::mount::stage0_plan`'s devpts spec carries
/// `gid=5,mode=620`, so every pty allocated on a Rivet board is already owned
/// by group 5. A group that the system uses and cannot name is the beginning of
/// two answers, so `/etc/group` names it.
///
/// `the_tty_gid_matches_the_one_devpts_is_actually_mounted_with` is the test
/// that ties the two together; changing one without the other fails the build's
/// test run rather than producing ptys owned by a group nobody can look up.
pub const TTY_GID: u32 = 5;

/// The gid of the group every git repository belongs to — `PLAN.md` §53.2's
/// ruling, in one number.
///
/// **Ruled by the maintainer 2026-08-27:** a repository is `root:git`, mode
/// `2775`, over the two alternatives §53.2 tabulated. What decided it was a
/// ruling already taken — **there is no per-repository access control**
/// (2026-08-26), so every account reaches every repository. Owning a
/// repository *as the account* is therefore correct only while there is one
/// account, and there is one today by accident rather than by design; the
/// arrangement goes wrong at the second account, silently, which is the worst
/// way for an access decision to go wrong.
///
/// **999, one below [`crate::account::ACCOUNT_UID_BASE`]**, so it cannot
/// collide with the user-private group `crate::account::identity` gives every
/// account. `the_repository_group_cannot_collide_with_an_account` asserts the
/// adjacency the way `the_account_range_cannot_reach_an_applications` asserts
/// the other end of it.
pub const REPO_GID: u32 = 999;

/// The name that gid answers to.
///
/// **`repo` rather than `git`, and the reason is a collision that already
/// exists on the board.** `/etc/group` is a flat namespace: this projection
/// writes a line for every system group *and* a user-private group for every
/// declared account, so a system group called `git` and an account called
/// `git` — which is what `10.0.0.2` runs today, and the obvious name for
/// the account a person pushes as — would render two `git:` records with
/// different gids. `getgrnam` returns the first; which one that is would be an
/// accident of the loop order in `render_group`.
///
/// `repo` also reads correctly where it is actually seen, which is `ls -l` on
/// `/state/git`: `drwxrwsr-x root repo`.
///
/// `AccountName::new` refuses every name in [`SYSTEM_GROUPS`] for the same
/// reason, so the collision is prevented at both ends rather than avoided at
/// one.
pub const REPO_GROUP: &str = "repo";

/// The gid that owns every DRM render node — `ADR-015` §6.
///
/// **A render node is shared and a card node is not**, and the difference is
/// the kernel's: DRM master is exclusive, so a card node has one holder and is
/// chowned to it, while any number of processes may render on one GPU. The
/// compositor holds the render node as the display arbiter and an application
/// holds it by declaring `gpu`, each under its own uid, so a chown to one uid
/// would take it from the other. The node is therefore `root:render 0660`
/// and the supervisor hands this gid, as a supplementary group, to exactly the
/// services it grants the node — never `0666`, which would make the
/// declaration a comment.
///
/// **998, one below [`REPO_GID`]**, so it stays under the range
/// `crate::account` keeps for accounts.
pub const RENDER_GID: u32 = 998;

/// The name [`RENDER_GID`] answers to — the name other Linux systems give the
/// same group.
pub const RENDER_GROUP: &str = "render";

/// A user account the **image** defines, as distinct from one the document does.
///
/// `ADR-008` §7.3 rules full Unix multi-user and makes users configuration:
/// "`/etc/passwd` and `/etc/group` become *generated projections* of that
/// document". That is about **human** users. `root` is not one of those — it is
/// uid 0, it is what every file in the image belongs to, and it exists whether
/// or not anybody has written a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemIdentity {
    pub name: &'static str,
    pub uid: u32,
    pub gid: u32,
    /// The fifth field. Free text; kept short because nothing reads it.
    pub gecos: &'static str,
    /// The sixth field.
    pub home: &'static str,
    /// Why this identity is the image's rather than the document's.
    pub reason: &'static str,
}

/// A group the image defines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemGroup {
    pub name: &'static str,
    pub gid: u32,
    pub reason: &'static str,
}

/// An account that exists because an **application is installed** — the other
/// half of `ADR-008` §7.3, added 2026-08-24.
///
/// `SystemIdentity` is `&'static` because the image's identities are known when
/// the image is built. This one cannot be: the set of installed applications is
/// a fact about *this machine right now*, read off `/state/app` by whoever is
/// projecting `/etc`. So it borrows, and the caller owns the list.
///
/// **`name` is the application's, and it is also the account's**, which is the
/// same one-name rule `crate::service::ServiceName` already carries: a
/// directory, a cgroup, a notify socket, and now a line in `/etc/passwd`. There
/// is deliberately no prefix — an account called `app_photo-index` would be a
/// second spelling of a name the machine already has, and `SO_PEERCRED`
/// (`ADR-015` §5.3) hands a listening service a *uid*, which it resolves back
/// through `getpwuid`. The name it gets back should be the name it declared a
/// `connects` on.
/// **It borrows the `ServiceName` itself and not the text**, so that this
/// module composes none of the application's paths: the home directory below is
/// `crate::service::data_dir`'s answer, and a `&str` here would have forced a
/// second spelling of a path that already has exactly one (`ADR-011`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApplicationAccount<'a> {
    /// The application's name — its directory under the volume's application
    /// root, and the account's name too.
    pub name: &'a crate::service::ServiceName,
    /// What `crate::service::app_identity` derived from that name.
    pub identity: crate::service::AppIdentity,
    /// **Which volume it is installed on** — the root, not the name:
    /// `crate::service::BUILT_IN_VOLUME` for `/state`, and a mount point for a
    /// declared one (`ADR-031` §8 item 4).
    ///
    /// It is here because `passwd`'s home field is `crate::service::data_dir`'s
    /// answer, and that answer is no longer a function of the name alone. The
    /// uid is: `ADR-015` §5.1.1's collision refusal is unchanged across volumes
    /// precisely because two applications of one name derive one uid wherever
    /// they sit — which is `ADR-031` §11 item 3's open question, not this
    /// field's business.
    pub volume: &'a str,
}

/// Exactly one, and the shortness is the decision.
///
/// The temptation is to pre-create service accounts — `netd`, `timed`,
/// `nobody`. **None of them is invented here**, and an account that exists for a
/// component that does not is a claim about the system that is not true
/// (`AGENTS.md` §6). Each arrives with the thing that needs it: an
/// application's identity comes from its own name at the moment it is
/// installed (`ADR-015` §5.1), which is what [`EtcInputs::applications`] carries
/// and is why no row for one can be written here.
///
/// **The struck clause stopped being true on 2026-08-24**, and the rule it was
/// an instance of is exactly why this list did not have to change: an
/// application's account arrives *with the application*, so it is an
/// [`EtcInputs::applications`] entry the running machine derives and never a
/// row here. `ADR-008` §7.3's narrowing says the same thing in the record's own
/// words — *"service accounts arrive with the services that need them and are
/// **not** pre-created"* — and this constant is what "not pre-created" looks
/// like when it is enforced rather than remembered: a compile-time list cannot
/// hold a row for software the image has never seen.
///
/// `root`'s home is `/state/home/root`, from `ADR-008` §3.1 and §7.3: home
/// directories are `/state/home`, because that is the filesystem an image update
/// never touches. It is on the partition `crate::state` mounts, and it does not
/// exist until something creates it — which is honest, because until `/state` is
/// mounted root genuinely has nowhere to keep anything.
pub const SYSTEM_IDENTITIES: &[SystemIdentity] = &[SystemIdentity {
    name: "root",
    uid: 0,
    gid: 0,
    gecos: "root",
    home: "/state/home/root",
    reason: "uid 0 owns every file in the image; getpwuid(0) must resolve",
}];

/// The groups the image defines, and every one of them is already in use.
///
/// **`git` joined the other two on 2026-08-27**, and it obeys the same rule
/// `SYSTEM_IDENTITIES` states below — *each arrives with the thing that needs
/// it*. The thing that needs this one is `[repos]`, which shipped the day
/// before: a repository has to be group-owned by something, and `PLAN.md`
/// §53.2 ruled what.
pub const SYSTEM_GROUPS: &[SystemGroup] = &[
    SystemGroup {
        name: "root",
        gid: 0,
        reason: "gid 0 owns every file in the image",
    },
    SystemGroup {
        name: "tty",
        gid: TTY_GID,
        reason: "stage0_plan mounts devpts with gid=5; the number is already load-bearing",
    },
    SystemGroup {
        name: REPO_GROUP,
        gid: REPO_GID,
        reason: "every repository is root:repo 2775, so every account that pushes is a \
                 member - PLAN.md 53.2, ruled 2026-08-27",
    },
    SystemGroup {
        name: RENDER_GROUP,
        gid: RENDER_GID,
        reason: "every DRM render node is root:render 0660, so every service granted one shares \
                 it through this group rather than taking it from another",
    },
];

// ---------------------------------------------------------------------------
// The files
// ---------------------------------------------------------------------------

pub const PASSWD: &str = "/etc/passwd";
pub const GROUP: &str = "/etc/group";
pub const SHELLS: &str = "/etc/shells";
pub const HOSTS: &str = "/etc/hosts";

/// The address this machine's own name resolves to in `/etc/hosts`.
///
/// `127.0.1.1`, Debian's convention, and see `render_hosts` for why it is not
/// `127.0.0.1`: giving the machine's name `localhost`'s own address would make
/// the two one thing, and they are not the same claim.
pub const HOSTNAME_ADDRESS: &str = "127.0.1.1";

/// The file that says, in the place musl looks, that **no account in this image
/// has a password**.
///
/// Borderline by the rule at the top of this module, and shipped anyway. Its
/// absence fails closed: `getspnam_r` finds nothing, and a password check falls
/// back to `/etc/passwd`'s `x`, which is not a valid hash and matches nothing.
/// So no wrong answer follows from leaving it out.
///
/// It ships because the *positive* statement is worth more than the inference.
/// An operator — or an agent — looking at a Rivet image and asking "can
/// anybody log in as root with a password" should find the answer written down
/// rather than have to reason about a fallback. `ADR-011` §2's second clause:
/// one authority is useless if the system will not say what it decided.
///
/// **No hash is ever written here by the image.** `ADR-008` §7.3's closing
/// warning — password hashes get the same care as the wireless PMK in `ADR-004`
/// §5 — and `AGENTS.md` §8.3's zero-scrub rule both forbid it. A hash is the
/// document's, it is `ADR-020`'s to protect, and it reaches the running machine
/// through the runtime projection, never through a signed image that anybody
/// may hold a copy of.
/// `/etc/resolv.conf` — **projected only when this machine has been told a
/// resolver**, and absent otherwise.
///
/// The absence is the design and it predates this file being rendered at all:
/// musl falls back to [`MUSL_FALLBACK_NAMESERVER`], where nothing listens, so a
/// machine with no nameserver fails fast instead of answering wrongly. A stale
/// copy baked into an image would be the bad case, and still is — this is
/// rendered onto the tmpfs, like everything else here, from the document's
/// `[network.resolver]` or, when that is silent, the DHCP lease
/// (`cmd/rivet-sysd`'s `leased_resolver`, `ADR-023`). Because a lease lands
/// after boot, `rivet-sysd` re-projects `/etc` on a reload, which the DHCP hook
/// asks for with `SIGHUP` once it has written one.
/// **Where the trust roots actually live** — on the read-only system slot, as
/// image content, versioned with the image and replaced by an A/B update.
///
/// This is the canonical path and the one anything Rivet writes should read.
/// [`CA_BUNDLE`] is a *projection* of this file for the ecosystem that cannot
/// be told where to look.
pub const CA_BUNDLE_SOURCE: &str = "/system/ssl/ca-certificates.crt";

/// **The directory the image's shared typefaces live in** — the whole of
/// `/system` any application may read.
///
/// Added 2026-09-05, and it exists because a *directory* is the unit a
/// confinement rule is written in. `rivet_core::confinement` grants this path
/// and nothing else under `/system`, so the three constants below are reachable
/// by a confined application and `/system/bin`, `/system/ssl` and
/// `/system/trust` are not. `cmd/rivet-image`'s `system::FONT_DIR` is where the
/// image puts them, and `the_font_directory_holds_every_system_face` below is
/// the guard that the two never drift — a directory that disagreed with the
/// files inside it would produce a policy granting an empty directory and three
/// faces nothing can open, which is exactly the shape of the regression this
/// constant was created to fix.
pub const SYSTEM_FONT_DIR: &str = "/system/share/fonts";

/// **The system typeface**, on the read-only system slot.
///
/// `PLAN.md` §104.38. Image content beside the trust roots and for the same
/// reason: several programs share it — the splash, the launcher, and any
/// example that draws — so it belongs to the image and is replaced by an A/B
/// update rather than installed per application.
///
/// An application that wants its OWN typeface still ships one as a package
/// asset, which is what Aura does with `IPAGothic` and `SourceCodePro`. This is
/// the one every Rivet screen falls back to, so a board looks like itself
/// before anything is installed on it.
pub const SYSTEM_FONT: &str = "/system/share/fonts/Geist.ttf";

/// **The system monospace**, beside [`SYSTEM_FONT`] and for its reasons.
///
/// Added 2026-09-04. **Geist Mono**, the same family as [`SYSTEM_FONT`] and
/// under the same licence file — one `OFL.txt` covers both, because upstream
/// ships them from one project under one copyright line.
///
/// # Why a second face is in the image at all
///
/// A monospace is not a style preference. Every screen this project draws that
/// shows a *number that changes* — a clock, a temperature, a byte count, a
/// progress figure — wants digits that do not move sideways as the value
/// changes, and a proportional face cannot give that. Drawing those in
/// [`SYSTEM_FONT`] is why a Rivet clock jitters.
///
/// # Why Geist Mono rather than the one Aura already ships
///
/// Aura's numbers are set in Source Code Pro, which is equally free to ship.
/// It was NOT promoted, for two reasons. Pairing it with Geist puts two
/// unrelated families on one screen. And Aura's sizes were chosen against
/// Source Code Pro's metrics on a wall over months, so Aura keeps its own —
/// `examples/aura/aura-renderer/assets/README.md` makes that case and it is a
/// good one. A system mono existing obliges nobody to draw with it.
pub const SYSTEM_MONO: &str = "/system/share/fonts/GeistMono.ttf";

/// The icon face, a subset of Material Icons — `PLAN.md` §104.38.
///
/// Icons as GLYPHS and not images, which is what lets any Rivet screen draw one
/// at any size with the rasteriser it already has, and means no image decoder
/// has to exist anywhere on the machine. `assets/fonts/README.md` says what is
/// in the subset.
pub const ICON_FONT: &str = "/system/share/fonts/MaterialIcons.ttf";

/// `/etc/ssl/certs/ca-certificates.crt` — the trust roots, where a C or Go
/// program will find them without being told.
///
/// # This is a TENTH path, and it is not musl's
///
/// `ADR-008` §5 justified `/etc` existing at all by musl's nine compiled-in
/// string literals. **This is not one of them.** It is Go's `crypto/x509`
/// default search list and OpenSSL's `SSL_CERT_FILE` default, each hardcoded by
/// a different library for the same purpose, and neither can be pointed
/// elsewhere without patching or an environment variable every caller has to
/// honour.
///
/// So this genuinely extends §5's rationale rather than fitting inside it, and
/// the maintainer said so when he ruled it (2026-09-01): he accepted one more
/// file in `/etc` reluctantly, as the price of being welcoming to C programs
/// (`.rulings/` R-359). Recorded here because a reader deserves the
/// disagreement rather than a rule that looks like it always covered this.
///
/// # Why a projection and not a file in the image
///
/// **A file placed at `/etc` in the image is masked at boot.** `/etc` is a
/// tmpfs mounted over whatever the read-only root has there (`ADR-007` §3.5),
/// so shipping the bundle at this path would produce a machine with the file in
/// its image, no file at runtime, and the same `x509: certificate signed by
/// unknown authority` it started with. `/etc/shells` is the precedent for the
/// shape taken instead: rendered from what this machine's own `/system` tree
/// actually holds, not from the document.
///
/// # Conditional, on the same terms as the others
///
/// An image without a bundle projects no file, rather than an empty one. An
/// empty trust store and an absent one both fail every verification, but only
/// the absent one says why — and a zero-byte `ca-certificates.crt` is a file an
/// operator would believe.
pub const CA_BUNDLE: &str = "/etc/ssl/certs/ca-certificates.crt";

pub const RESOLV_CONF: &str = "/etc/resolv.conf";

/// **Where this image's zone database lives** — `/system/share/zoneinfo`, and
/// deliberately not `/usr/share/zoneinfo`.
///
/// Added 2026-09-02 with `ADR-021` §9. `ADR-008` §3 rejects the FHS and puts
/// image content on the read-only system slot; the system typeface at
/// [`SYSTEM_FONT`] and the trust roots at [`CA_BUNDLE_SOURCE`] are the two
/// precedents and this is the third, in the same `/system/share` the first one
/// created.
///
/// **musl searches three prefixes for a NAMED zone and none of them is this**
/// — `/usr/share/zoneinfo/`, `/share/zoneinfo/` and `/etc/zoneinfo/`, read out
/// of `__tz.lo` and recorded in [`MUSL_ETC_PATHS`] and `ADR-008` §5.1. That
/// costs nothing, because **nothing on a Rivet asks musl for a zone by name**:
/// `TZ` is unset, so musl reads [`LOCALTIME`] and only [`LOCALTIME`], and the
/// database is read by *the projection*, which is Rust and knows where it put
/// it. Shipping at `/usr/share/zoneinfo` would buy `TZ=America/Chicago` working
/// for a program that set it, at the price of the first FHS directory in the
/// image; it was not worth it and the trade is written down here rather than
/// left to be rediscovered.
pub const ZONEINFO_DIR: &str = "/system/share/zoneinfo";

/// `/etc/localtime` — **the one path every runtime on this machine agrees on.**
///
/// It was in [`MUSL_ETC_PATHS`] and deliberately absent from the image until
/// 2026-09-02, on the rule at the top of this module: its absence produced
/// UTC, which is a *clean* answer for a machine nobody had told. `ADR-021` §9
/// gave the operator a way to tell it, so the file now exists when — and only
/// when — the document names a zone.
///
/// **Three runtimes, one file, and that is why it is this file and not `TZ`.**
/// musl reads this path when `TZ` is unset (`ADR-008` §5.3.1); Go's
/// `time.Local` loads `/etc/localtime` when `$TZ` is unset; chrono's `Local`
/// parses the same file. An environment variable would have to be set by
/// whatever starts each program — the supervisor for a service, the login shell
/// for a session, the launcher for an application — which is three authorities
/// for one fact and `ADR-011` says what that is.
pub const LOCALTIME: &str = "/etc/localtime";

pub const SHADOW: &str = "/etc/shadow";

/// The second field of a [`SHADOW`] record for an account with no password.
///
/// `*` rather than `!`. Both refuse every password; `!` conventionally means "a
/// hash exists and is locked", which would be false here, and `*` means "there
/// is no password and none is set in this file", which is exactly the fact.
pub const NO_PASSWORD: &str = "*";

/// Ordinary, world-readable. `/etc/passwd` and `/etc/group` are looked up by
/// unprivileged processes and have to be readable by them.
pub const ETC_FILE_MODE: u32 = 0o644;

/// `/etc/shadow` alone. Root-only, even though it holds nothing secret today.
///
/// The habit is the point: the day the runtime projection writes a real hash
/// into `/run`, the mode it is written with should already be settled, and a
/// file that was 0644 in the image is the obvious thing for that generator to
/// copy.
pub const SHADOW_MODE: u32 = 0o600;

/// One generated file, before anything has performed any I/O.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EtcFile {
    /// Absolute, and where it lands on the booted machine.
    pub path: &'static str,
    pub mode: u32,
    /// **Bytes, not text, since 2026-09-02.**
    ///
    /// Eight of the nine files here are text and were a `String` until
    /// `/etc/localtime` arrived, which is a tzfile — binary by RFC 8536, and
    /// not valid UTF-8 in general. Widening the type was the smaller change
    /// than giving one file a private path out of the projection: there is one
    /// generator (`ADR-007` §3.5) and one output list, and a second one would
    /// have to be excluded from every whole-list test that keeps the two
    /// honest. [`Self::text`] is what a reader of the other eight wants.
    pub contents: Vec<u8>,
    /// Why this file is the image's rather than the document's, in one sentence.
    pub reason: &'static str,
    /// What the contents are derived from — [`EtcDefinition::source`].
    pub source: &'static str,
    /// What an operator changes to change this file —
    /// [`EtcDefinition::changed_by`].
    pub changed_by: &'static str,
}

impl EtcFile {
    /// The contents as text, for the eight files whose format is text.
    ///
    /// **Lossy on purpose, and it is never reached for the eight.** Every
    /// renderer in this module composes a `String` and hands over its bytes, so
    /// a replacement character here would mean a renderer had produced
    /// something that was not the `String` it started as, which cannot happen.
    /// It exists so that a *test* or a diagnostic reading `/etc/passwd` does
    /// not have to handle an error that the type permits and the code does not
    /// produce — and so that reading `/etc/localtime` this way gives mojibake
    /// rather than a panic.
    #[must_use]
    pub fn text(&self) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.contents)
    }
}

/// One file's **definition**, separately from its bytes.
///
/// Split out of [`EtcFile`] 2026-08-23, for the reverse lookup `ADR-007`
/// §3.6.1 item 3 requires and `render_passwd` records as unwritten:
///
/// > A reverse lookup, so the question can be asked before the mistake rather
/// > than after it: `rivet explain /etc/resolv.conf` returns the field it is
/// > generated from, that it is read-only, and the command to change it.
///
/// Those are three separate questions, so they are three separate fields —
/// [`Self::source`], the read-only-ness of the whole tmpfs ([`ETC_FLAGS`] and
/// the remount `ADR-007` §3.5.1 describes), and [`Self::changed_by`].
///
/// **The definition carries no [`EtcInputs`]**, which is what makes the lookup
/// answerable without rendering anything: `crate::operation::explain` needs a
/// path, a mode and three sentences, and none of them depends on whether this
/// particular machine has a login shell.
///
/// **Deliberately not `PartialEq`.** It holds a function pointer, and comparing
/// two of those "does not produce meaningful results since their addresses are
/// not guaranteed to be unique" — rustc's own words, and a derived `PartialEq`
/// here is a warning rather than an opinion. [`Self::path`] is the identity
/// anything wanting to compare two definitions should compare, because it is
/// the identity `/etc` itself uses.
#[derive(Debug, Clone, Copy)]
pub struct EtcDefinition {
    /// Absolute, and where it lands on the booted machine.
    pub path: &'static str,
    pub mode: u32,
    /// What the contents are derived from, in the form `ADR-007` §3.6.1 item 2
    /// puts after "Generated by Rivet from" — so the two files that carry a
    /// provenance header take this string verbatim and the three whose format
    /// cannot carry one are still able to answer the question.
    pub source: &'static str,
    /// Why this file is rendered at all, in one sentence.
    pub reason: &'static str,
    /// What an operator changes in order to change this file.
    ///
    /// **Never "edit it"** — `ADR-007` §3.6.2 rules that a generated file is a
    /// view and never an interface. Every value here today names something
    /// other than the file, and none of them names a `rivet` verb, because no
    /// key of the configuration schema reaches any of these five
    /// (`ADR-007` §3.5.1). When one does, this is the field that has to change
    /// with it.
    pub changed_by: &'static str,
    /// Whether this machine projects the file **at all**.
    ///
    /// A field rather than a renderer returning an empty string, because those
    /// are two different facts: an empty `/etc/resolv.conf` is a file that
    /// exists and asserts no nameserver, and *no* `/etc/resolv.conf` is the
    /// state a machine with none configured is actually in. A generated view
    /// that exists but is empty is also a view an operator can misread as
    /// "the setting is broken" rather than "nothing was asked for".
    ///
    /// Every file that existed before conditional files did answers `true`
    /// unconditionally
    /// — and says so as `|_| true` rather than by defaulting, because
    /// [`ETC_FILES`] is the list a reader checks this against.
    pub projects: fn(&EtcInputs<'_>) -> bool,
    /// How the bytes are produced. A function pointer rather than a `match` on
    /// [`Self::path`] so that a definition **cannot** be added without a
    /// renderer: the alternative is a fallthrough arm, and the only honest
    /// fallthrough is a panic in a crate where `panic` is a denied lint.
    ///
    /// **`Vec<u8>` since 2026-09-02** — see [`EtcFile::contents`]. Eight of
    /// the nine renderers still compose a `String` and hand over its bytes;
    /// `/etc/localtime` is a tzfile and has none to compose.
    pub render: fn(&Self, &EtcInputs<'_>) -> Vec<u8>,
}

/// What the renderer needs that is not a constant in this module.
///
/// ~~One field, deliberately, and the shortness is load-bearing: **everything
/// else these files contain is either a constant here or a document key that
/// does not exist yet.**~~ **Two fields since 2026-08-24**, and the second one
/// is neither of those things: the applications installed on this machine are
/// not a constant and are not a document key — they are a directory listing.
/// See [`etc_projection`] for the list of facts still blocked on a schema key,
/// which is where a *third* field belongs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EtcInputs<'a> {
    /// The binaries this machine permits as login shells, **in
    /// [`LOGIN_SHELL_PATHS`] order, filtered to those that are actually
    /// there**.
    ///
    /// A login is rejected before any credential is looked
    /// at unless the account's shell is listed in `/etc/shells`, and the fix is
    /// "to list Rivet's own CLI in a generated `/etc/shells`".
    ///
    /// **Empty by default, and that is not a placeholder.** Writing a path
    /// here for a binary that is not there would produce an `/etc/shells`
    /// naming a file nothing can exec — which reads as working and is not
    /// (`AGENTS.md` §6). So the caller **derives this by looking**, and an
    /// entry appears on the day its binary ships and not a day before:
    /// `cmd/rivet-sysd`'s `etc` module asks the filesystem about each
    /// of [`LOGIN_SHELL_PATHS`] in turn.
    ///
    /// **That day was 2026-08-23 for [`CLI_PATH`].** `cmd/rivet` exists and
    /// `cmd/rivet-image` installs it there, so the look now succeeds on a
    /// real card and `/etc/shells` names a shell that is present. Two things
    /// had to be true at once and neither implies the other: the binary must be
    /// in the image, which is the image builder's claim, and it must be found
    /// at runtime, which is this field's.
    ///
    /// **It became a list on 2026-08-24**. The first entry
    /// is `root`'s shell in `/etc/passwd`; every entry is named in
    /// `/etc/shells`. A one-element list therefore renders byte-for-byte what
    /// the previous `Option<&str>` rendered, which is why that ruling cost no
    /// change on a card built without `nu`.
    ///
    /// **The default stays empty**, because `Default` here means "an image
    /// that ships no login shell" — a state a caller may legitimately describe,
    /// and not a claim about any particular build.
    pub login_shells: &'a [&'a str],
    /// The applications installed on **this machine**, each with the identity
    /// `crate::service::app_identity` derived from its name. Added 2026-08-24.
    ///
    /// `ADR-015` §5.1's first sandbox mechanism is *"UID/GID per
    /// application"*, and a uid that no name resolves to is a machine where
    /// `ls -l` on the application's own data prints a number, where
    /// `SO_PEERCRED` (§5.3) gives a listening service an unnameable caller, and
    /// where an operator reading `ps` cannot tell one application from another.
    /// So the accounts are projected, and this is how they arrive.
    ///
    /// **Empty by default, and — as with [`Self::login_shells`] — the caller
    /// derives it by looking rather than by asserting.** A machine with nothing
    /// installed renders exactly the `/etc` it rendered before this field
    /// existed, byte for byte.
    ///
    /// **The caller must have already refused any collision.** Two names can
    /// derive one uid (`crate::service::app_identity` argues why, and
    /// `crate::service::identities_collide` is the predicate); two rows here
    /// with one uid would put two answers into `getpwuid`, which is `ADR-011`
    /// exactly. The refusal belongs where every definition is held at once —
    /// `cmd/rivet-sysd`'s `applications` module — because this
    /// function renders and does not decide.
    pub applications: &'a [ApplicationAccount<'a>],

    /// What this machine calls itself — `crate::hostname`, ruled 2026-08-26.
    ///
    /// **The first field the configuration document reaches**, and the one
    /// this module's own table has been waiting for since 2026-08-23: *"the
    /// machine's hostname → `/etc/hosts` → blocked on no `hostname` key"*.
    /// Until this existed, [`etc_projection`]'s documentation could say the
    /// document contributed nothing to `/etc`, and `cmd/rivet-sysd`'s
    /// `the_document_contributes_nothing_and_a_new_key_must_decide_whether_it_does`
    /// was the guard that would fail the day a schema key arrived. It arrived,
    /// the guard fired, and this is the answer it demanded.
    ///
    /// `None` is a machine that has no name — either the kernel default
    /// `(none)`, or a machine that could not distinguish itself well enough to
    /// derive one (`crate::hostname::derived`). It renders **no line** rather
    /// than a line naming nothing, for [`EtcDefinition::projects`]'s reason
    /// exactly: an entry that resolved `(none)` would be a wrong answer where
    /// an absent one is a clean failure.
    pub hostname: Option<&'a crate::hostname::Hostname>,

    /// The nameservers this machine asks, in the order it asks them.
    ///
    /// `[network.resolver]` in the document (`rivet_config::resolver`, ruled
    /// 2026-09-01) **when the document names one**, else the DHCP lease
    /// (`cmd/rivet-sysd`'s `leased_resolver`, the lease half of `ADR-023`, built
    /// 2026-09-23). The caller computes which — one authority per `ADR-011`, and
    /// the document wins whenever it speaks. **Empty means neither had an
    /// answer**, which projects no `/etc/resolv.conf` at all rather than an
    /// empty one (see [`RESOLV_CONF`]): musl then fails a lookup fast against
    /// its own `127.0.0.1` fallback.
    pub nameservers: &'a [core::net::Ipv4Addr],

    /// The search list appended to an unqualified name, in order. Empty is the
    /// ordinary case and renders no `search` line.
    pub search: &'a [&'a str],

    /// The trust roots this machine ships, read from [`CA_BUNDLE_SOURCE`].
    ///
    /// **Derived by looking, like `login_shells`** — the caller reads the file
    /// off `/system` and the projection renders what was actually there. A
    /// constant here would produce an `/etc/ssl/certs/ca-certificates.crt` on
    /// an image that carries no bundle, which is the failure `AGENTS.md` §6
    /// calls reading as working and not being.
    pub ca_bundle: Option<&'a str>,

    /// What local time this machine keeps — the document's `timezone` key
    /// (`crate::timezone`), ruled 2026-09-02.
    ///
    /// **The fifth field the document reaches, and the last row of this
    /// module's own blocked-on-a-key table.** That row has read *"what timezone
    /// this machine keeps → the document (`ADR-021`) → no schema key yet"*
    /// since 2026-08-23.
    ///
    /// **Derived by looking, like [`Self::ca_bundle`] and for its reason.** The
    /// bytes are a tzfile off the read-only system slot, so the composition
    /// root reads [`ZONEINFO_DIR`] and hands over what was actually there.
    /// Composing them here is impossible — this crate performs no I/O — and
    /// asserting them would produce an `/etc/localtime` for a zone the image
    /// has no file for, which is the "reads as working and is not" failure
    /// `AGENTS.md` §6 names.
    ///
    /// `None` is a machine keeping UTC, and it covers three different states
    /// that are one state to this projection: the document named no zone, the
    /// document named a zone this image has no file for, and the file was
    /// unreadable. **They are one state HERE and three records in the journal**
    /// — the composition root is what can tell them apart, and it says so
    /// there, because a machine silently on UTC under a document that says
    /// `America/Chicago` is exactly `ADR-034` §5.3's applied-when-it-is-not.
    pub local_time: Option<LocalTime<'a>>,
}

/// The zone this machine keeps, and the bytes that define it.
///
/// **Two fields for two different readers**, which is why this is not just a
/// `&[u8]`: the tzfile is what `/etc/localtime` holds, and the *name* is what
/// the journal and `rivet explain` need in order to say which zone a machine is
/// on — a tzfile carries no identifier of its own, which is the one thing a
/// copy loses against a symlink.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalTime<'a> {
    /// The IANA identifier the document named — `crate::timezone::Timezone`
    /// as a string, because this crate's `/etc` half has no reason to depend
    /// on the validated type and every reader of this field is printing it.
    pub zone: &'a str,
    /// The tzfile, verbatim, as read off [`ZONEINFO_DIR`].
    ///
    /// **Verbatim and not re-encoded.** `AGENTS.md` §5: what reaches
    /// `/etc/localtime` is what the image shipped, so a machine's zone rules
    /// are the database's and nothing here can have altered them.
    pub tzfile: &'a [u8],
}

/// The whole of `/etc`, sorted by path — **the one generator's output.**
///
/// Ruled 2026-08-23 (`ADR-007` §3.5): there is one generator, it is the runtime
/// projection, and it renders all of `/etc`. `cmd/rivet-sysd` performs
/// this onto the tmpfs described by [`ETC_MOUNT_POINT`] and its neighbours.
///
/// Pure: same inputs, same bytes, no clock, no map iteration order, no
/// randomness. That is not left over from the withdrawn build-time renderer —
/// it is what makes the projection *convergent*, so a supervisor restart
/// rewrites identical bytes rather than churning a file something is reading.
///
/// # What the configuration document contributes today
///
/// Two of `Document::DECLARED_KEYS` reach this module: `hostname` reaches
/// `/etc/hosts` through [`EtcInputs::hostname`], and `timezone` reaches
/// `/etc/localtime` through [`EtcInputs::local_time`]. `rivet-config` cannot
/// be named here — `ADR-014` §3.2 made the dependency run the other way,
/// permanently — so the `Document` is read at the composition root and what
/// it yields arrives as [`EtcInputs`].
///
/// One document-owned fact is still blocked on something other than a key:
///
/// | Fact | Would reach | Blocked on |
/// | :-- | :-- | :-- |
/// | **human** accounts and password hashes | `passwd`, `group`, `shadow` | `ADR-008` §7.3 puts multi-user at Stage 4, and there is no authorization layer for a human user to attach to |
///
/// Application accounts reach `passwd`, `group` and `shadow` **today**,
/// through [`EtcInputs::applications`] — they are not the document's, which
/// is `ADR-008` §7.3's own distinction: *"service accounts arrive with the
/// services that need them"*.
///
/// `cmd/rivet-sysd`'s
/// `the_document_contributes_nothing_and_a_new_key_must_decide_whether_it_does`
/// is the guard that fails the day the schema grows a key nobody has routed
/// here yet.
pub fn etc_projection(inputs: &EtcInputs<'_>) -> Vec<EtcFile> {
    render_all(inputs)
}

/// The projection under the name `cmd/rivet-image` still imports it by.
///
/// **A compatibility alias, and it renders nothing onto a card.** The image
/// generated the then-five files for part of 2026-08-23 and the maintainer
/// withdrew it the same day (`ADR-007` §3.5); what remains in that crate is a
/// *test* — `core_still_defines_etc_even_though_the_image_no_longer_renders_it`
/// — which asserts that this definition survived the withdrawal and that none
/// of its output reached the tree.
///
/// It is kept rather than renamed because the two things a rename would touch
/// are both in that test, and a name is cheaper to leave than a build to break.
/// Delete it when that crate switches to [`etc_projection`].
pub fn image_etc(inputs: &EtcInputs<'_>) -> Vec<EtcFile> {
    render_all(inputs)
}

/// **The whole of `/etc`, as definitions** — sorted by path, and the one list.
///
/// [`etc_projection`] renders these; `crate::operation::explain` reads them
/// without rendering anything. Two readers of one list, which is `ADR-011` §2's
/// first clause; the second clause — *the system says which one* — is what
/// `explain` is for.
pub const ETC_FILES: &[EtcDefinition] = &[
    EtcDefinition {
        path: CA_BUNDLE,
        mode: ETC_FILE_MODE,
        source: "the trust roots this image ships, at /system/ssl/ca-certificates.crt",
        reason: "Go's crypto/x509 and OpenSSL each hardcode this path, and without it every \
                 TLS verification on the machine fails with `certificate signed by unknown \
                 authority` — which is what a Rivet did until 2026-09-01",
        changed_by: "the image: the bundle is content on the read-only system slot and an A/B \
                     update replaces it. Nothing on a running machine can change what this \
                     machine trusts",
        // Conditional on the image actually carrying one — see `CA_BUNDLE`.
        projects: |inputs| inputs.ca_bundle.is_some(),
        render: |_, inputs| render_ca_bundle(inputs).into_bytes(),
    },
    EtcDefinition {
        path: GROUP,
        mode: ETC_FILE_MODE,
        source: "the image's own group table, and the applications installed on this machine",
        reason: "gid 5 is already load-bearing: stage0_plan mounts devpts with gid=5, and \
                 every installed application runs under a group of its own (ADR-015 §5.1)",
        changed_by: "SYSTEM_GROUPS in rivet-core, which means building an image; and \
                     installing or removing an application, which does not",
        // Unconditional — nothing about a machine decides whether it is
        // projected.
        projects: |_| true,
        render: |_, inputs| render_group(inputs).into_bytes(),
    },
    EtcDefinition {
        path: HOSTS,
        mode: ETC_FILE_MODE,
        // Verbatim what `header` was called with before this table existed, so
        // that splitting the definition out did not change one byte of /etc.
        source: "the meaning of the loopback interface",
        reason: "absent, localhost is resolved over DNS — musl has no builtin",
        changed_by: "nothing: these two names are what the loopback interface means, and \
                     this machine's own hostname is not here (ADR-025)",
        // Unconditional — nothing about a machine decides whether it is
        // projected.
        projects: |_| true,
        render: |def, inputs| render_hosts(def, inputs).into_bytes(),
    },
    EtcDefinition {
        path: LOCALTIME,
        mode: ETC_FILE_MODE,
        source: "the `timezone` key of the configuration document, resolved against the zone \
                 database this image ships at /system/share/zoneinfo",
        reason: "musl, Go and chrono all read this exact path when TZ is unset, and without \
                 it every one of them keeps UTC SILENTLY - a machine whose document names a \
                 zone and whose clock is an hour out with nothing in any log (ADR-021 §9)",
        changed_by: "the `timezone` key of the configuration document - remove it and this \
                     file is not projected at all, which is the machine keeping UTC. WHICH \
                     ZONES EXIST is the image's: a name this image has no file for projects \
                     nothing and is reported by the supervisor",
        // **The fourth conditional file**, and the first whose condition is
        // *this machine could actually read the bytes*. An absent file is a
        // machine on UTC, which is a state; a present one whose contents were
        // invented would be a machine on a zone nobody shipped.
        projects: |inputs| inputs.local_time.is_some(),
        // The one renderer that composes nothing. `AGENTS.md` §5 — the bytes
        // are the database's and are copied rather than reproduced.
        render: |_, inputs| {
            inputs
                .local_time
                .map(|local| local.tzfile.to_vec())
                .unwrap_or_default()
        },
    },
    EtcDefinition {
        path: PASSWD,
        mode: ETC_FILE_MODE,
        source: "the image's own identity table, and the applications installed on this machine",
        reason: "uid 0 owns every file in the image and getpwnam(\"root\") must answer; and an \
                 application's own uid must have a name, or ls prints a number and SO_PEERCRED \
                 gives a listening service an unnameable caller (ADR-015 §5.1, §5.3)",
        changed_by: "SYSTEM_IDENTITIES in rivet-core, which means building an image; and \
                     installing or removing an application, which does not",
        // Unconditional — nothing about a machine decides whether it is
        // projected.
        projects: |_| true,
        render: |_, inputs| render_passwd(inputs).into_bytes(),
    },
    EtcDefinition {
        path: RESOLV_CONF,
        mode: ETC_FILE_MODE,
        source: "the `[network.resolver]` table of the configuration document",
        reason: "musl has no resolver configuration API and reads this path as a compiled-in \
                 string literal (ADR-008 §5); without it every name lookup goes to 127.0.0.1, \
                 where nothing listens",
        changed_by: "the `[network.resolver]` table of the configuration document — remove it \
                     and this file is not projected at all, which is not the same as an empty \
                     one",
        // **The second conditional file**, and the first that is
        // conditional on something the operator writes rather than on a mode.
        // No nameservers and no search list is *project nothing*: absent, musl
        // fails fast against its own fallback, and a present-but-empty file
        // would be a machine claiming to have been configured.
        projects: |inputs| !inputs.nameservers.is_empty() || !inputs.search.is_empty(),
        render: |def, inputs| render_resolv_conf(def, inputs).into_bytes(),
    },
    EtcDefinition {
        path: SHADOW,
        mode: SHADOW_MODE,
        source: "every account this machine has, none of which has a password",
        reason: "says positively that no account on this machine has a password",
        changed_by: "nothing yet: a password hash is the document's and ADR-008 §7.3 puts \
                     human accounts at Stage 4. An application account never gets one",
        // Unconditional — nothing about a machine decides whether it is
        // projected.
        projects: |_| true,
        render: |_, inputs| render_shadow(inputs).into_bytes(),
    },
    EtcDefinition {
        path: SHELLS,
        mode: ETC_FILE_MODE,
        source: "this machine's own /system tree",
        reason: "absent, musl substitutes /bin/sh and /bin/csh",
        changed_by: "whether a binary is present at /system/bin/rivet, which the \
                     projection probes for; changing it means building an image",
        // Unconditional — nothing about a machine decides whether it is
        // projected.
        projects: |_| true,
        render: |def, inputs| render_shells(def, inputs).into_bytes(),
    },
];

fn render_all(inputs: &EtcInputs<'_>) -> Vec<EtcFile> {
    ETC_FILES
        .iter()
        // A file this machine does not project is absent from
        // the output, not present and empty. See `EtcDefinition::projects`.
        .filter(|def| (def.projects)(inputs))
        .map(|def| EtcFile {
            path: def.path,
            mode: def.mode,
            contents: (def.render)(def, inputs),
            reason: def.reason,
            source: def.source,
            changed_by: def.changed_by,
        })
        .collect()
}

/// The header `ADR-007` §3.6.1 item 2 calls the highest-value honesty mechanism
/// in its list, "because it is read by exactly the person about to make the
/// mistake: they have the file open".
///
/// Two things are said that §3.6.1's example does not have to say, because its
/// example is a file generated from a configuration field and these are not:
/// that the source is **the image**, and that changing it therefore means
/// building one.
///
/// **Only two of the six files carry it**, and the reason is a format
/// limitation rather than a preference — see [`render_passwd`].
///
/// **Amended 2026-08-23 with the withdrawal.** The middle line used to read
/// *"/system is read-only (ADR-018 §3); changing this means building an image"*,
/// which was true of a file baked into the erofs root and is false of one on a
/// tmpfs. It would have sent an operator to rebuild an image to change a file
/// the running system rewrites on every boot — a provenance header that names
/// the wrong source is worse than none, because §3.6.1 item 2's whole argument
/// is that it is "read by exactly the person about to make the mistake".
fn header(what_owns_it: &str) -> String {
    format!(
        "# Generated by Rivet from {what_owns_it}. Do not edit.\n\
         # /etc is a read-only tmpfs, rendered afresh on every boot (ADR-007 §3.5).\n\
         # This file is a view, never an interface (ADR-007 §3.6.2).\n"
    )
}


/// `/etc/passwd`, and **no provenance header**.
///
/// The colon-separated databases — `passwd`, `group`, `shadow` — have no comment
/// syntax that musl's parsers are known to honour, and a header line that a
/// parser handed back as a *user record* would be considerably worse than no
/// header. `AGENTS.md` §5: refuse rather than guess. The reverse lookup of
/// `ADR-007` §3.6.1 item 3 — `rivet explain /etc/passwd` — is where the
/// provenance for these three belongs, and ~~it is unwritten~~ **it is written,
/// 2026-08-23**: [`crate::operation::explain`] answers it from [`ETC_FILES`], so
/// the three colon databases carry their provenance in the one place their
/// format allows it to be read.
///
/// Seven fields, colon-separated: name, password, uid, gid, gecos, home, shell.
/// The second is `x`, meaning "the hash is in `/etc/shadow`", which is true and
/// which is where [`render_shadow`] refuses to put one.
///
/// **The seventh field takes the *first* of [`EtcInputs::login_shells`]**, and
/// the file has room for exactly one — which is the asymmetry the dual-login-shell ruling
/// had to resolve and the reason [`LOGIN_SHELL_PATHS`] is ordered. An empty
/// field is what an image with no login shell renders, and it is honest: musl
/// hands `getpwnam` an empty shell, no `/etc/shells` entry matches it, and the
/// login is refused rather than exec'ing something that is not there.
/// # The application accounts, and their **empty seventh field**
///
/// Added 2026-08-24. Every installed application gets a record, after the
/// image's own and in the order the caller supplies — see
/// [`EtcInputs::applications`] for why they cannot be constants.
///
/// Their shell is the **empty string**, and it is the one field worth arguing.
/// `root`'s shell is this machine's login shell; an application's is nothing,
/// because an application account is an *identity*, not a way in. musl hands
/// `getpwnam("photo-index")` an empty shell, and anything that then checks it
/// against `/etc/shells` finds no match and refuses —
/// which is exactly the outcome wanted, reached by a mechanism that is already
/// there rather than by a new one.
///
/// Their home is `crate::service::data_dir` — the one directory the
/// application may write — so a shell that somehow started there would start
/// somewhere the identity actually owns rather than in `/`.
fn render_passwd(inputs: &EtcInputs<'_>) -> String {
    let shell = inputs.login_shells.first().copied().unwrap_or("");
    let mut out = String::new();
    for id in SYSTEM_IDENTITIES {
        // `write!` to a String cannot fail; the result is discarded rather than
        // unwrapped because `unwrap`/`expect` are warned across the workspace.
        let _ = writeln!(
            out,
            "{}:x:{}:{}:{}:{}:{}",
            id.name, id.uid, id.gid, id.gecos, id.home, shell
        );
    }
    for app in inputs.applications {
        let _ = writeln!(
            out,
            "{}:x:{}:{}:{}:{}:",
            app.name,
            app.identity.uid,
            app.identity.gid,
            APPLICATION_GECOS,
            crate::service::data_dir(app.volume, app.name),
        );
    }
    out
}

/// The fifth field of an application's `/etc/passwd` record.
///
/// Free text that nothing parses, kept short and kept **general**: it says what
/// kind of account this is, and deliberately not which application, a version,
/// or anything else that would go stale against the artifact it describes. The
/// name is already field one.
pub const APPLICATION_GECOS: &str = "Rivet application";

/// `/etc/group`. Four fields: name, password, gid, member list.
///
/// The member list is empty for two of the three system groups. `root` needs no
/// members — its membership comes from `/etc/passwd`'s primary gid — and nothing
/// is a member of `tty` either; devpts assigns the group to the pty, not to a
/// user.
///
/// **[`REPO_GROUP`] has no members either, and that is the state rather than an
/// omission.** `PLAN.md` §53.2 rules that repositories are `root:git` mode
/// `2775`, so the group is what *owns* them — and nothing on this machine has an
/// identity that would be added here. The group is written because the
/// directories are group-owned by it; a member list is a separate fact and
/// there is nothing true to put in it.
///
/// **And it is empty for every application group too**, added 2026-08-24, for
/// a third reason that is the whole point of them: an application's group has
/// exactly one member and that member is the application itself, through
/// `/etc/passwd`'s primary gid. A name in this field would be some *other*
/// identity that may read that application's files, and nothing is entitled to
/// put one there — the supervisor sets the supplementary group list to **empty**
/// before it sets the gid, so a membership written here would be a claim the
/// running machine contradicts.
fn render_group(inputs: &EtcInputs<'_>) -> String {
    let mut out = String::new();
    for g in SYSTEM_GROUPS {
        let _ = writeln!(out, "{}:x:{}:", g.name, g.gid);
    }
    for app in inputs.applications {
        let _ = writeln!(out, "{}:x:{}:", app.name, app.identity.gid);
    }
    out
}


/// `/etc/shadow`. Nine fields, of which this fills two.
///
/// name, password, last-change, min, max, warn, inactive, expire, reserved. The
/// seven empty trailing fields mean "no ageing policy", which is the honest
/// value: Rivet has no password ageing and inventing dates would put a clock
/// reading into a byte-reproducible artifact.
///
/// **Application accounts get a record too**, added 2026-08-24, and it is the
/// same positive statement [`SHADOW`] argues for: an application identity has
/// no password and can never be authenticated as. Leaving them out would fail
/// closed by the same fallback, and would leave the reader to infer it — which
/// is the inference this file exists to replace.
/// `/etc/resolv.conf` — one `nameserver` line each, then one `search` line.
///
/// # The order is the document's and is not sorted
///
/// musl asks the nameservers in the order the file lists them, so sorting would
/// change which server answers first — a behavioural change disguised as
/// tidiness. `ADR-007` §3.1's identical-state-identical-bytes rule is satisfied
/// because the *document's* order is canonical, not because this file imposes
/// one.
///
/// # Nothing here escapes anything, and nothing needs to
///
/// The format is whitespace-separated with no quoting, which is exactly why
/// `rivet_config::resolver::SearchDomain` refuses a value containing
/// whitespace: a domain with a newline in it would not be a bad domain, it
/// would be a second directive. The validation is at the parser (`ADR-011`),
/// so by the time a value reaches this function it cannot carry one.
/// The trust roots, **verbatim and with no provenance header**.
///
/// Every other rendered text file carries [`header`]. This one must not: it is
/// parsed by OpenSSL and Go as a sequence of PEM blocks, and while both skip
/// text outside `BEGIN CERTIFICATE`, "both implementations we know of tolerate
/// it" is not the same as "the format allows it". `render_passwd` refuses a
/// header for the same reason and says so at greater length.
///
/// The provenance is answerable anyway: `rivet explain` reads it from
/// [`ETC_FILES`], which is where the colon databases' provenance already lives.
fn render_ca_bundle(inputs: &EtcInputs<'_>) -> String {
    inputs.ca_bundle.unwrap_or_default().to_owned()
}

fn render_resolv_conf(def: &EtcDefinition, inputs: &EtcInputs<'_>) -> String {
    let mut out = header(def.source);
    for address in inputs.nameservers {
        out.push_str(&format!("nameserver {address}\n"));
    }
    if !inputs.search.is_empty() {
        out.push_str(&format!("search {}\n", inputs.search.join(" ")));
    }
    out
}

fn render_shadow(inputs: &EtcInputs<'_>) -> String {
    let mut out = String::new();
    for id in SYSTEM_IDENTITIES {
        let _ = writeln!(out, "{}:{}:::::::", id.name, NO_PASSWORD);
    }
    for app in inputs.applications {
        let _ = writeln!(out, "{}:{}:::::::", app.name, NO_PASSWORD);
    }
    out
}

/// `/etc/shells`, which is the file measurement on real hardware made mandatory.
///
/// Carries the header, with one wrinkle worth writing down: **musl's
/// `getusershell()` does not skip comments.** The object imports `fopen`,
/// `fmemopen`, `getline` and `fclose` and nothing that could parse one, so it
/// returns every line verbatim — including these three. That is harmless,
/// because no account's shell is ever the string `# Generated by Rivet…` and
/// the check is a string comparison. It is recorded so that nobody later
/// concludes the header is being honoured and puts something load-bearing in it.
///
/// **An empty list is a real answer, not a stub.** A present-and-empty
/// `/etc/shells` says "this machine has no login shell", and every login is
/// rejected. An *absent* one says "`/bin/sh` and `/bin/csh` are the login
/// shells" ([`MUSL_DEFSHELLS`]), which is a claim Rivet has never made — and
/// until 2026-08-23 an absent one is exactly what a running Rivet had, because
/// nothing rendered `/etc` at all.
///
/// **Every present shell is named, not just `root`'s.**
/// `getusershell()` is a membership test, so a shell that is missing here is
/// a shell no account may hold, whatever `/etc/passwd` says. Listing
/// [`CLI_PATH`] alongside [`NUSHELL_PATH`] is what keeps a future restricted
/// account viable without a second ruling.
fn render_shells(def: &EtcDefinition, inputs: &EtcInputs<'_>) -> String {
    let mut out = header(def.source);
    if inputs.login_shells.is_empty() {
        out.push_str(
            "#\n\
             # This machine has no login shell, so every login is rejected before any\n\
             # credential is looked at. That is deliberate and is\n\
             # not the same as this file being missing: missing would let musl claim\n\
             # /bin/sh and /bin/csh are login shells here, and neither exists.\n",
        );
    }
    for path in inputs.login_shells {
        let _ = writeln!(out, "{path}");
    }
    out
}

/// `/etc/hosts` — the loopback names, and this machine's own.
///
/// ~~The machine's own hostname is **not** here.~~ **It is, since 2026-08-26**,
/// and the paragraph below was never an argument against it — it was an
/// argument about *which authority supplies it*, and that authority now exists:
///
/// > That is configuration — the document's, per `ADR-007` — and it reaches
/// > `/etc/hosts` through the runtime projection. Baking a hostname into the
/// > image would give every board built from it the same name and would make
/// > the image, rather than the document, the authority for a machine's
/// > identity (`ADR-025`).
///
/// Every clause of that still holds. What changed is that the document grew a
/// `hostname` key, so the projection has something to project — this row of
/// this module's own table has said *"blocked on no `hostname` key"* since
/// 2026-08-23.
///
/// **Why the machine resolving its own name matters at all**, since it is easy
/// to read as decoration: without this line `gethostbyname(<our own name>)`
/// goes to DNS, and a Rivet ships no `/etc/resolv.conf` (§5.3), so musl falls
/// back to `127.0.0.1` where nothing listens. A program that looks itself up —
/// which is most things that bind a socket by name — would hang and then fail.
///
/// **It is `127.0.1.1` and not `127.0.0.1`**, which is Debian's convention and
/// is right for the same reason it is right there: the name must resolve
/// without the network, and giving it `localhost`'s own address would make
/// `localhost` and the machine's name one thing — so a service that bound to
/// its own name would answer for `localhost` too, and the two are not the same
/// claim.
///
/// The other two lines are not configuration in any sense: they are what the
/// loopback interface means. musl supplies no built-in for them
/// ([`MUSL_HAS_NO_BUILTIN_LOCALHOST`]).
///
/// Deliberately without the `ip6-localhost` and `ip6-loopback` aliases that
/// Debian ships. Nothing in Rivet uses them and `ADR-009` §4 is the general
/// form of the objection: a name inherited from another system's habits is a
/// thing to be maintained, and this one buys nothing.
fn render_hosts(def: &EtcDefinition, inputs: &EtcInputs<'_>) -> String {
    let mut out = header(def.source);
    out.push_str("127.0.0.1\tlocalhost\n");
    out.push_str("::1\tlocalhost\n");
    if let Some(hostname) = inputs.hostname {
        out.push_str(&format!("{HOSTNAME_ADDRESS}\t{hostname}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::mount::stage0_plan;

    /// **The confinement grants a DIRECTORY and the image writes FILES**, so
    /// the two have to be the same directory or the grant reaches nothing.
    ///
    /// `ADR-011`: one authority for one fact. [`SYSTEM_FONT_DIR`] is it, and
    /// this is the assertion that a face added later does not quietly land
    /// outside the only part of `/system` an application may read
    /// (`ADR-042` §4.2). A fourth face put in `/system/fonts` would compile,
    /// package, install, and be refused with `EACCES` on a board — which is
    /// precisely how this regression reached hardware the first time.
    #[test]
    fn the_font_directory_holds_every_system_face() {
        for face in [SYSTEM_FONT, SYSTEM_MONO, ICON_FONT] {
            assert!(
                face.starts_with(&format!("{SYSTEM_FONT_DIR}/")),
                "{face} is not inside {SYSTEM_FONT_DIR}, so rivet_core::confinement's grant does \
                 not reach it and every confined application that draws is refused it"
            );
            assert!(
                !face[SYSTEM_FONT_DIR.len() + 1..].contains('/'),
                "{face} is in a SUBDIRECTORY of {SYSTEM_FONT_DIR}; the grant reaches beneath the \
                 directory so that is still allowed, but cmd/rivet-image writes faces flat and a \
                 nested one would not be installed at all"
            );
        }
    }

    fn stage0() -> Vec<EtcFile> {
        etc_projection(&EtcInputs::default())
    }

    /// A card built without `nu`: the CLI is the only login shell present.
    ///
    /// This is the fixture every pre-2026-08-24 test used, kept under its own
    /// name so that the dual-login-shell ruling's promise — that such a card renders
    /// exactly what it always did — is asserted rather than assumed.
    fn with_shell() -> Vec<EtcFile> {
        etc_projection(&EtcInputs {
            login_shells: &[CLI_PATH],
            ..EtcInputs::default()
        })
    }

    /// A card built with `nu`, which is what `LOGIN_SHELL_PATHS` describes when
    /// both binaries are installed.
    fn with_both() -> Vec<EtcFile> {
        etc_projection(&EtcInputs {
            login_shells: LOGIN_SHELL_PATHS,
            ..EtcInputs::default()
        })
    }

    fn find<'a>(files: &'a [EtcFile], path: &str) -> Option<&'a EtcFile> {
        files.iter().find(|f| f.path == path)
    }

    fn body(files: &[EtcFile], path: &str) -> String {
        find(files, path)
            .map(|f| f.text().into_owned())
            .unwrap_or_default()
    }

    /// Lines that are neither blank nor comments — the records a parser sees.
    fn records(text: &str) -> Vec<&str> {
        text.lines()
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect()
    }

    // -- which files exist, and which deliberately do not ---------------------

    /// Whole-list, on the same reasoning as `stage0_plan_is_these_seven_in_this_order`:
    /// adding a file to `/etc` should require changing a test that says what
    /// `/etc` is.
    ///
    /// Renamed 2026-08-23 from `the_image_ships_these_five_etc_files_and_no_others`.
    /// The image ships none of them — it ships the empty mount point — and a
    /// test name asserting otherwise is exactly the stale claim `AGENTS.md` §7
    /// is about. A count once lived in this name too, for the files that are
    /// unconditional; it was removed for the same reason.
    ///
    /// **`tools/qemu/smoke.sh` scrapes this list**, by name, to decide how many
    /// `rendered` records a booted machine owes it — which is the argument for
    /// the scrape naming a test rather than carrying a number.
    #[test]
    fn the_projection_renders_these_etc_files_and_no_others() {
        let files = etc_projection(&EtcInputs::default());
        let paths: Vec<_> = files.iter().map(|f| f.path).collect();
        assert_eq!(
            paths,
            [
                "/etc/group",
                "/etc/hosts",
                "/etc/passwd",
                "/etc/shadow",
                "/etc/shells",
            ]
        );
    }

    /// The other half of the rule, and the one that stops it being satisfied by
    /// rendering everything. Each of these three is absent for a *reason*, and
    /// the reason is that musl's fallback is correct.
    ///
    /// **Note what changed and what did not on 2026-08-23.** These three are
    /// still absent, and they are absent from the *only* generator there is now
    /// — so `/etc/resolv.conf` is not present-and-stale, it simply is not there,
    /// and musl's `127.0.0.1` fallback fails cleanly ([`MUSL_FALLBACK_NAMESERVER`]).
    /// Each becomes a file on the day the mechanism that owns it exists.
    #[test]
    fn the_three_mechanism_owned_paths_are_not_rendered() {
        let files = stage0();
        for path in ["/etc/resolv.conf", "/etc/localtime", "/etc/services"] {
            assert!(
                find(&files, path).is_none(),
                "{path} belongs to a mechanism that does not exist yet; rendering a \
                 placeholder would be a confident wrong answer where musl's fallback \
                 is a clean failure"
            );
        }
    }

    /// Every file rendered is one musl actually reads. A file at a path nothing
    /// looks at would be decoration.
    #[test]
    fn every_rendered_file_is_a_path_musl_compiles_in() {
        for f in stage0() {
            assert!(
                MUSL_ETC_PATHS.contains(&f.path),
                "{} is rendered but is not a path musl reads — ADR-008 §5.1",
                f.path
            );
        }
    }

    /// `ADR-008` §5.1's eight, plus the ninth this module's own sweep found.
    #[test]
    fn the_musl_path_list_is_adr_008_5_1s_eight_plus_the_zoneinfo_prefix() {
        assert_eq!(MUSL_ETC_PATHS.len(), 9);
        for path in [
            "/etc/group",
            "/etc/hosts",
            "/etc/localtime",
            "/etc/passwd",
            "/etc/resolv.conf",
            "/etc/services",
            "/etc/shadow",
            "/etc/shells",
        ] {
            assert!(MUSL_ETC_PATHS.contains(&path), "{path} is ADR-008 §5.1's");
        }
        assert!(MUSL_ETC_PATHS.contains(&"/etc/zoneinfo/"));
        let mut sorted = MUSL_ETC_PATHS.to_vec();
        sorted.sort_unstable();
        assert_eq!(sorted, MUSL_ETC_PATHS, "the list is not sorted");
    }

    #[test]
    fn every_path_is_under_etc_and_absolute() {
        for f in stage0() {
            assert!(f.path.starts_with("/etc/"), "{} is not under /etc", f.path);
        }
    }

    #[test]
    fn every_file_says_why_it_holds_the_fact_it_holds() {
        for f in stage0() {
            assert!(!f.reason.is_empty(), "{} has no stated reason", f.path);
        }
    }

    // -- the definitions, which the reverse lookup reads ----------------------

    /// `ADR-007` §3.6.1 item 3 asks for three things by name — what it is
    /// generated from, that it is read-only, and the command to change it. The
    /// second is a property of the mount; the other two are per-file, so every
    /// definition has to be able to answer both.
    #[test]
    fn every_definition_says_what_it_is_derived_from_and_what_changes_it() {
        for def in ETC_FILES {
            assert!(!def.source.is_empty(), "{} names no source", def.path);
            assert!(
                !def.changed_by.is_empty(),
                "{} names no way to change it",
                def.path
            );
        }
    }

    /// **The one answer a view may never give.** `ADR-007` §3.6.2: a generated
    /// file is a read-only projection and *never* an interface, so no
    /// `changed_by` may send an operator to an editor — which is precisely the
    /// mistake §3.6's table calls the worst outcome available.
    #[test]
    fn no_definition_tells_an_operator_to_edit_the_file() {
        for def in ETC_FILES {
            let advice = def.changed_by.to_ascii_lowercase();
            for forbidden in ["edit", "vi ", "open it"] {
                assert!(
                    !advice.contains(forbidden),
                    "{} tells an operator to {forbidden:?}: {}",
                    def.path,
                    def.changed_by
                );
            }
        }
    }

    /// The projection is the definitions rendered, and nothing else. A file
    /// appearing in one and not the other is the two-authority failure
    /// `ADR-011` §3 names, in the smallest possible form.
    #[test]
    fn the_projection_renders_exactly_the_definitions_and_carries_their_prose() {
        let servers = [core::net::Ipv4Addr::new(192, 168, 50, 1)];
        let inputs = EtcInputs {
            login_shells: &[CLI_PATH],
            // **Every conditional file is asked for**, so "every definition"
            // is literally true here. A fixture that left one out would make
            // this test agree with a projection that had quietly stopped
            // rendering one.
            nameservers: &servers,
            ca_bundle: Some("-----BEGIN CERTIFICATE-----
"),
            // A hand-composed tzfile header rather than a real zone: this
            // test is about the projection rendering every definition, and
            // the bytes are the database's.
            local_time: Some(LOCALTIME_FIXTURE),
            ..EtcInputs::default()
        };
        let files = etc_projection(&inputs);
        assert_eq!(files.len(), ETC_FILES.len());
        for (file, def) in files.iter().zip(ETC_FILES) {
            assert_eq!(file.path, def.path);
            assert_eq!(file.mode, def.mode);
            assert_eq!(file.reason, def.reason);
            assert_eq!(file.source, def.source);
            assert_eq!(file.changed_by, def.changed_by);
        }
    }

    /// A zone, as the composition root hands it over.
    ///
    /// **Hand-composed, and labelled as such** — `AGENTS.md` §4. It is a real
    /// TZif version-1 header followed by an all-zero body: enough that
    /// `crate::timezone::is_tzfile` accepts it and that a byte-for-byte
    /// assertion has something to compare, and *not* a claim that musl would
    /// resolve a time from it. The tests that need a real zone are on the
    /// board, not here — this crate performs no I/O and has no database.
    const LOCALTIME_FIXTURE: LocalTime<'static> = LocalTime {
        zone: "America/Chicago",
        tzfile: b"TZif2\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
    };

    // ---- /etc/hosts and the machine's own name — ruled 2026-08-26 ---------

    /// **The row this module's table waited on since 2026-08-23.** A machine
    /// with a name resolves it without a network — see `render_hosts` for what
    /// its absence costs a program that looks itself up.
    #[test]
    fn a_named_machine_resolves_its_own_name_without_a_network() {
        let name = crate::hostname::Hostname::new("kitchen-pi").expect("a name");
        let files = etc_projection(&EtcInputs {
            hostname: Some(&name),
            ..EtcInputs::default()
        });
        let hosts = find(&files, HOSTS).expect("hosts is always projected");
        assert!(
            hosts.text().contains(&format!("{HOSTNAME_ADDRESS}\tkitchen-pi\n")),
            "{}",
            hosts.text()
        );
    }

    /// **Not `127.0.0.1`.** Giving the machine's own name `localhost`'s address
    /// would make the two one thing, and a service bound to its own name would
    /// answer for `localhost` too.
    #[test]
    fn the_machines_own_name_does_not_share_localhosts_address() {
        let name = crate::hostname::Hostname::new("kitchen-pi").expect("a name");
        let files = etc_projection(&EtcInputs {
            hostname: Some(&name),
            ..EtcInputs::default()
        });
        let hosts = find(&files, HOSTS).expect("hosts is always projected");
        assert_ne!(HOSTNAME_ADDRESS, "127.0.0.1");
        for line in hosts.text().lines() {
            if line.starts_with("127.0.0.1") {
                assert!(!line.contains("kitchen-pi"), "{line}");
            }
        }
    }

    /// **A nameless machine renders no line, not a line naming nothing.** The
    /// same rule `EtcDefinition::projects` applies to every conditional file:
    /// an entry resolving `(none)` would be a wrong answer where an absent one
    /// is a clean failure.
    #[test]
    fn a_nameless_machine_gets_no_line_rather_than_a_line_naming_nothing() {
        let hosts = find(&with_shell(), HOSTS)
            .expect("hosts is always projected")
            .text()
            .into_owned();
        assert!(!hosts.contains(HOSTNAME_ADDRESS), "{hosts}");
        assert!(!hosts.contains(crate::hostname::KERNEL_DEFAULT), "{hosts}");
        // And the loopback names are untouched, which is the promise a machine
        // built before this key existed renders exactly what it always did.
        assert!(hosts.contains("127.0.0.1\tlocalhost\n"), "{hosts}");
        assert!(hosts.contains("::1\tlocalhost\n"), "{hosts}");
    }

    /// A derived name reaches `/etc/hosts` exactly as a typed one does — there
    /// is one path, which is what makes the default a real default rather than
    /// a special case.
    #[test]
    fn a_derived_name_reaches_etc_hosts_by_the_same_path_a_typed_one_does() {
        let derived = crate::hostname::derived(&[0xa1, 0xb2, 0xc3, 0xd4]).expect("derivable");
        let files = etc_projection(&EtcInputs {
            hostname: Some(&derived),
            ..EtcInputs::default()
        });
        let hosts = find(&files, HOSTS).expect("hosts is always projected");
        assert!(hosts.text().contains("rivet-a1b2c3d4"), "{}", hosts.text());
    }

    /// **THREE definitions are conditional, and each was ruled.** A fourth
    /// appearing without a ruling is the drift this guard is for.
    ///
    /// [`RESOLV_CONF`] is conditional on `[network.resolver]`, since
    /// 2026-09-01: musl falls back to [`MUSL_FALLBACK_NAMESERVER`] where
    /// nothing listens, so *not projected* is a machine that fails name
    /// lookups fast rather than one that answers them wrongly from a stale
    /// file.
    ///
    /// **[`LOCALTIME`] joined it on 2026-09-02** (`ADR-021` §9), and its
    /// absence is a distinct meaning of its own: not *nobody supplied one* but
    /// **this machine keeps UTC** — a state a machine can be in on purpose,
    /// and what musl does with no file.
    ///
    /// **[`CA_BUNDLE`] is the third**, conditional on the image itself
    /// carrying trust roots at [`CA_BUNDLE_SOURCE`]. The name in the test was
    /// a number and has been corrected rather than the number quietly bumped,
    /// because `AGENTS.md` §7 is about exactly this species of claim.
    #[test]
    fn the_conditional_files_are_the_three_that_were_ruled() {
        const CONDITIONAL: [&str; 3] = [CA_BUNDLE, LOCALTIME, RESOLV_CONF];
        let nothing = EtcInputs::default();
        for def in ETC_FILES {
            let projected = (def.projects)(&nothing);
            assert_eq!(
                projected,
                !CONDITIONAL.contains(&def.path),
                "{} is conditional and only {CONDITIONAL:?} were ruled to be",
                def.path
            );
        }
    }

    /// The resolver file appears the moment the document names a nameserver,
    /// and the order written is the order asked — musl queries them in file
    /// order, so sorting would silently change which server answers first.
    #[test]
    fn a_document_with_nameservers_projects_them_in_order() {
        let servers = [
            core::net::Ipv4Addr::new(192, 168, 50, 1),
            core::net::Ipv4Addr::new(1, 1, 1, 1),
        ];
        let inputs = EtcInputs {
            nameservers: &servers,
            search: &["lan"],
            ..EtcInputs::default()
        };
        let files = etc_projection(&inputs);
        let resolv = files
            .iter()
            .find(|f| f.path == RESOLV_CONF)
            .expect("a document with nameservers projects one");
        let resolv = resolv.text();
        let lines: Vec<&str> = resolv
            .lines()
            .filter(|l| !l.starts_with('#'))
            .collect();
        assert_eq!(
            lines,
            [
                "nameserver 192.168.50.1",
                "nameserver 1.1.1.1",
                "search lan"
            ]
        );
    }

    // ---- /etc/localtime — `ADR-021` §9, ruled 2026-09-02 --------------------

    /// **The bytes reach `/etc` unaltered**, which is the whole contract: what
    /// musl maps has to be what the database shipped, or the machine is on a
    /// zone nobody published. `AGENTS.md` §5 — copied, never reproduced.
    #[test]
    fn the_tzfile_reaches_etc_byte_for_byte() {
        let files = etc_projection(&EtcInputs {
            local_time: Some(LOCALTIME_FIXTURE),
            ..EtcInputs::default()
        });
        let localtime = find(&files, LOCALTIME).expect("a machine with a zone projects one");
        assert_eq!(localtime.contents, LOCALTIME_FIXTURE.tzfile);
        assert_eq!(localtime.mode, ETC_FILE_MODE);
        // And what reached /etc is something musl would actually accept, which
        // is the check musl itself performs before it falls back to UTC.
        assert!(crate::timezone::is_tzfile(&localtime.contents));
    }

    /// **Absent is a machine keeping UTC, and it is an absent FILE, not an
    /// empty one:** a zero-byte `/etc/localtime` is not a tzfile, so musl
    /// would unmap it and keep UTC anyway — and an operator running `ls /etc`
    /// would see a timezone that is configured.
    #[test]
    fn no_timezone_is_no_file_rather_than_an_empty_one() {
        let files = etc_projection(&EtcInputs::default());
        assert!(
            !files.iter().any(|f| f.path == LOCALTIME),
            "a machine with no zone projected one: {files:#?}"
        );
    }

    /// The projection stays pure with the new field — a supervisor restart
    /// rewrites identical bytes rather than churning a file something is
    /// reading.
    #[test]
    fn a_zone_renders_the_same_bytes_every_time() {
        let inputs = EtcInputs {
            local_time: Some(LOCALTIME_FIXTURE),
            ..EtcInputs::default()
        };
        assert_eq!(etc_projection(&inputs), etc_projection(&inputs));
    }

    /// **`/etc/localtime` carries no provenance header and cannot**, which is
    /// the same limit `/etc/passwd` has and a sharper one: a comment prepended
    /// to a tzfile moves every offset in it and musl would refuse the magic.
    /// Asserted so that a future pass adding headers "everywhere" fails here
    /// rather than on a board.
    #[test]
    fn a_tzfile_is_not_given_a_header() {
        let files = etc_projection(&EtcInputs {
            local_time: Some(LOCALTIME_FIXTURE),
            ..EtcInputs::default()
        });
        let localtime = find(&files, LOCALTIME).expect("a machine with a zone projects one");
        assert!(localtime.contents.starts_with(crate::timezone::TZIF_MAGIC));
        assert!(!localtime.text().contains("Generated by Rivet"));
    }

    /// The provenance header is `ADR-007` §3.6.1 item 2's, and the string after
    /// "Generated by Rivet from" is the *same* string `explain` reports. Two
    /// spellings of one fact is how the header and the lookup would come to
    /// disagree, and this is the assertion that they cannot.
    #[test]
    fn the_header_names_the_same_source_the_definition_does() {
        let files = with_shell();
        for path in [SHELLS, HOSTS] {
            let (contents, source) = find(&files, path)
                .map(|f| (f.text().into_owned(), f.source))
                .unwrap_or_default();
            assert!(
                contents.starts_with(&format!("# Generated by Rivet from {source}.")),
                "{path} header does not name {source:?}:\n{contents}"
            );
        }
    }

    // -- /etc/shells: the file hardware measurement made mandatory ------------

    /// The whole reason this module exists, as an assertion rather than as a
    /// paragraph. Injecting `MUSL_DEFSHELLS = &[]` must break something.
    #[test]
    fn an_absent_shells_file_would_claim_bin_sh_is_a_login_shell() {
        assert_eq!(MUSL_DEFSHELLS, ["/bin/sh", "/bin/csh"]);
        // And Rivet ships neither, at any stage, by ADR-005 §5.4.
        for path in LOGIN_SHELL_PATHS {
            assert!(!MUSL_DEFSHELLS.contains(path));
        }
    }

    #[test]
    fn a_login_shell_that_is_present_is_the_only_line_in_shells() {
        let text = body(&with_shell(), SHELLS);
        assert_eq!(records(&text), [CLI_PATH]);
    }

    /// `/etc/shells` names **every** shell that is
    /// there, because `getusershell()` is a membership test and an unlisted
    /// shell is refused before authentication.
    #[test]
    fn every_present_login_shell_is_named_in_shells() {
        let text = body(&with_both(), SHELLS);
        assert_eq!(records(&text), [NUSHELL_PATH, CLI_PATH]);
    }

    /// The property that made the dual-login-shell ruling
    /// cheap: **a card built without `nu` renders exactly what it rendered
    /// before the ruling.** Asserted against every file, not just the two the
    /// ruling touches, because the cheap way to break this is to change the
    /// shape of `EtcInputs` and disturb a renderer that was never in scope.
    #[test]
    fn a_card_without_nu_renders_byte_for_byte_what_it_did_before_the_ruling() {
        let files = with_shell();
        assert!(body(&files, PASSWD).contains(&format!(":{CLI_PATH}\n")));
        assert_eq!(records(&body(&files, SHELLS)), [CLI_PATH]);
        for (before, after) in stage0().iter().zip(files.iter()) {
            assert_eq!(before.path, after.path);
            if before.path != PASSWD && before.path != SHELLS {
                assert_eq!(
                    before.contents, after.contents,
                    "{} changed and has nothing to do with a login shell",
                    before.path
                );
            }
        }
    }

    /// Stage 0's honest answer: the file is present, and it lists nothing.
    #[test]
    fn no_login_shell_means_a_present_file_that_permits_nothing() {
        let text = body(&stage0(), SHELLS);
        assert!(
            records(&text).is_empty(),
            "expected no records, got {text:?}"
        );
        assert!(
            !text.is_empty(),
            "the file must still exist and explain itself"
        );
        assert!(
            text.contains("no login shell"),
            "an empty shells file must say why: {text}"
        );
    }

    /// musl's `getusershell()` hands the caller every line verbatim, comments
    /// included, so the set of strings a login check compares against is *every
    /// line of this file*. The property that makes the header harmless is
    /// therefore not "comments are skipped" — it is that no line other than the
    /// intended shell can be equal to any account's shell field, and a shell
    /// field is always an absolute path.
    #[test]
    fn every_line_getusershell_would_return_is_either_the_shell_or_not_a_path() {
        for (text, expected) in [
            (body(&stage0(), SHELLS), None),
            (body(&with_shell(), SHELLS), Some("/system/bin/rivet")),
        ] {
            for line in text.lines() {
                if Some(line) == expected {
                    continue;
                }
                assert!(
                    !line.starts_with('/'),
                    "getusershell() would offer {line:?} as a login shell"
                );
                // And never one of musl's own, which would be the fallback
                // sneaking back in through the file that exists to prevent it.
                assert!(!MUSL_DEFSHELLS.contains(&line), "{line} is musl's fallback");
            }
        }
    }

    // -- /etc/passwd, /etc/group, /etc/shadow ---------------------------------

    #[test]
    fn passwd_has_root_at_uid_zero_with_its_home_on_the_persistent_partition() {
        let text = body(&stage0(), PASSWD);
        let recs = records(&text);
        assert_eq!(
            recs.len(),
            1,
            "exactly one identity is the image's: {recs:?}"
        );
        let fields: Vec<&str> = recs.first().unwrap_or(&"").split(':').collect();
        assert_eq!(fields.len(), 7, "passwd has seven fields: {fields:?}");
        assert_eq!(fields.first(), Some(&"root"));
        assert_eq!(fields.get(1), Some(&"x"), "the hash lives in /etc/shadow");
        assert_eq!(fields.get(2), Some(&"0"));
        assert_eq!(fields.get(3), Some(&"0"));
        // ADR-008 §3.1: home directories are /state/home, which an update never
        // touches. Anywhere under /system would be lost on the next update.
        assert_eq!(fields.get(5), Some(&"/state/home/root"));
    }

    #[test]
    fn roots_shell_is_the_machines_login_shell_when_one_is_present() {
        let text = body(&with_shell(), PASSWD);
        assert!(
            text.contains(&format!(":/state/home/root:{CLI_PATH}")),
            "{text}"
        );
    }

    /// `/etc/passwd` has one seventh field and `LOGIN_SHELL_PATHS` is ordered,
    /// so `root` gets the **first** present entry — `nu` when it shipped, the
    /// CLI when it did not. This is the whole of the rule that orders the candidates.
    #[test]
    fn root_gets_the_first_present_login_shell_and_not_the_last() {
        let both = body(&with_both(), PASSWD);
        assert!(
            both.contains(&format!(":/state/home/root:{NUSHELL_PATH}")),
            "{both}"
        );
        assert!(!both.contains(CLI_PATH), "{both}");

        let cli_only = body(&with_shell(), PASSWD);
        assert!(
            cli_only.contains(&format!(":/state/home/root:{CLI_PATH}")),
            "{cli_only}"
        );
    }

    /// With no login shell, the field is empty rather than naming a binary that
    /// is not on the machine.
    #[test]
    fn roots_shell_is_empty_rather_than_a_path_to_a_binary_that_is_not_there() {
        let text = body(&stage0(), PASSWD);
        let recs = records(&text);
        let fields: Vec<&str> = recs.first().unwrap_or(&"").split(':').collect();
        assert_eq!(fields.get(6), Some(&""));
    }

    /// **The cross-module test.** `stage0_plan` mounts devpts with `gid=5`, and
    /// this file is what makes 5 nameable. They are in different modules and
    /// nothing links them; the failure they produce together is a pty owned by a
    /// group that resolves to nothing.
    #[test]
    fn the_tty_gid_matches_the_one_devpts_is_actually_mounted_with() {
        let devpts = stage0_plan().iter().find(|s| s.fstype == "devpts");
        assert!(devpts.is_some(), "devpts left the stage 0 plan");
        let data = devpts.and_then(|s| s.data).unwrap_or("");
        assert!(
            data.split(',').any(|opt| opt == format!("gid={TTY_GID}")),
            "devpts is mounted with {data:?} but /etc/group names tty as gid {TTY_GID}"
        );
        assert!(body(&stage0(), GROUP).contains(&format!("tty:x:{TTY_GID}:")));
    }

    #[test]
    fn group_holds_exactly_the_groups_that_are_already_in_use() {
        let text = body(&stage0(), GROUP);
        assert_eq!(
            records(&text),
            ["root:x:0:", "tty:x:5:", "repo:x:999:", "render:x:998:"]
        );
        for line in records(&text) {
            assert_eq!(line.split(':').count(), 4, "group has four fields: {line}");
        }
    }

    /// **The repository group has no members, and the file says so rather than
    /// leaving it to be inferred.** `PLAN.md` §53.2 makes the group what *owns*
    /// a repository — `root:git`, mode `2775` — and nothing on this machine has
    /// an identity that would be added to it. A member list that grew one would
    /// be a claim about who may reach those directories that nothing else on the
    /// machine agrees with.
    #[test]
    fn the_repository_group_is_projected_with_no_members() {
        let text = body(&stage0(), GROUP);
        let line = records(&text)
            .into_iter()
            .find(|l| l.starts_with(&format!("{REPO_GROUP}:")))
            .expect("the repository group is always projected");
        assert_eq!(line, format!("{REPO_GROUP}:x:{REPO_GID}:"));
    }

    /// [`REPO_GID`] sits one below the reserved range `crate::account` keeps
    /// empty, so a system gid and anything that range ever holds cannot be one
    /// number.
    #[test]
    fn the_repository_group_sits_below_the_reserved_range() {
        assert_eq!(REPO_GID + 1, crate::account::ACCOUNT_UID_BASE);
        for g in SYSTEM_GROUPS {
            assert!(
                g.gid < crate::account::ACCOUNT_UID_BASE,
                "{} is a system group inside the reserved range",
                g.name
            );
        }
    }

    #[test]
    fn shadow_locks_every_identity_and_carries_no_hash() {
        let text = body(&stage0(), SHADOW);
        assert_eq!(records(&text), ["root:*:::::::"]);
        for line in records(&text) {
            assert_eq!(line.split(':').count(), 9, "shadow has nine fields: {line}");
            let hash = line.split(':').nth(1).unwrap_or("");
            assert_eq!(hash, NO_PASSWORD, "a hash reached the projection: {line}");
        }
    }

    /// `AGENTS.md` §8.3 binds history, so the assertion is about *shape* rather
    /// than about any particular string: nothing that could be a crypt hash may
    /// appear in any file this module renders.
    #[test]
    fn no_file_this_module_renders_contains_anything_shaped_like_a_password_hash() {
        for f in with_shell() {
            assert!(
                !f.text().contains("$1$")
                    && !f.text().contains("$5$")
                    && !f.text().contains("$6$")
                    && !f.text().contains("$y$"),
                "{} contains a crypt hash prefix",
                f.path
            );
        }
    }

    #[test]
    fn only_shadow_is_root_only_and_everything_else_is_world_readable() {
        for f in stage0() {
            if f.path == SHADOW {
                assert_eq!(
                    f.mode, 0o600,
                    "{} must not be readable by anyone else",
                    f.path
                );
            } else {
                assert_eq!(
                    f.mode, 0o644,
                    "{} is looked up by unprivileged processes",
                    f.path
                );
            }
            assert_eq!(f.mode & 0o022, 0, "{} is group- or world-writable", f.path);
            assert_eq!(f.mode & 0o111, 0, "{} is executable", f.path);
        }
    }

    // -- /etc/hosts ------------------------------------------------------------

    #[test]
    fn hosts_maps_both_loopback_addresses_and_nothing_else() {
        let text = body(&stage0(), HOSTS);
        assert_eq!(records(&text), ["127.0.0.1\tlocalhost", "::1\tlocalhost"]);
    }

    /// The hostname is the document's (`ADR-025`), and the schema has no key
    /// for it yet — so the projection names no machine rather than inventing
    /// one. Twenty boards from one image would otherwise all be called the same
    /// thing, which is the failure this test exists to keep out.
    #[test]
    fn hosts_names_no_machine_because_that_is_configuration() {
        let text = body(&stage0(), HOSTS);
        for line in records(&text) {
            let names: Vec<&str> = line.split_whitespace().skip(1).collect();
            assert_eq!(names, ["localhost"], "an extra name appeared: {line}");
        }
    }

    // -- the honesty mechanisms ADR-007 §3.6.1 requires ------------------------

    /// Item 2 of §3.6.1's list, on the two files whose format can carry it.
    #[test]
    fn the_files_that_can_carry_a_provenance_header_do() {
        for path in [SHELLS, HOSTS] {
            let text = body(&stage0(), path);
            assert!(text.starts_with("# Generated by Rivet"), "{path}: {text}");
            assert!(text.contains("Do not edit"), "{path}");
            assert!(
                text.contains("read-only"),
                "{path} must say why an edit fails"
            );
            assert!(text.contains("ADR-007"), "{path} must point at the rule");
        }
    }

    /// And the three that cannot must carry no comment at all, rather than a
    /// header a parser might return as a record.
    #[test]
    fn the_colon_databases_carry_no_comment_lines_at_all() {
        for path in [PASSWD, GROUP, SHADOW] {
            let text = body(&stage0(), path);
            assert!(
                !text.contains('#'),
                "{path} has a comment; musl's parsers are not known to skip one: {text}"
            );
        }
    }

    #[test]
    fn every_file_ends_with_a_newline() {
        for f in with_shell() {
            assert!(
                f.text().ends_with('\n'),
                "{} has no trailing newline",
                f.path
            );
        }
    }

    // -- determinism -----------------------------------------------------------

    /// **Why determinism still matters after the withdrawal.** It used to be
    /// about the signed image: `ADR-015` §4 signs it and `cmd/rivet-image`
    /// guarantees two builds are byte-identical. Nothing of this reaches an
    /// image any more, and the property is *more* load-bearing rather than
    /// less: `ADR-017` §3.1 expects the supervisor to be restarted, and a
    /// projection that varied would rewrite `/etc/passwd` underneath whatever
    /// is reading it, on a schedule nobody chose.
    #[test]
    fn the_same_inputs_produce_the_same_bytes() {
        assert_eq!(
            etc_projection(&EtcInputs::default()),
            etc_projection(&EtcInputs::default())
        );
        assert_eq!(with_shell(), with_shell());
    }

    /// The alias `cmd/rivet-image`'s surviving test still imports, pinned so
    /// that it cannot quietly become a *different* renderer — which is the one
    /// way two generators could come back without anybody deciding to.
    #[test]
    fn the_compatibility_alias_is_the_same_projection_and_not_a_second_one() {
        assert_eq!(image_etc(&EtcInputs::default()), stage0());
        assert_eq!(
            image_etc(&EtcInputs {
                login_shells: &[CLI_PATH],
                ..EtcInputs::default()
            }),
            with_shell()
        );
    }

    #[test]
    fn the_files_are_sorted_and_no_path_appears_twice() {
        let paths: Vec<_> = stage0().iter().map(|f| f.path).collect();
        let mut sorted = paths.clone();
        sorted.sort_unstable();
        assert_eq!(paths, sorted);
        sorted.dedup();
        assert_eq!(sorted.len(), paths.len(), "a path appears twice");
    }

    /// `/etc/shadow`'s third field is conventionally the day the password last
    /// changed, counted from the epoch. It is the one place in these five files
    /// where a clock reading would arrive looking reasonable, and it would cost
    /// the determinism above — invisibly, because a projection compared against
    /// itself minutes later still matches.
    #[test]
    fn shadows_ageing_fields_are_empty_rather_than_a_day_count() {
        let text = body(&stage0(), SHADOW);
        for line in records(&text) {
            let ageing: Vec<&str> = line.split(':').skip(2).collect();
            assert!(
                ageing.iter().all(|f| f.is_empty()),
                "an ageing field is set, which would put a clock reading in /etc: {line}"
            );
        }
    }

    // -- where /etc lives ------------------------------------------------------

    /// `ADR-007` §3.5 in one assertion. A `/etc` that were anything but a tmpfs
    /// would be a place edits could persist, which §3.6's table calls the worst
    /// outcome available.
    #[test]
    fn etc_is_a_tmpfs_at_the_path_musl_compiles_in() {
        assert_eq!(ETC_MOUNT_POINT, "/etc");
        assert_eq!(ETC_FSTYPE, "tmpfs");
        for path in MUSL_ETC_PATHS {
            assert!(
                path.starts_with(&format!("{ETC_MOUNT_POINT}/")),
                "{path} is not under the filesystem the projection mounts"
            );
        }
    }

    /// The three flags, and the absence of a fourth. `noexec` here is the
    /// **opposite** call from `crate::mount`'s `/run`, and asserting it both
    /// ways is what stops a later tidy-up from making them agree.
    #[test]
    fn etc_is_noexec_nosuid_nodev_because_nothing_in_it_is_a_program() {
        for flag in [MountFlag::NoExec, MountFlag::NoSuid, MountFlag::NoDev] {
            assert!(ETC_FLAGS.contains(&flag), "/etc is missing {flag:?}");
        }
        assert!(
            !ETC_FLAGS.contains(&MountFlag::NoAtime),
            "noatime is ADR-018 §7's, for the card; there is no card behind a tmpfs"
        );
    }

    /// A tmpfs mounts `1777` by default, which would be a world-writable `/etc`
    /// — the hand-edit `ADR-007` §3.6.1 item 1 exists to refuse, available to
    /// every process on the machine.
    #[test]
    fn the_tmpfs_is_mode_755_rather_than_the_world_writable_default() {
        let data = ETC_TMPFS_DATA.unwrap_or("");
        assert!(
            data.split(',').any(|opt| opt == "mode=755"),
            "the /etc tmpfs would mount 1777: {data:?}"
        );
    }

    /// The cap is only honest if the thing being capped fits inside it with
    /// room to spare. Measured rather than asserted from memory.
    #[test]
    fn the_whole_projection_fits_inside_the_tmpfs_cap() {
        let bytes: usize = with_shell().iter().map(|f| f.contents.len()).sum();
        // `size=1m` is 1 MiB. A hundredth of it is already generous for five
        // short text files, and this fails long before the tmpfs would.
        assert!(
            bytes < 1024 * 1024 / 100,
            "the projection is {bytes} bytes, which is close enough to ETC_TMPFS_DATA's \
             cap that the cap needs re-deciding rather than the test relaxing"
        );
    }

    /// **PID 1 does not mount `/etc`, and that is `ADR-017` §3.1.8's ruling
    /// rather than an omission.** Adding it to `stage0_plan` must break a test
    /// that says why it is not there.
    #[test]
    fn etc_is_not_in_pid_1s_mount_plan() {
        assert!(
            !stage0_plan().iter().any(|s| s.target == ETC_MOUNT_POINT),
            "PID 1 mounts /etc; ADR-017 §3.1.5's job list did not grow, and the \
             supervisor renders /etc before it starts anything that reads it"
        );
        // Nor may anything in the plan be mounted over /etc's parent.
        for spec in stage0_plan() {
            assert!(
                !ETC_MOUNT_POINT.starts_with(&format!("{}/", spec.target)),
                "{ETC_MOUNT_POINT} is inside {}, which would hide it",
                spec.target
            );
        }
    }

    /// Each path is a candidate, not a claim. One naming something under
    /// `/state` or `/run` would put a login shell on a filesystem an update or
    /// a reboot can empty.
    #[test]
    fn the_login_shell_path_is_on_the_read_only_system_image() {
        assert_eq!(CLI_PATH, "/system/bin/rivet");
        assert_eq!(NUSHELL_PATH, "/system/bin/nu");
        for path in LOGIN_SHELL_PATHS {
            assert!(path.starts_with("/system/"), "{path}");
        }
    }

    /// The order is the ruling, so it is asserted rather
    /// than left to whoever next edits the array. Reversing it would silently
    /// make `cmd/rivet` `root`'s shell on a card that ships `nu` — which
    /// reads as working, and takes away the pipeline that was the point of
    /// §5.1.
    #[test]
    fn nushell_is_first_and_the_cli_is_the_fallback() {
        // The restricted shell WAS third, in the list only so `getusershell()`
        // would admit the accounts naming it. That program is gone (`ADR-035`),
        // nothing calls `getusershell()` any more, and the entry is gone with
        // the program it named.
        assert_eq!(LOGIN_SHELL_PATHS, [NUSHELL_PATH, CLI_PATH]);
        assert_eq!(LOGIN_SHELL_PATHS[0], NUSHELL_PATH);
        assert_eq!(LOGIN_SHELL_PATHS[1], CLI_PATH);
    }

    // -- the accounts an installed application brings with it -----------------

    fn app(name: &str) -> crate::service::ServiceName {
        crate::service::ServiceName::new(name).expect("a valid fixture name")
    }

    fn with_apps<'a>(
        names: &'a [crate::service::ServiceName],
        into: &'a mut Vec<ApplicationAccount<'a>>,
    ) -> Vec<EtcFile> {
        into.clear();
        for name in names {
            into.push(ApplicationAccount {
                name,
                identity: crate::service::app_identity(name),
                volume: crate::service::BUILT_IN_VOLUME,
            });
        }
        etc_projection(&EtcInputs {
            login_shells: &[CLI_PATH],
            applications: into,
            hostname: None,
            nameservers: &[],
            search: &[],
            ca_bundle: None,
            local_time: None,
        })
    }

    /// **The promise the field was added under.** A machine with nothing
    /// installed renders exactly what it rendered before application accounts
    /// existed — byte for byte, not merely "the same shape".
    #[test]
    fn a_machine_with_no_applications_renders_exactly_what_it_always_did() {
        let names: Vec<crate::service::ServiceName> = Vec::new();
        let mut accounts = Vec::new();
        assert_eq!(with_apps(&names, &mut accounts), with_shell());
    }

    /// One installed application, in all three databases, with the identity its
    /// own name derives.
    #[test]
    fn an_installed_application_gets_a_passwd_a_group_and_a_shadow_record() {
        let names = vec![app("photo-index")];
        let mut accounts = Vec::new();
        let files = with_apps(&names, &mut accounts);
        let id = crate::service::app_identity(&names[0]);

        let passwd_text = body(&files, PASSWD);
        let passwd = records(&passwd_text);
        assert_eq!(passwd.len(), SYSTEM_IDENTITIES.len() + 1, "{passwd:?}");
        let record = passwd.last().copied().unwrap_or_default();
        let fields: Vec<&str> = record.split(':').collect();
        assert_eq!(
            fields.len(),
            7,
            "a passwd record has seven fields: {record}"
        );
        assert_eq!(fields[0], "photo-index");
        assert_eq!(fields[1], "x", "the hash lives in /etc/shadow");
        assert_eq!(fields[2], id.uid.to_string());
        assert_eq!(fields[3], id.gid.to_string());
        assert_eq!(fields[5], crate::service::data_dir(crate::service::BUILT_IN_VOLUME, &names[0]));
        assert_eq!(
            fields[6], "",
            "an application account is an identity, not a way in"
        );

        let group_text = body(&files, GROUP);
        let group = records(&group_text);
        assert_eq!(group.len(), SYSTEM_GROUPS.len() + 1);
        assert_eq!(
            group.last().copied().unwrap_or_default(),
            format!("photo-index:x:{}:", id.gid),
            "an application's group has exactly one member, through passwd's primary gid"
        );

        let shadow_text = body(&files, SHADOW);
        let shadow = records(&shadow_text);
        assert_eq!(shadow.len(), SYSTEM_IDENTITIES.len() + 1);
        assert_eq!(
            shadow.last().copied().unwrap_or_default(),
            format!("photo-index:{NO_PASSWORD}:::::::"),
            "an application account must never be authenticable"
        );
    }

    /// **The one that would let a login through.** An application account's
    /// shell must be in no `/etc/shells`, so a login is refused before any
    /// credential is looked at. This is the test that fails on the
    /// day somebody gives these records a shell "so they look normal".
    #[test]
    fn no_application_accounts_shell_is_a_permitted_login_shell() {
        let names = vec![app("photo-index"), app("metrics")];
        let mut accounts = Vec::new();
        let files = with_apps(&names, &mut accounts);
        let shells_text = body(&files, SHELLS);
        let permitted = records(&shells_text);
        let passwd_text = body(&files, PASSWD);
        for record in records(&passwd_text)
            .into_iter()
            .skip(SYSTEM_IDENTITIES.len())
        {
            let shell = record.split(':').nth(6).unwrap_or("MISSING");
            assert!(
                !permitted.contains(&shell),
                "{record} names a shell /etc/shells permits"
            );
        }
    }

    /// An application never takes a name or a uid the image already defines.
    /// `root` being shadowed by an application called `root` would be a machine
    /// with two answers to `getpwnam("root")`, and the second is the one musl
    /// would find only if it read the whole file.
    #[test]
    fn no_application_account_can_collide_with_the_images_own() {
        let names = vec![app("photo-index"), app("metrics"), app("a"), app("z9")];
        let mut accounts = Vec::new();
        let files = with_apps(&names, &mut accounts);
        let passwd = body(&files, PASSWD);
        let mut seen_names: Vec<&str> = Vec::new();
        let mut seen_uids: Vec<&str> = Vec::new();
        for record in records(&passwd) {
            let fields: Vec<&str> = record.split(':').collect();
            assert!(!seen_names.contains(&fields[0]), "{} twice", fields[0]);
            assert!(!seen_uids.contains(&fields[2]), "uid {} twice", fields[2]);
            seen_names.push(fields[0]);
            seen_uids.push(fields[2]);
        }
        // And no application landed on uid 0 — which the range forbids, and
        // which is asserted here as well because this is the file where being
        // wrong about it is silent.
        assert_eq!(
            seen_uids.iter().filter(|u| **u == "0").count(),
            1,
            "something other than root claims uid 0"
        );
    }

    /// The projection stays pure with the new field: same inputs, same bytes.
    /// It is what makes a supervisor restart rewrite identical files rather
    /// than churn one something is reading.
    #[test]
    fn the_projection_is_still_a_function_of_its_inputs_alone() {
        let names = vec![app("photo-index"), app("metrics")];
        let (mut a, mut b) = (Vec::new(), Vec::new());
        let first = with_apps(&names, &mut a);
        let second = with_apps(&names, &mut b);
        assert_eq!(first, second);
    }
}
