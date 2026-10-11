//! **Landlock: `rivet_core::confinement`'s policy, as a kernel ruleset.**
//! `ADR-042`, built 2026-09-04.
//!
//! This module is the *only* place in the tree that names a Landlock constant.
//! `rivet-core` owns what an application may reach and says so in Rivet's own
//! verbs (`AGENTS.md` §3 — the domain names no platform type); this is the
//! table that turns each verb into a bit, and the three syscalls that make the
//! kernel believe it.
//!
//! # The three calls, and which side of the `fork` each is on
//!
//! | Call | Where | Why there |
//! | :-- | :-- | :-- |
//! | `landlock_create_ruleset(NULL, 0, VERSION)` | the parent, before every spawn | it is a **question**, and `AGENTS.md` §5 says never cache the kernel's verdict |
//! | `landlock_create_ruleset(attr, …)` + `landlock_add_rule` | the parent, before `fork` | each rule needs a directory **opened**, and the child between `fork` and `exec` may not allocate the `Vec` that would take |
//! | `landlock_restrict_self(fd, 0)` | the **child**, before `exec` | it restricts the calling thread and everything it later execs. In the parent it would confine the supervisor |
//!
//! That last row is the failure this whole feature is most likely to have, and
//! it is the reason [`restrict_self`] is a free function taking a raw
//! descriptor rather than a method on anything: there is nothing here that
//! could be called on the wrong process by accident and still compile.
//!
//! # Ordering against the rest of the privilege drop
//!
//! `crate::kernel`'s `pre_exec` closure runs `confine()`, then this, then
//! `drop_privilege`. Both edges matter:
//!
//! * **After `PR_SET_NO_NEW_PRIVS`**, because `landlock_restrict_self` refuses
//!   with `EPERM` for a caller that has neither that bit nor `CAP_SYS_ADMIN`.
//!   Setting the bit first is not an optimisation; it is the precondition.
//! * **Before `setuid`**, which is not required and is chosen anyway: the
//!   ruleset is inherited across `execve` and can never be removed, so the
//!   narrowest possible window between "still root" and "confined" is the one
//!   with nothing in it that could fail *after* the drop and leave a child
//!   running unconfined as its own uid.
//!
//! # What a missing right does, and why that is not the same as a missing
//! Landlock
//!
//! The ABI has grown, and a kernel that predates a right cannot **handle** it —
//! `landlock_create_ruleset` answers `EINVAL` for a `handled_access_fs` with a
//! bit the kernel does not know. So [`handled_for`] intersects with what the
//! running ABI supports, and the rights that drop out are named, per
//! application, on the journal line the supervisor emits. `ADR-042` §5 argues
//! why that is reported rather than refused, and why **ABI 0 — no Landlock at
//! all — is refused** instead.
//!
//! # The one network right this module handles
//!
//! ABI 4 adds `LANDLOCK_ACCESS_NET_BIND_TCP` and
//! `LANDLOCK_ACCESS_NET_CONNECT_TCP`. [`build`] handles the first and grants
//! it to nothing, so on ABI 4 and later an application cannot `bind` a TCP
//! port at all — [`handled_access_net_for`] is where that is decided and says
//! why. Below ABI 4 nothing changes: there is no `handled_access_net` field
//! for a kernel that old to have an opinion about, and [`RulesetAttrNet`]'s
//! own comment is the argument that reaching for the newer struct only on the
//! ABI that defines it is sound rather than merely convenient.

use rivet_core::confinement::{Access, Policy, Presence};

use crate::kernel::Errno;

// ---------------------------------------------------------------------------
// The kernel's own numbers
// ---------------------------------------------------------------------------

/// `landlock_create_ruleset(2)`. Same number on `aarch64` and `x86_64`; the
/// build refuses any other architecture below rather than guessing.
const SYS_CREATE_RULESET: libc::c_long = 444;
/// `landlock_add_rule(2)`.
const SYS_ADD_RULE: libc::c_long = 445;
/// `landlock_restrict_self(2)`.
const SYS_RESTRICT_SELF: libc::c_long = 446;

/// **The syscall numbers above are asserted, not assumed.** Landlock's three
/// syscalls were added together and every architecture that has them uses
/// 444/445/446 except the ones with their own numbering, which Rivet does not
/// build for (`ADR-002`). A new target arriving would compile silently against
/// three wrong numbers, so it does not compile at all.
const _: () = assert!(
    cfg!(target_arch = "aarch64") || cfg!(target_arch = "x86_64"),
    "the Landlock syscall numbers here are aarch64's and x86_64's; check them before adding a target"
);

/// `LANDLOCK_CREATE_RULESET_VERSION` — ask, do not create.
const CREATE_RULESET_VERSION: u32 = 1;
/// `LANDLOCK_RULE_PATH_BENEATH`.
const RULE_PATH_BENEATH: libc::c_int = 1;

/// `struct landlock_path_beneath_attr`, which the kernel declares **packed**.
///
/// Twelve bytes, not sixteen. A `#[repr(C)]` here would pad to sixteen and the
/// kernel would read `parent_fd` out of the padding — a rule for descriptor
/// zero, silently, on every call.
#[repr(C, packed)]
struct PathBeneath {
    allowed_access: u64,
    parent_fd: i32,
}

/// `struct landlock_ruleset_attr`, on an ABI that has `handled_access_net`.
///
/// **Not packed, and does not need to be**: three `u64`s are already aligned
/// to their own size, so `#[repr(C)]` and `#[repr(C, packed)]` describe the
/// same bytes here. Written out rather than reused as `[u64; 3]` so the field
/// names carry the same meaning the kernel's header gives them.
///
/// **Extensible on purpose, and this relies on the kernel's own rule for
/// it.** `landlock_create_ruleset` accepts an attribute struct shorter or
/// longer than the one the running kernel knows: shorter is zero-filled,
/// longer is accepted only if every byte past what the kernel knows is zero
/// (`E2BIG` otherwise). So this struct is passed whole on every ABI from 4
/// up — including one that predates `scoped` (ABI 4 and 5, where it is the
/// zero tail the kernel fills in for itself) and one that might add a fourth
/// field later (where THIS struct becomes the zero-filled short side of the
/// same rule). Nothing here has to track which ABI grew which field.
#[repr(C)]
struct RulesetAttrNet {
    handled_access_fs: u64,
    handled_access_net: u64,
    /// `LANDLOCK_SCOPE_*` — ABI 6. Always zero: scoping abstract unix sockets
    /// and signals between sandboxes is no part of this change, and a
    /// kernel this struct reaches (ABI 4 or 5) does not know the field
    /// exists at all.
    scoped: u64,
}

// `LANDLOCK_ACCESS_FS_*`, in the order the kernel declares them.
const FS_EXECUTE: u64 = 1 << 0;
const FS_WRITE_FILE: u64 = 1 << 1;
const FS_READ_FILE: u64 = 1 << 2;
const FS_READ_DIR: u64 = 1 << 3;
const FS_REMOVE_DIR: u64 = 1 << 4;
const FS_REMOVE_FILE: u64 = 1 << 5;
const FS_MAKE_CHAR: u64 = 1 << 6;
const FS_MAKE_DIR: u64 = 1 << 7;
const FS_MAKE_REG: u64 = 1 << 8;
const FS_MAKE_SOCK: u64 = 1 << 9;
const FS_MAKE_FIFO: u64 = 1 << 10;
const FS_MAKE_BLOCK: u64 = 1 << 11;
const FS_MAKE_SYM: u64 = 1 << 12;
/// ABI 2.
const FS_REFER: u64 = 1 << 13;
/// ABI 3.
const FS_TRUNCATE: u64 = 1 << 14;
/// ABI 5.
const FS_IOCTL_DEV: u64 = 1 << 15;

// `LANDLOCK_ACCESS_NET_*`, ABI 4. A separate bit space from `FS_*` above —
// they live in the ruleset's `handled_access_net` field, not
// `handled_access_fs`, so `1 << 0` here does not collide with `FS_EXECUTE`.
/// `LANDLOCK_ACCESS_NET_BIND_TCP`. The one network right this module handles
/// — see [`handled_access_net_for`] for what that means and why the other one
/// is not here.
const NET_BIND_TCP: u64 = 1 << 0;
// `LANDLOCK_ACCESS_NET_CONNECT_TCP` exists at this bit and is deliberately
// never read anywhere in this module — see `handled_access_net_for`. Named
// here in a plain comment rather than a `const` so that `deny(dead_code)`
// never has to fire for it: an unused item is a question a reader has to
// answer, and the answer belongs beside `NET_BIND_TCP` where it is decided,
// not scattered as an `#[allow]`.

/// The lowest ABI at which Landlock has any network right at all — the ABI
/// that added `handled_access_net` to `struct landlock_ruleset_attr`.
///
/// Below this, [`build`] passes the same one-`u64` attribute it always has:
/// there is no `handled_access_net` field for a kernel this old to reject or
/// honour, and probing for one with a longer struct would be asking a
/// question the kernel has no field to answer.
const NET_RIGHTS_ABI: u32 = 4;

/// Every filesystem right the running ABI knows about.
///
/// **This is the set the ruleset HANDLES**, which is what makes everything not
/// granted denied. A right that is not handled is not restricted at all, so a
/// short list here is a wide sandbox — see [`unhandled_words`], which is how
/// that shortfall reaches an operator instead of staying in this function.
#[must_use]
pub const fn handled_for(abi: u32) -> u64 {
    if abi == 0 {
        return 0;
    }
    let mut mask = FS_EXECUTE
        | FS_WRITE_FILE
        | FS_READ_FILE
        | FS_READ_DIR
        | FS_REMOVE_DIR
        | FS_REMOVE_FILE
        | FS_MAKE_CHAR
        | FS_MAKE_DIR
        | FS_MAKE_REG
        | FS_MAKE_SOCK
        | FS_MAKE_FIFO
        | FS_MAKE_BLOCK
        | FS_MAKE_SYM;
    if abi >= 2 {
        mask |= FS_REFER;
    }
    if abi >= 3 {
        mask |= FS_TRUNCATE;
    }
    if abi >= 5 {
        mask |= FS_IOCTL_DEV;
    }
    mask
}

/// Every network right the running ABI knows about **that this ruleset
/// handles** — which is at most one bit, and it is the point of this whole
/// change.
///
/// **`BIND_TCP`, granted to nothing.** The gap this closes: without it, an
/// application could `bind` any TCP port its uid could reach, including one
/// the supervisor had promised to another application by pre-binding it and
/// handing over the descriptor — `SPEC-03` §3's listening-socket contract.
/// Handling the right and adding no `landlock_add_rule` for it is the whole
/// implementation of *deny* — Landlock refuses everything a ruleset handles
/// and no rule allows.
///
/// **`CONNECT_TCP` is never added here, on purpose, and that is `ADR-042` §7
/// rather than an oversight.** `BIND_TCP` can be handled and granted to
/// nothing because no application has a lawful reason to bind one at all —
/// the supervisor pre-binds every socket a manifest declares, per the same
/// `SPEC-03` §3 contract. Outbound connections cannot be denied the same
/// way: most applications need *some* host reachable, and deciding which
/// ones needs a vocabulary the manifest does not have. Handling
/// `CONNECT_TCP` today, with no policy row to narrow it, would deny every
/// application every outbound TCP connection — a functional regression
/// dressed as a security one, and a different decision than this change
/// makes.
///
/// **`bind` here means TCP only.** Landlock's network access vector says
/// nothing about UDP, so `rivet-mdnsd`'s bound `5353` and `rivet-netd`'s
/// netlink socket are untouched by this function and by everything `build`
/// does with its answer.
#[must_use]
pub const fn handled_access_net_for(abi: u32) -> u64 {
    if abi >= NET_RIGHTS_ABI {
        NET_BIND_TCP
    } else {
        0
    }
}

/// The lowest ABI at which a Rivet policy means what it says.
///
/// **One.** Every verb `rivet_core::confinement` uses for a *path* — read,
/// list, execute, write, create, delete — is in Landlock's first release. The
/// three later ones ([`Access::REPARENT`], [`Access::TRUNCATE`],
/// [`Access::DEVICE_CONTROL`]) are refinements whose absence widens the
/// sandbox in a way this module can name exactly, which is why `ADR-042` §5
/// reports them rather than refusing.
pub const MINIMUM_ABI: u32 = 1;

/// One Rivet verb, as the kernel's bits, on a kernel that knows them all.
///
/// Kept whole and intersected afterwards rather than written per-ABI: the
/// mapping is a fact about what a verb *means* and does not change with the
/// kernel, and the shortfall is then one `&` in one place instead of a
/// condition in nine.
#[must_use]
pub const fn bits_of(verb: Access) -> u64 {
    // A `match` cannot be written over a bitset, and a chain of `if`s is what
    // this is: one verb in, one mask out.
    if verb.bits() == Access::READ.bits() {
        FS_READ_FILE
    } else if verb.bits() == Access::LIST.bits() {
        FS_READ_DIR
    } else if verb.bits() == Access::EXECUTE.bits() {
        FS_EXECUTE
    } else if verb.bits() == Access::WRITE.bits() {
        FS_WRITE_FILE
    } else if verb.bits() == Access::CREATE.bits() {
        // **`MAKE_CHAR` and `MAKE_BLOCK` are deliberately absent.** A device
        // node an application made itself is a door to hardware no manifest
        // declared; `rivet_core::confinement::Access::CREATE` says so, and this
        // is where the saying is enforced.
        FS_MAKE_REG | FS_MAKE_DIR | FS_MAKE_SOCK | FS_MAKE_FIFO | FS_MAKE_SYM
    } else if verb.bits() == Access::DELETE.bits() {
        FS_REMOVE_FILE | FS_REMOVE_DIR
    } else if verb.bits() == Access::TRUNCATE.bits() {
        FS_TRUNCATE
    } else if verb.bits() == Access::REPARENT.bits() {
        FS_REFER
    } else if verb.bits() == Access::DEVICE_CONTROL.bits() {
        FS_IOCTL_DEV
    } else {
        0
    }
}

/// Every verb in `access`, as the kernel's bits.
#[must_use]
pub fn mask_of(access: Access) -> u64 {
    const EVERY: &[Access] = &[
        Access::READ,
        Access::LIST,
        Access::EXECUTE,
        Access::WRITE,
        Access::CREATE,
        Access::DELETE,
        Access::TRUNCATE,
        Access::REPARENT,
        Access::DEVICE_CONTROL,
    ];
    EVERY
        .iter()
        .filter(|verb| access.holds(**verb))
        .fold(0, |mask, verb| mask | bits_of(*verb))
}

/// Which of Rivet's verbs this kernel cannot enforce, in words.
///
/// Empty on ABI 5 and later. **Not a generic "degraded" flag**: an operator
/// reading *"truncate"* knows that a file inside a read-only grant can still
/// be emptied, which is a specific thing to reason about, and a boolean is not.
#[must_use]
pub fn unhandled_words(abi: u32) -> Vec<&'static str> {
    let handled = handled_for(abi);
    let mut out = Vec::new();
    if handled & FS_REFER == 0 {
        out.push("reparent");
    }
    if handled & FS_TRUNCATE == 0 {
        out.push("truncate");
    }
    if handled & FS_IOCTL_DEV == 0 {
        out.push("device-control");
    }
    out
}

// ---------------------------------------------------------------------------
// The syscalls
// ---------------------------------------------------------------------------

/// What the running kernel offers: the Landlock ABI version, or **0**.
///
/// Zero is the honest answer for every way of not having it — the syscall is
/// missing (`ENOSYS`), the LSM is compiled out or not in `CONFIG_LSM`
/// (`EOPNOTSUPP`), or something else refused. They are one answer here because
/// they have one consequence: `ADR-042` §5 refuses to start an application, and
/// the errno is carried on the refusal so the difference is still readable.
///
/// **Asked at every spawn.** `AGENTS.md` §5: the kernel is the authority for
/// system state and its verdicts are never cached.
#[must_use]
pub fn abi() -> (u32, i32) {
    // SAFETY: a query, not a creation. `LANDLOCK_CREATE_RULESET_VERSION` with a
    // NULL attribute and a zero size is the documented way to ask, and it
    // creates no descriptor and changes nothing about this process.
    #[allow(unsafe_code)]
    let answer = unsafe {
        libc::syscall(
            SYS_CREATE_RULESET,
            std::ptr::null::<u8>(),
            0_usize,
            CREATE_RULESET_VERSION,
        )
    };
    if answer < 0 {
        return (0, std::io::Error::last_os_error().raw_os_error().unwrap_or(0));
    }
    (u32::try_from(answer).unwrap_or(0), 0)
}

/// A path a rule could not be added for, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fault {
    /// The path the policy named.
    pub path: String,
    /// What the kernel said.
    pub errno: Errno,
    /// What was being attempted.
    pub doing: &'static str,
}

impl std::fmt::Display for Fault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}: {}", self.doing, self.path, self.errno)
    }
}

/// A ruleset that has been built and not yet applied.
///
/// Holds the descriptor, and closes it on drop. **It is not the confinement**:
/// a `Ruleset` that is created and never handed to [`restrict_self`] in a child
/// confines nothing at all, which is the single most likely way this feature
/// could appear to work — see `ADR-042` §6 for the test that would catch it.
pub struct Ruleset {
    fd: std::os::fd::OwnedFd,
    /// Paths that were granted, in the policy's order. For the journal.
    pub granted: Vec<String>,
    /// Optional paths that were not there, so were not granted.
    pub absent: Vec<String>,
}

impl Ruleset {
    /// The descriptor, for the `pre_exec` closure to pass to [`restrict_self`].
    #[must_use]
    pub fn raw(&self) -> std::os::fd::RawFd {
        std::os::fd::AsRawFd::as_raw_fd(&self.fd)
    }
}

/// Build the ruleset for one policy, in the parent, before `fork`.
///
/// # Errors
///
/// * the ruleset could not be created — the kernel refused the handled set;
/// * a [`Presence::Required`] path could not be opened;
/// * `landlock_add_rule` refused a path that opened.
///
/// **A required path that is absent is a fault and not a smaller ruleset.** Its
/// own installed files and its own data directory are things the machine
/// promised; if one is missing the application is broken, and starting it
/// confined-but-differently would hide that behind an `EACCES` inside the
/// program (`ADR-015` §5.1.1's argument about the uid, in a new place).
pub fn build(policy: &Policy, abi: u32) -> Result<Ruleset, Fault> {
    let handled_fs = handled_for(abi);
    let handled_net = handled_access_net_for(abi);

    // **Two shapes of attribute, gated on the same ABI `handled_access_net_for`
    // gates on** — `ADR-042` §7. Below ABI 4 there is no `handled_access_net`
    // field for a kernel to have an opinion about, so the call is exactly what
    // it always was: one `u64`, and the kernel zero-fills the rest of its own
    // struct. At ABI 4 and above `handled_net` may be nonzero, and a `u64`
    // could not carry it — passing one would leave `BIND_TCP` unhandled on
    // every kernel new enough to enforce it, which is this feature doing
    // nothing while believing itself built.
    let created = if abi >= NET_RIGHTS_ABI {
        let attr = RulesetAttrNet {
            handled_access_fs: handled_fs,
            handled_access_net: handled_net,
            scoped: 0,
        };
        // SAFETY: `attr` is one `RulesetAttrNet` on this stack, and the size
        // passed is its own size. `RulesetAttrNet`'s own doc comment is the
        // argument that this is sound on ABI 4, 5 and 6 alike: the struct is
        // `struct landlock_ruleset_attr`'s own layout, and a kernel that knows
        // fewer of its fields than are set here still accepts the call because
        // every byte past what it knows (`scoped`, on ABI 4 and 5) is zero.
        #[allow(unsafe_code)]
        unsafe {
            libc::syscall(
                SYS_CREATE_RULESET,
                std::ptr::from_ref(&attr),
                std::mem::size_of::<RulesetAttrNet>(),
                0_u32,
            )
        }
    } else {
        let attr: u64 = handled_fs;
        // SAFETY: `attr` is one `u64` on this stack and the size passed is its
        // own size. `struct landlock_ruleset_attr` begins with
        // `handled_access_fs` and the kernel accepts a struct shorter than the
        // one it knows, filling the rest with zero — which is what is wanted
        // on a kernel this old: it has no `handled_access_net` field at all,
        // so there is nothing this call could hand it about the network.
        #[allow(unsafe_code)]
        unsafe {
            libc::syscall(
                SYS_CREATE_RULESET,
                std::ptr::from_ref(&attr),
                std::mem::size_of::<u64>(),
                0_u32,
            )
        }
    };
    if created < 0 {
        return Err(Fault {
            path: String::new(),
            errno: Errno::from_io(&std::io::Error::last_os_error()),
            doing: "creating the ruleset for",
        });
    }
    // SAFETY: a descriptor the kernel just returned, owned by nothing else.
    // `landlock_create_ruleset` sets `FD_CLOEXEC` on it, so it survives the
    // fork — which is when the child needs it — and is gone at `exec`.
    #[allow(unsafe_code)]
    let fd = unsafe {
        <std::os::fd::OwnedFd as std::os::fd::FromRawFd>::from_raw_fd(
            i32::try_from(created).unwrap_or(-1),
        )
    };
    let mut ruleset = Ruleset {
        fd,
        granted: Vec::new(),
        absent: Vec::new(),
    };

    for grant in &policy.grants {
        let allowed = mask_of(grant.access) & handled_fs;
        if allowed == 0 {
            // Every verb this grant asked for is one the kernel cannot handle.
            // Adding a rule with an empty mask is `EINVAL`, and the honest
            // reading is that there is nothing to say about this path on this
            // kernel — `unhandled_words` is where an operator learns that.
            continue;
        }
        let Some(parent) = open_path(&grant.path)? else {
            match grant.presence {
                Presence::Required => {
                    return Err(Fault {
                        path: grant.path.clone(),
                        errno: Errno(libc::ENOENT),
                        doing: "the policy requires",
                    });
                }
                Presence::Optional => {
                    ruleset.absent.push(grant.path.clone());
                    continue;
                }
            }
        };
        let rule = PathBeneath {
            allowed_access: allowed,
            parent_fd: std::os::fd::AsRawFd::as_raw_fd(&parent),
        };
        // SAFETY: `rule` is a live `#[repr(C, packed)]` value matching
        // `struct landlock_path_beneath_attr`, and `parent` is open for the
        // whole call.
        #[allow(unsafe_code)]
        let added = unsafe {
            libc::syscall(
                SYS_ADD_RULE,
                ruleset.raw(),
                RULE_PATH_BENEATH,
                std::ptr::from_ref(&rule),
                0_u32,
            )
        };
        if added != 0 {
            return Err(Fault {
                path: grant.path.clone(),
                errno: Errno::from_io(&std::io::Error::last_os_error()),
                doing: "adding a rule for",
            });
        }
        ruleset.granted.push(grant.path.clone());
    }

    Ok(ruleset)
}

/// `open(path, O_PATH | O_CLOEXEC)`, or `Ok(None)` when there is nothing there.
///
/// `O_PATH` because a rule needs the *inode* and not the contents: a directory
/// opened this way can be named in a rule without being read, and a device node
/// opened this way does not go near the driver.
fn open_path(path: &str) -> Result<Option<std::os::fd::OwnedFd>, Fault> {
    let Ok(c_path) = std::ffi::CString::new(path) else {
        return Err(Fault {
            path: path.to_string(),
            errno: Errno(libc::EINVAL),
            doing: "a NUL byte is in the path",
        });
    };
    // SAFETY: `c_path` is NUL-terminated and lives across the call.
    #[allow(unsafe_code)]
    let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_PATH | libc::O_CLOEXEC) };
    if fd < 0 {
        let error = std::io::Error::last_os_error();
        // **`ENOENT` is the only "not there" answer taken as one.** An `EACCES`
        // or an `EIO` is a machine that could not tell, and reading it as
        // absence is `AGENTS.md` §7's defect exactly.
        if error.raw_os_error() == Some(libc::ENOENT) {
            return Ok(None);
        }
        return Err(Fault {
            path: path.to_string(),
            errno: Errno::from_io(&error),
            doing: "opening",
        });
    }
    // SAFETY: a descriptor `open` just returned and nothing else holds.
    #[allow(unsafe_code)]
    Ok(Some(unsafe {
        <std::os::fd::OwnedFd as std::os::fd::FromRawFd>::from_raw_fd(fd)
    }))
}

/// **Apply the ruleset to THIS thread and everything it execs.** Irreversible.
///
/// # Safety
///
/// The caller must be the child of a `fork`, before `exec`, and must already
/// have set `PR_SET_NO_NEW_PRIVS` — without it the kernel answers `EPERM` for
/// any caller lacking `CAP_SYS_ADMIN`.
///
/// Async-signal-safe: one syscall, no allocation, no lock, no libc state.
#[allow(unsafe_code)]
pub unsafe fn restrict_self(ruleset_fd: std::os::fd::RawFd) -> std::io::Result<()> {
    // SAFETY: `ruleset_fd` is a Landlock ruleset descriptor this process holds.
    let rc = unsafe { libc::syscall(SYS_RESTRICT_SELF, ruleset_fd, 0_u32) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn every_rivet_verb_maps_onto_at_least_one_kernel_bit() {
        for verb in [
            Access::READ,
            Access::LIST,
            Access::EXECUTE,
            Access::WRITE,
            Access::CREATE,
            Access::DELETE,
            Access::TRUNCATE,
            Access::REPARENT,
            Access::DEVICE_CONTROL,
        ] {
            assert_ne!(
                bits_of(verb),
                0,
                "{:?} maps to nothing, so a policy asking for it would confine nothing",
                verb.words()
            );
        }
    }

    /// The verb says *"never a device node"*; this is the assertion that the
    /// mapping keeps the promise.
    #[test]
    fn create_never_reaches_a_device_node() {
        let mask = bits_of(Access::CREATE);
        assert_eq!(mask & FS_MAKE_CHAR, 0);
        assert_eq!(mask & FS_MAKE_BLOCK, 0);
        assert_ne!(mask & FS_MAKE_REG, 0);
    }

    /// A handled set with a bit the kernel does not know is `EINVAL` at
    /// creation, so the ABI gate is the difference between a ruleset and no
    /// ruleset at all.
    #[test]
    fn the_handled_set_grows_with_the_abi_and_is_empty_without_one() {
        assert_eq!(handled_for(0), 0);
        assert_eq!(handled_for(1) & FS_REFER, 0);
        assert_ne!(handled_for(2) & FS_REFER, 0);
        assert_eq!(handled_for(2) & FS_TRUNCATE, 0);
        assert_ne!(handled_for(3) & FS_TRUNCATE, 0);
        assert_eq!(handled_for(4) & FS_IOCTL_DEV, 0);
        assert_ne!(handled_for(5) & FS_IOCTL_DEV, 0);
        assert_eq!(handled_for(6), handled_for(5));
    }

    #[test]
    fn what_a_kernel_cannot_enforce_is_named_rather_than_counted() {
        assert_eq!(
            unhandled_words(1),
            vec!["reparent", "truncate", "device-control"]
        );
        assert_eq!(unhandled_words(3), vec!["device-control"]);
        assert!(unhandled_words(5).is_empty());
    }

    /// `handled_access_net_for` is empty below ABI 4 and exactly `BIND_TCP` —
    /// never `CONNECT_TCP` — from ABI 4 on. `ADR-042` §7 is the argument for
    /// why the second half of that sentence does not change on a newer
    /// kernel: this ruleset does not grow a right just because the kernel
    /// gained one.
    #[test]
    fn the_handled_net_set_is_bind_tcp_alone_from_abi_4() {
        assert_eq!(handled_access_net_for(0), 0);
        assert_eq!(handled_access_net_for(3), 0);
        assert_eq!(handled_access_net_for(4), NET_BIND_TCP);
        assert_eq!(handled_access_net_for(6), NET_BIND_TCP);
        // Bit 1 is `LANDLOCK_ACCESS_NET_CONNECT_TCP`. Asserted absent by
        // number rather than by a named constant, because this module
        // deliberately keeps no constant for it — see the comment beside
        // `NET_BIND_TCP`.
        assert_eq!(handled_access_net_for(6) & (1 << 1), 0);
    }

    /// `struct landlock_path_beneath_attr` is packed and twelve bytes. A
    /// `#[repr(C)]` would be sixteen and the kernel would read `parent_fd` out
    /// of padding — a rule for descriptor 0, on every call, silently.
    #[test]
    fn the_rule_struct_is_the_shape_the_kernel_declares() {
        assert_eq!(std::mem::size_of::<PathBeneath>(), 12);
    }

    /// `struct landlock_ruleset_attr` on an ABI with `handled_access_net` is
    /// three `u64`s: twenty-four bytes, none of it padding.
    #[test]
    fn the_net_ruleset_attr_is_three_u64s() {
        assert_eq!(std::mem::size_of::<RulesetAttrNet>(), 24);
    }

    /// Try the two opens in a forked child, optionally confined first, and
    /// report what the kernel said as an exit code.
    ///
    /// **The child does nothing but syscalls and `_exit`.** It is between
    /// `fork` and nothing — there is no `exec` here — so allocating or taking a
    /// lock could deadlock against whatever the test harness's other threads
    /// held at the moment of the fork.
    fn probe(ruleset: Option<&Ruleset>, inside: &std::ffi::CString, outside: &std::ffi::CString) -> (bool, bool) {
        let fd = ruleset.map_or(-1, Ruleset::raw);
        #[allow(unsafe_code)]
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0, "fork: {}", std::io::Error::last_os_error());
        if pid == 0 {
            #[allow(unsafe_code)]
            unsafe {
                if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
                    libc::_exit(90);
                }
                if fd >= 0 && restrict_self(fd).is_err() {
                    libc::_exit(91);
                }
                let a = libc::open(inside.as_ptr(), libc::O_WRONLY | libc::O_CREAT, 0o600);
                let b = libc::open(outside.as_ptr(), libc::O_RDONLY);
                // Bit 0: the grant worked. Bit 1: the escape worked.
                let mut code = 0;
                if a >= 0 {
                    code |= 1;
                }
                if b >= 0 {
                    code |= 2;
                }
                libc::_exit(code);
            }
        }
        let mut status = 0;
        #[allow(unsafe_code)]
        unsafe {
            libc::waitpid(pid, &raw mut status, 0);
        }
        assert!(
            libc::WIFEXITED(status),
            "the probe child did not exit normally: {status}"
        );
        let code = libc::WEXITSTATUS(status);
        assert!(code < 4, "the probe child failed before it opened anything: {code}");
        (code & 1 != 0, code & 2 != 0)
    }

    /// **The proof, and both halves of it** — `ADR-042` §6.
    ///
    /// One binary, one boot, one uid, two children: the confined one is
    /// refused a file the unconfined one opens, and both write the file the
    /// policy grants. A test that only showed the refusal could not tell a
    /// working ruleset from a path that is not there, which is why the
    /// unconfined run is the same code and the same paths.
    #[test]
    fn a_confined_child_is_refused_a_path_an_unconfined_one_opens() {
        let (version, errno) = abi();
        assert!(
            version >= MINIMUM_ABI,
            "this kernel reports no Landlock (errno {errno}), so this test cannot run. It is not \
             skipped: a machine that cannot enforce ADR-042 is one where every application is \
             refused, and a green suite here would say otherwise"
        );

        let root = std::env::temp_dir().join(format!("rivet-landlock-{}", std::process::id()));
        let allowed = root.join("allowed");
        std::fs::create_dir_all(&allowed).expect("a temporary directory");
        let outside_path = root.join("outside");
        std::fs::write(&outside_path, b"reachable before the ruleset\n").expect("a file to read");

        let inside = std::ffi::CString::new(allowed.join("written").to_string_lossy().as_bytes())
            .expect("a path with no NUL");
        let outside = std::ffi::CString::new(outside_path.to_string_lossy().as_bytes())
            .expect("a path with no NUL");

        let mut policy = Policy::default();
        policy.allow(
            allowed.to_string_lossy().into_owned(),
            Access::READ
                .with(Access::LIST)
                .with(Access::WRITE)
                .with(Access::CREATE),
            Presence::Required,
            "the one directory this test grants",
        );
        let ruleset = build(&policy, version).expect("a ruleset");

        let (free_grant, free_escape) = probe(None, &inside, &outside);
        assert!(free_grant, "the unconfined child could not write the granted path");
        assert!(
            free_escape,
            "the unconfined child could not read the path outside the grant either, so the \
             confined run below would prove nothing about Landlock"
        );

        let (held_grant, held_escape) = probe(Some(&ruleset), &inside, &outside);
        assert!(
            held_grant,
            "the ruleset broke the application: it could not write the directory its policy grants"
        );
        assert!(
            !held_escape,
            "THE RULESET ENFORCED NOTHING: the confined child opened a path outside its policy, \
             at the same uid, in the same boot, that the unconfined child also opened"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// Try to `bind` a loopback TCP socket in a forked child, optionally
    /// confined first, and report what the kernel said as an exit code.
    ///
    /// **Raw syscalls, for [`probe`]'s reason.** Between `fork` and nothing —
    /// there is no `exec` here — allocating could deadlock against a lock
    /// another thread of the test binary held at the moment of the fork.
    /// `socket`/`bind` on a stack-built `sockaddr_in` need none.
    fn probe_bind(ruleset: Option<&Ruleset>) -> bool {
        let fd = ruleset.map_or(-1, Ruleset::raw);
        #[allow(unsafe_code)]
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0, "fork: {}", std::io::Error::last_os_error());
        if pid == 0 {
            #[allow(unsafe_code)]
            unsafe {
                if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
                    libc::_exit(90);
                }
                if fd >= 0 && restrict_self(fd).is_err() {
                    libc::_exit(91);
                }
                let sock = libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0);
                if sock < 0 {
                    libc::_exit(92);
                }
                let mut addr: libc::sockaddr_in = std::mem::zeroed();
                addr.sin_family = libc::AF_INET as libc::sa_family_t;
                // Port 0: an ephemeral port. `BIND_TCP` is checked against the
                // `bind` call itself, not against which port was asked for, so
                // which one the kernel would have handed back is irrelevant to
                // what this proves.
                addr.sin_port = 0;
                addr.sin_addr.s_addr = u32::from(std::net::Ipv4Addr::LOCALHOST).to_be();
                let rc = libc::bind(
                    sock,
                    std::ptr::from_ref(&addr).cast::<libc::sockaddr>(),
                    std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
                );
                // Bit 0: the bind succeeded.
                libc::_exit(i32::from(rc == 0));
            }
        }
        let mut status = 0;
        #[allow(unsafe_code)]
        unsafe {
            libc::waitpid(pid, &raw mut status, 0);
        }
        assert!(
            libc::WIFEXITED(status),
            "the bind probe child did not exit normally: {status}"
        );
        let code = libc::WEXITSTATUS(status);
        assert!(code < 2, "the bind probe child failed before it could bind at all: {code}");
        code == 1
    }

    /// **The network half of the proof — `ADR-042` §7.** Same two-children
    /// shape as [`a_confined_child_is_refused_a_path_an_unconfined_one_opens`]
    /// immediately above: one process, one uid, and only the second child
    /// restricted, so a bind that succeeds unconfined and fails confined
    /// cannot be explained by anything but the ruleset.
    #[test]
    fn a_confined_child_cannot_bind_a_tcp_port_an_unconfined_one_can() {
        let (version, errno) = abi();
        assert!(
            version >= MINIMUM_ABI,
            "this kernel reports no Landlock (errno {errno}), so this test cannot run"
        );
        if version < NET_RIGHTS_ABI {
            // `handled_access_net_for` returns 0 below ABI 4 — there is no
            // network right for a kernel this old to enforce, so a confined
            // child here would be confined only on the filesystem, which the
            // test above already proves. Not a skip that hides a failure:
            // the assertion this test makes has nothing to bind against.
            println!(
                "landlock ABI {version} predates network rights (needs {NET_RIGHTS_ABI}); \
                 nothing to prove here"
            );
            return;
        }

        // An empty policy: no filesystem grants at all. This test is about
        // the network right alone, and a grant here would be an unrelated
        // variable in the same run.
        let policy = Policy::default();
        let ruleset = build(&policy, version).expect("a ruleset with no filesystem grants");

        assert!(
            probe_bind(None),
            "the unconfined child could not bind a loopback TCP port at all, so the confined run \
             below would prove nothing about Landlock"
        );
        assert!(
            !probe_bind(Some(&ruleset)),
            "THE RULESET DID NOT REFUSE A TCP BIND: a confined child bound a loopback port, at \
             the same uid, in the same process, that the unconfined child also bound"
        );
    }

    /// Run `program --version` in a forked child, optionally confined first.
    /// `Ok(())` when it ran and exited zero; otherwise the child's exit code,
    /// which is `100 + errno` when `execve` itself was refused.
    ///
    /// **Everything the child touches is built before the fork**, for
    /// [`probe`]'s reason.
    fn probe_exec(ruleset: Option<&Ruleset>, program: &std::ffi::CStr) -> Result<(), i32> {
        let fd = ruleset.map_or(-1, Ruleset::raw);
        let flag = c"--version";
        let argv: [*const libc::c_char; 3] = [program.as_ptr(), flag.as_ptr(), std::ptr::null()];
        let envp: [*const libc::c_char; 1] = [std::ptr::null()];
        #[allow(unsafe_code)]
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0, "fork: {}", std::io::Error::last_os_error());
        if pid == 0 {
            #[allow(unsafe_code)]
            unsafe {
                if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
                    libc::_exit(90);
                }
                if fd >= 0 && restrict_self(fd).is_err() {
                    libc::_exit(91);
                }
                libc::execve(program.as_ptr(), argv.as_ptr(), envp.as_ptr());
                libc::_exit(100 + *libc::__errno_location());
            }
        }
        let mut status = 0;
        #[allow(unsafe_code)]
        unsafe {
            libc::waitpid(pid, &raw mut status, 0);
        }
        assert!(libc::WIFEXITED(status), "the exec probe child did not exit normally: {status}");
        match libc::WEXITSTATUS(status) {
            0 => Ok(()),
            code => Err(code),
        }
    }

    /// **`rivet-fleetd`, confined by the policy the supervisor gives it, is
    /// refused `git`** — and does not need it (`ADR-044`, `ADR-042`).
    ///
    /// Fleet-as-code's git runs in `rivet-mcpd`, as control-plane operations
    /// `rivet-fleetd` asks for, and the application sandbox is not widened to
    /// let an application run a system program. This test holds the second half:
    /// a policy row that granted execute on git's directory would turn it red.
    /// `cmd/rivet-fleetd`'s `no_source_file_in_this_crate_starts_a_process`
    /// holds the first.
    ///
    /// The policy is `rivet_core::confinement::policy` for the name
    /// `rivet-fleetd`, over a scratch volume. Its manifest's only input to that
    /// function is `library`, which `cmd/rivet-fleetd/Cargo.toml` declares
    /// empty, so a bare daemon manifest computes the same rows. The program is
    /// `rivet_core::git::BINARY` where it exists, and otherwise the first `git`
    /// on `PATH`; no row of the policy covers either. The unconfined child runs
    /// the same program, so the refusal is the ruleset and not a missing file.
    #[test]
    fn rivet_fleetd_confined_by_its_own_policy_cannot_run_git() {
        let (version, errno) = abi();
        assert!(
            version >= MINIMUM_ABI,
            "this kernel reports no Landlock (errno {errno}), so this test cannot run"
        );

        let git = if std::path::Path::new(rivet_core::git::BINARY).exists() {
            std::path::PathBuf::from(rivet_core::git::BINARY)
        } else {
            std::env::var_os("PATH")
                .and_then(|path| {
                    std::env::split_paths(&path)
                        .map(|dir| dir.join("git"))
                        .find(|candidate| candidate.is_file())
                })
                .expect("no git at rivet_core::git::BINARY or on PATH, so there is nothing to run")
        };
        println!("git under test: {}", git.display());

        let root = std::env::temp_dir().join(format!("rivet-landlock-git-{}", std::process::id()));
        let volume = root.join("state");
        let name = rivet_core::service::ServiceName::new("rivet-fleetd").expect("a service name");
        let manifest = rivet_core::package::application::Manifest::new(
            rivet_core::package::application::ProgramKind::Daemon,
        );
        assert!(manifest.library().is_empty());
        let policy =
            rivet_core::confinement::policy(&volume.to_string_lossy(), &name, &manifest, None, &[]);
        for grant in &policy.grants {
            if grant.presence == Presence::Required {
                std::fs::create_dir_all(&grant.path).expect("a required directory");
            }
        }
        let ruleset = build(&policy, version).expect("rivet-fleetd's ruleset");

        let program = std::ffi::CString::new(git.to_string_lossy().as_bytes()).expect("no NUL");
        assert_eq!(
            probe_exec(None, &program),
            Ok(()),
            "the unconfined child could not run {} either, so the confined run proves nothing",
            git.display()
        );
        let confined = probe_exec(Some(&ruleset), &program);
        std::fs::remove_dir_all(&root).ok();
        assert_eq!(
            confined,
            Err(100 + libc::EACCES),
            "confined by rivet-fleetd's own policy, {} was expected to be refused at execve with \
             EACCES, and the child exited {confined:?}. Ok(()) means the policy now grants an \
             application execute on a system program, which ADR-042 does not. Granted: {:?}",
            git.display(),
            ruleset.granted
        );
    }

    /// **This kernel, right now.** Not an assertion about a value — the
    /// container, the emulator and the board answer differently and all three
    /// are legitimate. What is asserted is that the probe *answers*: a negative
    /// return with a zero errno would be this function losing the difference
    /// between "no Landlock" and "the call failed" (`AGENTS.md` §7).
    #[test]
    fn the_abi_probe_answers() {
        let (version, errno) = abi();
        if version == 0 {
            assert_ne!(
                errno, 0,
                "abi() reported no Landlock and no reason, so nothing can tell a kernel without \
                 it from a probe that broke"
            );
        } else {
            assert_eq!(errno, 0);
            assert!(version >= MINIMUM_ABI);
        }
        println!("landlock ABI on this kernel: {version} (errno {errno})");
    }
}
