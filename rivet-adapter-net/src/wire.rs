//! rtnetlink's wire format: constants, encoding, and refusing a bad reply.
//!
//! **Nothing here makes a syscall, and nothing here is Linux-only.** That is
//! deliberate and it is half the reason this crate exists in the shape it does:
//! the format is bytes, so it compiles and its tests run on the Windows
//! development host, and only [`crate::socket`] needs a kernel. See the survey
//! in [`crate`]'s module documentation.
//!
//! # Where these numbers come from
//!
//! From the kernel's UAPI headers — `linux/netlink.h`, `linux/rtnetlink.h`,
//! `linux/if_link.h`, `linux/if_addr.h`, `linux/if.h` — and **not** from
//! `libc`. Two reasons, and the second is the load-bearing one:
//!
//! 1. They are wire-protocol constants rather than platform types. A value that
//!    appears in a byte stream sent to a kernel belongs with the code that
//!    writes the byte stream.
//! 2. **`libc`'s values are the *host's*, and the host is not the target.**
//!    `libc::EEXIST` on Windows is 17 by coincidence and `libc::AF_INET` is 2
//!    by coincidence; neither is guaranteed and `AF_NETLINK` does not exist
//!    there at all. Taking them from `libc` would make this module's meaning
//!    depend on which machine compiled it.
//!
//! Point 2 is a claim that can go stale, so it is not left as prose:
//! `tests::the_constants_match_the_kernel_headers` is compiled on Linux only
//! and asserts every constant here against `libc`'s. `AGENTS.md` §4 —
//! a suite that cannot be run rots, so the guard runs wherever it can.
//!
//! # Byte order
//!
//! **Netlink is native-endian**, unlike almost every other protocol. The
//! headers and the fixed-layout payload structs are written with
//! `to_ne_bytes`. The exception is the *contents* of an address attribute:
//! `IFA_ADDRESS`, `IFA_LOCAL`, `IFA_BROADCAST` and `RTA_GATEWAY` carry a
//! `struct in_addr`, which is network byte order — which is to say the four
//! octets in the order they are written down, exactly what
//! [`Ipv4Addr::octets`](std::net::Ipv4Addr::octets) returns. So there is no
//! conversion, and that is worth stating because reaching for `to_be_bytes`
//! there produces an address that is right on one architecture and reversed on
//! the other.

// ---------------------------------------------------------------------------
// Alignment
// ---------------------------------------------------------------------------

/// Netlink aligns every message and every attribute to four bytes.
pub const NLMSG_ALIGNTO: usize = 4;
/// `struct nlmsghdr`: len, type, flags, seq, pid.
pub const NLMSG_HDRLEN: usize = 16;
/// `struct rtattr`: len, type.
pub const RTA_HDRLEN: usize = 4;

/// Round up to the next four-byte boundary.
///
/// Saturating rather than wrapping: a length near `usize::MAX` is nonsense, and
/// wrapping it to a small number would turn a refusal into a buffer walk.
pub const fn nlmsg_align(len: usize) -> usize {
    len.saturating_add(NLMSG_ALIGNTO - 1) & !(NLMSG_ALIGNTO - 1)
}

// ---------------------------------------------------------------------------
// Message types and flags — linux/netlink.h, linux/rtnetlink.h
// ---------------------------------------------------------------------------

pub const NLMSG_NOOP: u16 = 1;
/// Carries a `struct nlmsgerr`. **An error code of zero is an acknowledgement,
/// not a failure** — see [`error_code`].
pub const NLMSG_ERROR: u16 = 2;
pub const NLMSG_DONE: u16 = 3;
/// The kernel dropped messages because the receive buffer filled.
pub const NLMSG_OVERRUN: u16 = 4;

pub const RTM_NEWLINK: u16 = 16;
pub const RTM_GETLINK: u16 = 18;
pub const RTM_NEWADDR: u16 = 20;
/// Withdraw one address from an interface.
pub const RTM_DELADDR: u16 = 21;
/// Ask the kernel which addresses are assigned. **The reply is `RTM_NEWADDR`**,
/// not a `RTM_GETADDR` — netlink answers a `GET` with the `NEW` of the same
/// object, which is why [`RTM_NEWADDR`] appears on both sides of this crate.
pub const RTM_GETADDR: u16 = 22;
pub const RTM_NEWROUTE: u16 = 24;
/// Withdraw one route.
pub const RTM_DELROUTE: u16 = 25;
/// Ask the kernel which routes exist. Answered with [`RTM_NEWROUTE`], as
/// [`RTM_GETADDR`] is answered with [`RTM_NEWADDR`].
pub const RTM_GETROUTE: u16 = 26;

pub const NLM_F_REQUEST: u16 = 0x0001;
/// **The kernel set this, not us.** One reply out of many; the sequence ends
/// with [`NLMSG_DONE`]. Read rather than sent — a request never carries it.
pub const NLM_F_MULTI: u16 = 0x0002;
pub const NLM_F_ACK: u16 = 0x0004;
/// Create the object if it does not exist.
pub const NLM_F_CREATE: u16 = 0x0400;
/// Fail with `EEXIST` if it does. **Deliberately paired with
/// [`NLM_F_CREATE`]** everywhere in this crate: without it the kernel replaces
/// silently, and "the address was already there" and "we just overwrote
/// somebody's address" become the same answer.
///
/// **This is the same bit as [`NLM_F_MATCH`]**, and that is Linux's doing
/// rather than a mistake here: `linux/netlink.h` overloads the flag word by
/// request *kind*, so `0x0200` means "fail if it exists" on a `NEW` and "filter
/// by the payload" on a `GET`. Both names exist so that each call site reads as
/// what it is asking for; using the wrong one compiles and is wrong.
pub const NLM_F_EXCL: u16 = 0x0200;

/// Return the whole table rather than one entry. Half of [`NLM_F_DUMP`].
pub const NLM_F_ROOT: u16 = 0x0100;
/// Return every entry matching the request's payload. The other half of
/// [`NLM_F_DUMP`]. See [`NLM_F_EXCL`] for why this bit has two names.
pub const NLM_F_MATCH: u16 = 0x0200;
/// **A dump.** The kernel answers with a sequence of messages, each flagged
/// [`NLM_F_MULTI`], spread over as many datagrams as it needs, and terminated
/// by [`NLMSG_DONE`]. `crate::port::transact` cannot read one — it reads a
/// single datagram — which is why `crate::port::dump` exists.
pub const NLM_F_DUMP: u16 = NLM_F_ROOT | NLM_F_MATCH;
/// **The kernel set this, not us**, same as [`NLM_F_MULTI`]. A dump that was
/// disturbed by a concurrent change on the kernel side — another netlink
/// client adding or removing what is being dumped, mid-dump — carries this on
/// whichever message reports it, which may be an ordinary data message or
/// [`NLMSG_DONE`] itself. `crate::port::dump` treats either as *this snapshot
/// cannot be trusted* rather than a shorter but otherwise valid answer.
pub const NLM_F_DUMP_INTR: u16 = 0x10;

// ---------------------------------------------------------------------------
// Address families and netlink protocols
// ---------------------------------------------------------------------------

pub const AF_UNSPEC: u8 = 0;
pub const AF_INET: u8 = 2;
/// The socket family, as passed to `socket(2)`. Wider type because that is the
/// argument's type.
pub const AF_NETLINK: i32 = 16;
/// The netlink protocol that carries routing and link messages.
pub const NETLINK_ROUTE: i32 = 0;

// ---------------------------------------------------------------------------
// Link flags — linux/if.h
// ---------------------------------------------------------------------------

/// Administratively up: the operator asked for it.
pub const IFF_UP: u32 = 0x0001;
/// Operationally up: the kernel believes it can carry traffic.
pub const IFF_RUNNING: u32 = 0x0040;
/// The physical link has a carrier. `ADR-004` §8.2 read exactly this on the
/// board — `<BROADCAST,MULTICAST,UP,LOWER_UP>` — and it is the flag that
/// distinguishes "brought up" from "plugged in".
pub const IFF_LOWER_UP: u32 = 0x1_0000;

// ---------------------------------------------------------------------------
// Attribute identifiers
// ---------------------------------------------------------------------------

/// `IFLA_IFNAME` — the interface's name, NUL-terminated.
pub const IFLA_IFNAME: u16 = 3;

/// `IFA_ADDRESS` — the peer address on a point-to-point link, and the local
/// address everywhere else. Sent alongside [`IFA_LOCAL`], which is what
/// `iproute2` does and what makes the two agree on an ordinary link.
pub const IFA_ADDRESS: u16 = 1;
pub const IFA_LOCAL: u16 = 2;
pub const IFA_BROADCAST: u16 = 4;

pub const RTA_OIF: u16 = 4;
pub const RTA_GATEWAY: u16 = 5;
/// The route's metric. Part of what the kernel matches a deletion on, so a
/// route read back with one is deleted with it.
pub const RTA_PRIORITY: u16 = 6;
/// The routing table as a 32-bit number. The kernel sends it beside
/// `rtm_table`, which is a byte and reads `RT_TABLE_COMPAT` for a table above
/// 255, so this is the one to believe when both are present.
pub const RTA_TABLE: u16 = 15;

// ---------------------------------------------------------------------------
// Route constants — linux/rtnetlink.h
// ---------------------------------------------------------------------------

/// The table `ip route show` prints without being asked.
pub const RT_TABLE_MAIN: u8 = 254;
/// The scopes `ifa_scope` and `rtm_scope` are drawn from — `linux/rtnetlink.h`.
///
/// **These are NOT the numbers `/proc/net/if_inet6` prints**, and the two are
/// easy to confuse because both are called "scope" and both are small. `ip`
/// renders `link` from `253` here and from `0x20` there. A reader that mapped
/// one table's numbers with the other's names would print `global` for a
/// link-local address, which is the class of failure `AGENTS.md` §5 is about —
/// it does not fail, it succeeds with a plausible wrong word.
pub const RT_SCOPE_SITE: u8 = 200;
/// Valid only on this link — a `169.254.0.0/16` address, or the peer end of a
/// point-to-point link.
pub const RT_SCOPE_LINK: u8 = 253;
/// Valid only on this machine — `127.0.0.1` and everything else on `lo`.
pub const RT_SCOPE_HOST: u8 = 254;
/// The address is being removed. Reported as the number rather than as a word,
/// because a caller seeing it is seeing a race rather than a configuration.
pub const RT_SCOPE_NOWHERE: u8 = 255;
/// Who installed the route. `RTPROT_STATIC` says "an administrator", which is
/// what Rivet is here — and it is what makes a route this crate installed
/// distinguishable from one a DHCP client installed when both exist.
pub const RTPROT_STATIC: u8 = 4;
/// The destination is more than one hop away — correct for a default route via
/// a gateway.
pub const RT_SCOPE_UNIVERSE: u8 = 0;
pub const RTN_UNICAST: u8 = 1;

/// `struct ifinfomsg`: family, pad, type, index, flags, change.
pub const IFINFOMSG_LEN: usize = 16;
/// `struct ifaddrmsg`: family, prefixlen, flags, scope, index.
pub const IFADDRMSG_LEN: usize = 8;
/// `struct rtmsg`: family, dst_len, src_len, tos, table, protocol, scope, type,
/// flags.
pub const RTMSG_LEN: usize = 12;

// ---------------------------------------------------------------------------
// Building a request
// ---------------------------------------------------------------------------

/// One netlink message under construction.
///
/// The length field cannot be known until the attributes are on, so it is
/// written last by [`finish`](Self::finish) rather than tracked as the caller
/// goes. A builder that asked the caller to supply a length would be asking for
/// the one field the kernel checks hardest.
#[derive(Debug, Clone)]
pub struct Message {
    bytes: Vec<u8>,
}

impl Message {
    pub fn new(kind: u16, flags: u16, seq: u32) -> Self {
        let mut bytes = Vec::with_capacity(64);
        // nlmsg_len, patched by `finish`.
        bytes.extend_from_slice(&0u32.to_ne_bytes());
        bytes.extend_from_slice(&kind.to_ne_bytes());
        bytes.extend_from_slice(&flags.to_ne_bytes());
        bytes.extend_from_slice(&seq.to_ne_bytes());
        // nlmsg_pid. Zero means "the kernel", which is the correct value in a
        // request: it is the *destination*, not the sender's identity.
        bytes.extend_from_slice(&0u32.to_ne_bytes());
        Self { bytes }
    }

    /// Append the fixed-layout struct that follows the header.
    pub fn push_payload(&mut self, payload: &[u8]) {
        self.bytes.extend_from_slice(payload);
        self.pad();
    }

    /// Append one `rtattr`.
    ///
    /// The attribute's own length excludes the trailing padding — that is what
    /// `RTA_LENGTH` means — but the *next* attribute starts aligned, so the
    /// padding goes on afterwards and is not counted.
    pub fn push_attr(&mut self, kind: u16, payload: &[u8]) {
        let len = RTA_HDRLEN.saturating_add(payload.len());
        self.bytes
            .extend_from_slice(&u16::try_from(len).unwrap_or(u16::MAX).to_ne_bytes());
        self.bytes.extend_from_slice(&kind.to_ne_bytes());
        self.bytes.extend_from_slice(payload);
        self.pad();
    }

    fn pad(&mut self) {
        let padded = nlmsg_align(self.bytes.len());
        self.bytes.resize(padded, 0);
    }

    /// Write the length and hand over the bytes.
    pub fn finish(mut self) -> Vec<u8> {
        let len = u32::try_from(self.bytes.len()).unwrap_or(u32::MAX);
        let header = self.bytes.get_mut(..4);
        if let Some(header) = header {
            header.copy_from_slice(&len.to_ne_bytes());
        }
        self.bytes
    }
}

// ---------------------------------------------------------------------------
// Reading a reply
// ---------------------------------------------------------------------------

/// A reply that does not parse.
///
/// `AGENTS.md` §5 — *refuse rather than guess*, stated there for exactly this
/// case: a wrong guess about a kernel's answer does not fail, it *succeeds*,
/// and reports a plausible link state that something upstream will act on. Each
/// variant carries the numbers involved so the console line says what was
/// actually received rather than that something was wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Malformed {
    /// Fewer than [`NLMSG_HDRLEN`] bytes remain.
    ShortHeader { available: usize },
    /// `nlmsg_len` claims a message smaller than its own header. Taken
    /// literally this would advance the cursor backwards or not at all, so it
    /// is refused rather than clamped — clamping is how a malformed reply
    /// becomes an infinite loop.
    LengthBelowHeader { nlmsg_len: u32 },
    /// `nlmsg_len` claims more bytes than were received.
    LengthBeyondBuffer { nlmsg_len: u32, available: usize },
    /// The fixed-layout struct after the header is short.
    ShortPayload { want: usize, got: usize },
    /// An `rtattr` header does not fit.
    ShortAttribute { available: usize },
    /// `rta_len` is below [`RTA_HDRLEN`]. Same reasoning as
    /// [`LengthBelowHeader`](Self::LengthBelowHeader).
    AttributeLengthBelowHeader { rta_len: u16 },
    /// `rta_len` runs off the end of the message.
    AttributeBeyondMessage { rta_len: u16, available: usize },
}

impl core::fmt::Display for Malformed {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ShortHeader { available } => write!(
                f,
                "a netlink reply ended after {available} bytes, before a {NLMSG_HDRLEN}-byte header"
            ),
            Self::LengthBelowHeader { nlmsg_len } => write!(
                f,
                "a netlink message declares nlmsg_len={nlmsg_len}, below its own {NLMSG_HDRLEN}-byte header"
            ),
            Self::LengthBeyondBuffer {
                nlmsg_len,
                available,
            } => write!(
                f,
                "a netlink message declares nlmsg_len={nlmsg_len} but only {available} bytes were received"
            ),
            Self::ShortPayload { want, got } => {
                write!(f, "a netlink payload needs {want} bytes and carries {got}")
            }
            Self::ShortAttribute { available } => write!(
                f,
                "an attribute list ended after {available} bytes, before a {RTA_HDRLEN}-byte header"
            ),
            Self::AttributeLengthBelowHeader { rta_len } => write!(
                f,
                "an attribute declares rta_len={rta_len}, below its own {RTA_HDRLEN}-byte header"
            ),
            Self::AttributeBeyondMessage { rta_len, available } => write!(
                f,
                "an attribute declares rta_len={rta_len} with {available} bytes left in the message"
            ),
        }
    }
}

/// One message out of a datagram, borrowed rather than copied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageRef<'a> {
    pub kind: u16,
    pub flags: u16,
    pub seq: u32,
    /// The port that sent it. Zero is the kernel.
    pub port: u32,
    /// Everything after the header, with the message's own padding trimmed.
    pub payload: &'a [u8],
}

/// Split a received buffer into messages.
///
/// A netlink datagram is a *sequence* of messages, not one, and a dump is a
/// sequence spread over several datagrams. Treating the first message as the
/// whole answer is the classic netlink defect: it works on every reply that
/// happens to be short.
pub fn messages(buffer: &[u8]) -> Result<Vec<MessageRef<'_>>, Malformed> {
    let mut out = Vec::new();
    let mut rest = buffer;
    while !rest.is_empty() {
        // Trailing alignment padding after the last message is not an error.
        if rest.len() < NLMSG_HDRLEN {
            if rest.iter().all(|b| *b == 0) {
                break;
            }
            return Err(Malformed::ShortHeader {
                available: rest.len(),
            });
        }
        let nlmsg_len = read_u32(rest, 0)?;
        let len = usize::try_from(nlmsg_len).unwrap_or(usize::MAX);
        if len < NLMSG_HDRLEN {
            return Err(Malformed::LengthBelowHeader { nlmsg_len });
        }
        if len > rest.len() {
            return Err(Malformed::LengthBeyondBuffer {
                nlmsg_len,
                available: rest.len(),
            });
        }
        let message = rest.get(..len).unwrap_or(&[]);
        out.push(MessageRef {
            kind: read_u16(message, 4)?,
            flags: read_u16(message, 6)?,
            seq: read_u32(message, 8)?,
            port: read_u32(message, 12)?,
            payload: message.get(NLMSG_HDRLEN..).unwrap_or(&[]),
        });
        // The next message starts aligned. `advance` is at least NLMSG_HDRLEN
        // by the check above, so this cannot fail to make progress.
        let advance = nlmsg_align(len).min(rest.len());
        rest = rest.get(advance..).unwrap_or(&[]);
    }
    Ok(out)
}

/// The `error` field of an `NLMSG_ERROR` message.
///
/// **Zero means success.** `NLM_F_ACK` asks the kernel to confirm a request it
/// accepted, and it confirms with an `NLMSG_ERROR` carrying zero — so a reader
/// that treats the message type as the verdict reports every success as a
/// failure. The value is otherwise a *negative* errno.
pub fn error_code(payload: &[u8]) -> Result<i32, Malformed> {
    let raw = read_u32(payload, 0).map_err(|_| Malformed::ShortPayload {
        want: 4,
        got: payload.len(),
    })?;
    Ok(raw as i32)
}

/// The completion status carried in `NLMSG_DONE`'s own payload.
///
/// Historically empty. A kernel new enough to report [`NLM_F_DUMP_INTR`] also
/// puts an error code here, in the same encoding [`error_code`] reads from
/// `NLMSG_ERROR`: negative is an errno, zero or positive is not an error.
///
/// Returns `Some` only for the negative case — the one [`crate::port::dump`]
/// must refuse with; an empty or non-negative payload is `None`, meaning the
/// dump completing is not itself in question.
///
/// # Errors
///
/// [`Malformed::ShortPayload`] for a payload of one to three bytes: too short
/// to hold the `i32` and too long to be "nothing was reported".
pub fn dump_done_error(payload: &[u8]) -> Result<Option<i32>, Malformed> {
    if payload.is_empty() {
        return Ok(None);
    }
    if payload.len() < 4 {
        return Err(Malformed::ShortPayload {
            want: 4,
            got: payload.len(),
        });
    }
    let code = read_u32(payload, 0)? as i32;
    Ok(if code < 0 { Some(code) } else { None })
}

/// What an `RTM_NEWLINK` reply says about an interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IfInfo {
    pub index: i32,
    pub flags: u32,
}

impl IfInfo {
    /// Administratively up — the state `RTM_NEWLINK` with `IFF_UP` produces.
    pub fn is_up(self) -> bool {
        self.flags & IFF_UP != 0
    }

    /// A carrier is present. Read but never *required*: `ADR-004` §2.3 makes
    /// the wired link the one that "cannot fail to associate", and a bring-up
    /// that refused on an unplugged cable would turn a missing cable into a
    /// boot failure rather than a reported fact.
    pub fn has_carrier(self) -> bool {
        self.flags & IFF_LOWER_UP != 0
    }
}

/// Decode the `struct ifinfomsg` that follows a link message's header.
pub fn parse_ifinfo(payload: &[u8]) -> Result<IfInfo, Malformed> {
    if payload.len() < IFINFOMSG_LEN {
        return Err(Malformed::ShortPayload {
            want: IFINFOMSG_LEN,
            got: payload.len(),
        });
    }
    Ok(IfInfo {
        // Offsets: family(1) pad(1) type(2) index(4) flags(4) change(4).
        index: read_u32(payload, 4)? as i32,
        flags: read_u32(payload, 8)?,
    })
}

/// What an `RTM_NEWADDR` reply says about one assigned address.
///
/// The counterpart of [`IfInfo`], and the reason this crate can now *read*
/// addresses rather than only assign them: `ADR-034` §3.3 promises a lost fleet
/// register can be rebuilt by asking the fleet, and a machine that cannot say
/// what address it has cannot honour that.
///
/// The address itself is **not** here. It arrives as an attribute — `IFA_LOCAL`
/// on every link that has one, `IFA_ADDRESS` otherwise — and which of the two
/// to believe is a decision rather than a decode, so it is made in
/// [`crate::address`] where it can be tested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IfAddr {
    /// [`AF_INET`] or `AF_INET6`. **Checked by the caller even when the request
    /// asked for one family**: `RTM_GETADDR` with `ifa_family = AF_INET` is a
    /// *filter*, and a kernel that ignored it would otherwise have its IPv6
    /// addresses read as four-byte IPv4 ones.
    pub family: u8,
    pub prefix_len: u8,
    pub flags: u8,
    /// One of the `RT_SCOPE_*` constants above, **not** an `if_inet6` scope.
    pub scope: u8,
    /// The interface this address is on. `/sys/class/net/<name>/ifindex` is the
    /// other end of this number, and is how a caller turns it back into a name.
    pub index: u32,
}

/// Decode the `struct ifaddrmsg` that follows an address message's header.
///
/// Offsets: `family(1) prefixlen(1) flags(1) scope(1) index(4)` — the same
/// eight bytes [`crate::address::assign`] writes, read back.
pub fn parse_ifaddr(payload: &[u8]) -> Result<IfAddr, Malformed> {
    if payload.len() < IFADDRMSG_LEN {
        return Err(Malformed::ShortPayload {
            want: IFADDRMSG_LEN,
            got: payload.len(),
        });
    }
    Ok(IfAddr {
        family: *payload.first().unwrap_or(&0),
        prefix_len: *payload.get(1).unwrap_or(&0),
        flags: *payload.get(2).unwrap_or(&0),
        scope: *payload.get(3).unwrap_or(&0),
        index: read_u32(payload, 4)?,
    })
}

/// Walk an attribute list.
///
/// Returns `(type, value)` pairs in the order the kernel sent them. Unknown
/// types are carried rather than dropped, because a caller that wants to report
/// what arrived cannot do so from a filtered list.
pub fn attributes(payload: &[u8]) -> Result<Vec<(u16, &[u8])>, Malformed> {
    let mut out = Vec::new();
    let mut rest = payload;
    while !rest.is_empty() {
        if rest.len() < RTA_HDRLEN {
            if rest.iter().all(|b| *b == 0) {
                break;
            }
            return Err(Malformed::ShortAttribute {
                available: rest.len(),
            });
        }
        let rta_len = read_u16(rest, 0)?;
        let len = usize::from(rta_len);
        if len < RTA_HDRLEN {
            return Err(Malformed::AttributeLengthBelowHeader { rta_len });
        }
        if len > rest.len() {
            return Err(Malformed::AttributeBeyondMessage {
                rta_len,
                available: rest.len(),
            });
        }
        let kind = read_u16(rest, 2)?;
        out.push((kind, rest.get(RTA_HDRLEN..len).unwrap_or(&[])));
        let advance = nlmsg_align(len).min(rest.len());
        rest = rest.get(advance..).unwrap_or(&[]);
    }
    Ok(out)
}

/// Read a native-endian `u32` at `offset`, refusing rather than panicking.
fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, Malformed> {
    let slice = bytes
        .get(offset..offset.saturating_add(4))
        .and_then(|s| <[u8; 4]>::try_from(s).ok())
        .ok_or(Malformed::ShortPayload {
            want: offset.saturating_add(4),
            got: bytes.len(),
        })?;
    Ok(u32::from_ne_bytes(slice))
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, Malformed> {
    let slice = bytes
        .get(offset..offset.saturating_add(2))
        .and_then(|s| <[u8; 2]>::try_from(s).ok())
        .ok_or(Malformed::ShortPayload {
            want: offset.saturating_add(2),
            got: bytes.len(),
        })?;
    Ok(u16::from_ne_bytes(slice))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    // -----------------------------------------------------------------------
    // Alignment
    // -----------------------------------------------------------------------

    #[test]
    fn alignment_rounds_up_to_four_and_leaves_multiples_alone() {
        assert_eq!(
            [0, 1, 2, 3, 4, 5, 16, 17].map(nlmsg_align),
            [0, 4, 4, 4, 4, 8, 16, 20]
        );
    }

    /// Saturating rather than wrapping. `usize::MAX` wrapped would align to 0,
    /// and a zero-length message is how a parser stops making progress.
    #[test]
    fn alignment_near_the_top_of_the_range_does_not_wrap_to_zero() {
        assert_ne!(nlmsg_align(usize::MAX), 0);
    }

    // -----------------------------------------------------------------------
    // Building
    // -----------------------------------------------------------------------

    #[test]
    fn a_bare_message_is_its_header_and_the_length_is_written_last() {
        let bytes = Message::new(RTM_GETLINK, NLM_F_REQUEST, 7).finish();
        assert_eq!(bytes.len(), NLMSG_HDRLEN);
        let parsed = messages(&bytes).expect("well-formed");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].kind, RTM_GETLINK);
        assert_eq!(parsed[0].flags, NLM_F_REQUEST);
        assert_eq!(parsed[0].seq, 7);
        assert_eq!(parsed[0].payload, b"");
        // The length field itself, read independently of `messages`.
        assert_eq!(u32::from_ne_bytes(bytes[..4].try_into().unwrap()), 16);
    }

    /// The padding case that actually occurs: `IFLA_IFNAME` for `eth0` is five
    /// bytes with its NUL, so `rta_len` is 9 and the next attribute — or the
    /// end of the message — is at 12.
    #[test]
    fn an_odd_length_attribute_is_padded_but_its_own_length_is_not() {
        let mut m = Message::new(RTM_GETLINK, NLM_F_REQUEST, 1);
        m.push_payload(&[0u8; IFINFOMSG_LEN]);
        m.push_attr(IFLA_IFNAME, b"eth0\0");
        let bytes = m.finish();

        assert_eq!(bytes.len(), NLMSG_HDRLEN + IFINFOMSG_LEN + 12);
        let parsed = messages(&bytes).expect("well-formed");
        let attrs = attributes(&parsed[0].payload[IFINFOMSG_LEN..]).expect("well-formed");
        assert_eq!(attrs, vec![(IFLA_IFNAME, &b"eth0\0"[..])]);
        // The declared length excludes the padding: 4 header + 5 name.
        let rta_len_at = NLMSG_HDRLEN + IFINFOMSG_LEN;
        assert_eq!(
            u16::from_ne_bytes(bytes[rta_len_at..rta_len_at + 2].try_into().unwrap()),
            9
        );
    }

    #[test]
    fn two_attributes_both_survive_the_padding_between_them() {
        let mut m = Message::new(RTM_NEWADDR, NLM_F_REQUEST, 1);
        m.push_payload(&[0u8; IFADDRMSG_LEN]);
        m.push_attr(IFLA_IFNAME, b"wlan0\0");
        m.push_attr(IFA_LOCAL, &[10, 0, 0, 1]);
        let bytes = m.finish();
        let parsed = messages(&bytes).expect("well-formed");
        let attrs = attributes(&parsed[0].payload[IFADDRMSG_LEN..]).expect("well-formed");
        assert_eq!(
            attrs,
            vec![(IFLA_IFNAME, &b"wlan0\0"[..]), (IFA_LOCAL, &[10, 0, 0, 1][..])]
        );
    }

    // -----------------------------------------------------------------------
    // Parsing: the refusals
    // -----------------------------------------------------------------------

    #[test]
    fn several_messages_in_one_datagram_are_all_returned() {
        let mut buffer = Message::new(RTM_NEWLINK, 0, 1).finish();
        buffer.extend(Message::new(NLMSG_DONE, 0, 2).finish());
        let parsed = messages(&buffer).expect("well-formed");
        assert_eq!(
            parsed.iter().map(|m| (m.kind, m.seq)).collect::<Vec<_>>(),
            [(RTM_NEWLINK, 1), (NLMSG_DONE, 2)]
        );
    }

    #[test]
    fn an_empty_buffer_holds_no_messages_and_is_not_an_error() {
        assert_eq!(messages(&[]), Ok(vec![]));
    }

    #[test]
    fn trailing_zero_padding_is_not_mistaken_for_a_truncated_message() {
        let mut buffer = Message::new(NLMSG_DONE, 0, 1).finish();
        buffer.extend_from_slice(&[0, 0, 0]);
        assert_eq!(messages(&buffer).map(|m| m.len()), Ok(1));
    }

    #[test]
    fn a_truncated_header_that_is_not_padding_is_refused() {
        assert_eq!(
            messages(&[1, 2, 3]),
            Err(Malformed::ShortHeader { available: 3 })
        );
    }

    /// The variant that matters most: a length of zero would leave the cursor
    /// where it is and spin for ever. It must be an error, not a clamp.
    #[test]
    fn a_message_length_below_its_own_header_is_refused_rather_than_clamped() {
        for claimed in [0u32, 1, 15] {
            let mut bytes = Message::new(NLMSG_DONE, 0, 1).finish();
            bytes[..4].copy_from_slice(&claimed.to_ne_bytes());
            assert_eq!(
                messages(&bytes),
                Err(Malformed::LengthBelowHeader {
                    nlmsg_len: claimed
                }),
                "nlmsg_len={claimed}"
            );
        }
    }

    #[test]
    fn a_message_claiming_more_bytes_than_arrived_is_refused() {
        let mut bytes = Message::new(NLMSG_DONE, 0, 1).finish();
        bytes[..4].copy_from_slice(&64u32.to_ne_bytes());
        assert_eq!(
            messages(&bytes),
            Err(Malformed::LengthBeyondBuffer {
                nlmsg_len: 64,
                available: 16
            })
        );
    }

    #[test]
    fn an_attribute_length_below_its_own_header_is_refused() {
        // rta_len = 2, rta_type = 3.
        let attrs = [2u8, 0, 3, 0];
        assert_eq!(
            attributes(&attrs),
            Err(Malformed::AttributeLengthBelowHeader { rta_len: 2 })
        );
    }

    #[test]
    fn an_attribute_running_past_the_end_of_the_message_is_refused() {
        let attrs = [200u8, 0, 3, 0, 0, 0, 0, 0];
        assert_eq!(
            attributes(&attrs),
            Err(Malformed::AttributeBeyondMessage {
                rta_len: 200,
                available: 8
            })
        );
    }

    #[test]
    fn a_truncated_attribute_header_that_is_not_padding_is_refused() {
        assert_eq!(
            attributes(&[9, 0, 3]),
            Err(Malformed::ShortAttribute { available: 3 })
        );
        // ...and one that *is* padding stops cleanly.
        assert_eq!(attributes(&[0, 0, 0]), Ok(vec![]));
    }

    // -----------------------------------------------------------------------
    // ifinfomsg and error codes
    // -----------------------------------------------------------------------

    #[test]
    fn an_ifinfomsg_yields_the_index_and_flags_at_the_right_offsets() {
        let mut payload = vec![0u8; IFINFOMSG_LEN];
        payload[4..8].copy_from_slice(&2i32.to_ne_bytes());
        payload[8..12].copy_from_slice(&(IFF_UP | IFF_LOWER_UP).to_ne_bytes());
        // A change mask that is all ones, so a reader taking the wrong field
        // would see 0xFFFFFFFF rather than a plausible flag word.
        payload[12..16].copy_from_slice(&u32::MAX.to_ne_bytes());

        let info = parse_ifinfo(&payload).expect("well-formed");
        assert_eq!(info.index, 2);
        assert!(info.is_up());
        assert!(info.has_carrier());
        assert_ne!(info.flags, u32::MAX, "ifi_change was read as ifi_flags");
    }

    #[test]
    fn a_short_ifinfomsg_is_refused_rather_than_zero_filled() {
        assert_eq!(
            parse_ifinfo(&[0u8; 8]),
            Err(Malformed::ShortPayload {
                want: IFINFOMSG_LEN,
                got: 8
            })
        );
    }

    #[test]
    fn a_down_interface_is_neither_up_nor_carrying() {
        let info = parse_ifinfo(&[0u8; IFINFOMSG_LEN]).expect("well-formed");
        assert!(!info.is_up());
        assert!(!info.has_carrier());
    }

    /// `wlan0` on the board was `UP` in `ADR-004` §8.1's reading with no
    /// association, so up-without-carrier is a state that actually occurs and
    /// the two flags must stay independent.
    #[test]
    fn up_and_carrier_are_independent_flags() {
        let mut payload = vec![0u8; IFINFOMSG_LEN];
        payload[8..12].copy_from_slice(&IFF_UP.to_ne_bytes());
        let info = parse_ifinfo(&payload).expect("well-formed");
        assert!(info.is_up());
        assert!(!info.has_carrier());
    }

    #[test]
    fn an_error_code_of_zero_is_an_acknowledgement() {
        assert_eq!(error_code(&0i32.to_ne_bytes()), Ok(0));
    }

    #[test]
    fn an_error_code_comes_back_negative_as_the_kernel_sent_it() {
        // -19 is ENODEV: the interface does not exist.
        assert_eq!(error_code(&(-19i32).to_ne_bytes()), Ok(-19));
    }

    #[test]
    fn an_error_message_with_no_payload_is_refused() {
        assert_eq!(
            error_code(&[]),
            Err(Malformed::ShortPayload { want: 4, got: 0 })
        );
    }

    // -- dump_done_error ----------------------------------------------------

    #[test]
    fn an_empty_done_payload_is_not_an_error() {
        assert_eq!(dump_done_error(&[]), Ok(None));
    }

    #[test]
    fn a_negative_done_payload_is_the_errno_to_refuse_with() {
        assert_eq!(dump_done_error(&(-19i32).to_ne_bytes()), Ok(Some(-19)));
    }

    #[test]
    fn a_zero_or_positive_done_payload_is_not_an_error() {
        assert_eq!(dump_done_error(&0i32.to_ne_bytes()), Ok(None));
        assert_eq!(dump_done_error(&4i32.to_ne_bytes()), Ok(None));
    }

    /// One to three bytes is long enough to be more than "nothing reported"
    /// and too short to hold the `i32` — refused rather than zero-extended,
    /// which would make a truncated reply read as success.
    #[test]
    fn a_done_payload_of_one_to_three_bytes_is_refused() {
        for len in 1..4 {
            assert_eq!(
                dump_done_error(&vec![0xAB; len]),
                Err(Malformed::ShortPayload { want: 4, got: len }),
                "len={len}"
            );
        }
    }

    #[test]
    fn every_refusal_renders_the_numbers_it_carries() {
        let cases = [
            (Malformed::ShortHeader { available: 3 }, "3"),
            (Malformed::LengthBelowHeader { nlmsg_len: 2 }, "2"),
            (
                Malformed::LengthBeyondBuffer {
                    nlmsg_len: 64,
                    available: 16,
                },
                "64",
            ),
            (Malformed::ShortPayload { want: 16, got: 8 }, "16"),
            (Malformed::ShortAttribute { available: 3 }, "3"),
            (Malformed::AttributeLengthBelowHeader { rta_len: 2 }, "2"),
            (
                Malformed::AttributeBeyondMessage {
                    rta_len: 200,
                    available: 8,
                },
                "200",
            ),
        ];
        for (error, expected) in cases {
            assert!(
                error.to_string().contains(expected),
                "{error:?} does not name {expected}"
            );
        }
    }

    /// The claim in this module's documentation — that these are the kernel's
    /// numbers and not the host's — checked against `libc` wherever `libc` can
    /// answer. `AGENTS.md` §4: a constant table that nothing checks has gone
    /// stale, so the guard runs on every cycle on Linux rather than being a
    /// sentence in a comment.
    ///
    /// It cannot run on the Windows development host, which is precisely the
    /// asymmetry that made hard-coding correct in the first place.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_constants_match_the_kernel_headers() {
        assert_eq!(i64::from(AF_NETLINK), i64::from(libc::AF_NETLINK));
        assert_eq!(i64::from(NETLINK_ROUTE), i64::from(libc::NETLINK_ROUTE));
        assert_eq!(u32::from(AF_UNSPEC), libc::AF_UNSPEC as u32);
        assert_eq!(u32::from(AF_INET), libc::AF_INET as u32);
        assert_eq!(IFF_UP, libc::IFF_UP as u32);
        assert_eq!(IFF_RUNNING, libc::IFF_RUNNING as u32);
        assert_eq!(IFF_LOWER_UP, libc::IFF_LOWER_UP as u32);
        assert_eq!(NLMSG_ERROR, libc::NLMSG_ERROR as u16);
        assert_eq!(NLMSG_DONE, libc::NLMSG_DONE as u16);
        assert_eq!(NLMSG_NOOP, libc::NLMSG_NOOP as u16);
        assert_eq!(NLMSG_OVERRUN, libc::NLMSG_OVERRUN as u16);
        assert_eq!(RTM_NEWLINK, libc::RTM_NEWLINK);
        assert_eq!(RTM_GETLINK, libc::RTM_GETLINK);
        assert_eq!(RTM_NEWADDR, libc::RTM_NEWADDR);
        assert_eq!(RTM_NEWROUTE, libc::RTM_NEWROUTE);
        assert_eq!(RTM_DELROUTE, libc::RTM_DELROUTE);
        assert_eq!(RTM_GETROUTE, libc::RTM_GETROUTE);
        assert_eq!(NLM_F_REQUEST, libc::NLM_F_REQUEST as u16);
        assert_eq!(NLM_F_ACK, libc::NLM_F_ACK as u16);
        assert_eq!(NLM_F_CREATE, libc::NLM_F_CREATE as u16);
        assert_eq!(NLM_F_EXCL, libc::NLM_F_EXCL as u16);
        assert_eq!(NLM_F_DUMP_INTR, libc::NLM_F_DUMP_INTR as u16);
        assert_eq!(IFA_ADDRESS, libc::IFA_ADDRESS);
        assert_eq!(IFA_LOCAL, libc::IFA_LOCAL);
        assert_eq!(IFA_BROADCAST, libc::IFA_BROADCAST);
        assert_eq!(RTA_OIF, libc::RTA_OIF);
        assert_eq!(RTA_GATEWAY, libc::RTA_GATEWAY);
        assert_eq!(RTA_PRIORITY, libc::RTA_PRIORITY);
        assert_eq!(RTA_TABLE, libc::RTA_TABLE);
        assert_eq!(IFLA_IFNAME, libc::IFLA_IFNAME);
        assert_eq!(RT_TABLE_MAIN, libc::RT_TABLE_MAIN);
        assert_eq!(RTPROT_STATIC, libc::RTPROT_STATIC);
        assert_eq!(RT_SCOPE_UNIVERSE, libc::RT_SCOPE_UNIVERSE);
        assert_eq!(RTN_UNICAST, libc::RTN_UNICAST);
        // The struct sizes the offsets above depend on.
        //
        // `rtmsg` is deliberately absent from this list: `libc` 0.2 does not
        // declare it, checked 2026-08-23 (`cannot find type rtmsg in crate
        // libc`). So [`RTMSG_LEN`] is the one size here with no independent
        // confirmation, and saying so is better than quietly omitting it. Its
        // nine `unsigned char` fields plus a `unsigned` are 12 bytes with no
        // padding on any architecture Rivet targets; the check that would
        // actually settle it is a route installed on a running kernel.
        assert_eq!(NLMSG_HDRLEN, core::mem::size_of::<libc::nlmsghdr>());
        assert_eq!(IFINFOMSG_LEN, core::mem::size_of::<libc::ifinfomsg>());
        assert_eq!(IFADDRMSG_LEN, core::mem::size_of::<libc::ifaddrmsg>());
        assert_eq!(RTA_HDRLEN, core::mem::size_of::<libc::rtattr>());
        assert_eq!(RTMSG_LEN, 12);
    }
}
