//! nf_tables' wire format: the batch, the objects, the expressions, and
//! reading a rule back. `ADR-023` §2.
//!
//! **Nothing here makes a syscall, and nothing here is Linux-only**, for the
//! reason [`crate::wire`] gives: the format is bytes, so it compiles and its
//! tests run on the development host, and only [`crate::socket`] needs a
//! kernel. No nftables library is used; the messages are written by hand
//! beside the rtnetlink ones, and `docs/design-candidates/Nftables-Library-Survey.md`
//! is why.
//!
//! # Where these numbers come from
//!
//! The kernel's UAPI headers — `linux/netfilter/nfnetlink.h`,
//! `linux/netfilter/nf_tables.h`, `linux/netfilter.h`,
//! `linux/netfilter/nf_conntrack_common.h` — and not `libc`, for
//! [`crate::wire`]'s second reason: they are the target kernel's numbers, not
//! the host's.
//!
//! # Byte order, which is NOT netlink's usual
//!
//! The netlink header and the attribute headers are native-endian as
//! everywhere else in this crate. **Every integer an nf_tables attribute
//! carries — a register, a key, an operator, an offset, a hook number, a
//! verdict — is big-endian**, because the kernel reads them with `ntohl`. The
//! values an expression compares against are different again: they are the
//! bytes the comparison sees in the register, so a port or an address is in
//! network order and a uid or a conntrack state is in the machine's own order.
//! Each builder below says which it writes.

use crate::wire::{self, Malformed, Message};

// ---------------------------------------------------------------------------
// The netlink protocol and nfnetlink's framing
// ---------------------------------------------------------------------------

/// The netlink protocol that carries netfilter's messages.
pub const NETLINK_NETFILTER: i32 = 12;

/// nf_tables' subsystem number. A message type is `subsystem << 8 | op`.
pub const NFNL_SUBSYS_NFTABLES: u16 = 10;
/// Opens a transaction. Everything up to [`NFNL_MSG_BATCH_END`] is applied
/// together or not at all.
pub const NFNL_MSG_BATCH_BEGIN: u16 = 0x10;
pub const NFNL_MSG_BATCH_END: u16 = 0x11;
/// `struct nfgenmsg`'s version field.
pub const NFNETLINK_V0: u8 = 0;
/// `struct nfgenmsg`: family, version, `res_id` (big-endian).
pub const NFGENMSG_LEN: usize = 4;

pub const NFT_MSG_NEWTABLE: u16 = 0;
pub const NFT_MSG_DELTABLE: u16 = 2;
pub const NFT_MSG_NEWCHAIN: u16 = 3;
pub const NFT_MSG_NEWRULE: u16 = 6;
pub const NFT_MSG_GETRULE: u16 = 7;

/// The message type for one nf_tables operation.
#[must_use]
pub const fn message_type(op: u16) -> u16 {
    (NFNL_SUBSYS_NFTABLES << 8) | op
}

/// Append to the end of a chain rather than its start.
pub const NLM_F_APPEND: u16 = 0x0800;

// ---------------------------------------------------------------------------
// Families, hooks, verdicts — linux/netfilter.h
// ---------------------------------------------------------------------------

pub const NFPROTO_UNSPEC: u8 = 0;
/// IPv4 and IPv6 in one table — `ADR-023` §7.
pub const NFPROTO_INET: u8 = 1;
pub const NFPROTO_IPV4: u8 = 2;
pub const NFPROTO_IPV6: u8 = 10;

/// Packets a local process sends. A connection attempt's first packet passes
/// here carrying the socket that made it, which is what lets a rule ask whose
/// socket it is.
pub const NF_INET_LOCAL_OUT: u32 = 3;

pub const NF_DROP: u32 = 0;
pub const NF_ACCEPT: u32 = 1;

pub const IPPROTO_TCP: u8 = 6;

// ---------------------------------------------------------------------------
// Attributes — linux/netfilter/nf_tables.h
// ---------------------------------------------------------------------------

/// Set on an attribute whose value is itself a list of attributes. The kernel
/// sets it on everything nested it sends, so a reader masks it off with
/// [`NLA_TYPE_MASK`].
pub const NLA_F_NESTED: u16 = 0x8000;
/// The type bits of an attribute's type field.
pub const NLA_TYPE_MASK: u16 = 0x3fff;

pub const NFTA_TABLE_NAME: u16 = 1;

pub const NFTA_CHAIN_TABLE: u16 = 1;
pub const NFTA_CHAIN_NAME: u16 = 3;
pub const NFTA_CHAIN_HOOK: u16 = 4;
pub const NFTA_CHAIN_POLICY: u16 = 5;
pub const NFTA_CHAIN_TYPE: u16 = 7;

pub const NFTA_HOOK_HOOKNUM: u16 = 1;
pub const NFTA_HOOK_PRIORITY: u16 = 2;

pub const NFTA_RULE_TABLE: u16 = 1;
pub const NFTA_RULE_CHAIN: u16 = 2;
pub const NFTA_RULE_HANDLE: u16 = 3;
pub const NFTA_RULE_EXPRESSIONS: u16 = 4;

pub const NFTA_LIST_ELEM: u16 = 1;

pub const NFTA_EXPR_NAME: u16 = 1;
pub const NFTA_EXPR_DATA: u16 = 2;

pub const NFTA_DATA_VALUE: u16 = 1;
pub const NFTA_DATA_VERDICT: u16 = 2;
pub const NFTA_VERDICT_CODE: u16 = 1;

pub const NFTA_IMMEDIATE_DREG: u16 = 1;
pub const NFTA_IMMEDIATE_DATA: u16 = 2;

pub const NFTA_META_DREG: u16 = 1;
pub const NFTA_META_KEY: u16 = 2;

pub const NFTA_CMP_SREG: u16 = 1;
pub const NFTA_CMP_OP: u16 = 2;
pub const NFTA_CMP_DATA: u16 = 3;

pub const NFTA_PAYLOAD_DREG: u16 = 1;
pub const NFTA_PAYLOAD_BASE: u16 = 2;
pub const NFTA_PAYLOAD_OFFSET: u16 = 3;
pub const NFTA_PAYLOAD_LEN: u16 = 4;

pub const NFTA_BITWISE_SREG: u16 = 1;
pub const NFTA_BITWISE_DREG: u16 = 2;
pub const NFTA_BITWISE_LEN: u16 = 3;
pub const NFTA_BITWISE_MASK: u16 = 4;
pub const NFTA_BITWISE_XOR: u16 = 5;

pub const NFTA_CT_DREG: u16 = 1;
pub const NFTA_CT_KEY: u16 = 2;

/// The register a rule's verdict is written to.
pub const NFT_REG_VERDICT: u32 = 0;
/// The first data register, 16 bytes wide — wide enough for an IPv6 address.
pub const NFT_REG_1: u32 = 1;

pub const NFT_META_SKUID: u32 = 10;
pub const NFT_META_NFPROTO: u32 = 15;
pub const NFT_META_L4PROTO: u32 = 16;

pub const NFT_CMP_EQ: u32 = 0;
pub const NFT_CMP_NEQ: u32 = 1;

pub const NFT_PAYLOAD_NETWORK_HEADER: u32 = 1;
pub const NFT_PAYLOAD_TRANSPORT_HEADER: u32 = 2;

pub const NFT_CT_STATE: u32 = 0;

/// `ct state new`'s bit. `NF_CT_STATE_BIT(IP_CT_NEW)` is `1 << (2 + 1)`: bit 0
/// is `invalid`, and each `ip_conntrack_info` value is shifted up by one.
pub const NF_CT_STATE_NEW: u32 = 1 << 3;

// ---------------------------------------------------------------------------
// Building
// ---------------------------------------------------------------------------

/// A list of attributes under construction, which may be nested inside
/// another.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Attrs {
    bytes: Vec<u8>,
}

impl Attrs {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// One attribute with `value` as its contents, padded so the next starts
    /// aligned. The declared length excludes the padding, as in
    /// [`Message::push_attr`].
    #[must_use]
    pub fn bytes(mut self, kind: u16, value: &[u8]) -> Self {
        let len = wire::RTA_HDRLEN.saturating_add(value.len());
        self.bytes
            .extend_from_slice(&u16::try_from(len).unwrap_or(u16::MAX).to_ne_bytes());
        self.bytes.extend_from_slice(&kind.to_ne_bytes());
        self.bytes.extend_from_slice(value);
        self.bytes.resize(wire::nlmsg_align(self.bytes.len()), 0);
        self
    }

    /// A NUL-terminated string, which is how nf_tables names everything.
    #[must_use]
    pub fn string(self, kind: u16, value: &str) -> Self {
        let mut with_nul = value.as_bytes().to_vec();
        with_nul.push(0);
        self.bytes(kind, &with_nul)
    }

    /// A big-endian `u32` — every integer nf_tables reads from an attribute.
    #[must_use]
    pub fn be32(self, kind: u16, value: u32) -> Self {
        self.bytes(kind, &value.to_be_bytes())
    }

    /// An attribute whose value is `inner`, flagged [`NLA_F_NESTED`].
    #[must_use]
    pub fn nest(self, kind: u16, inner: Self) -> Self {
        self.bytes(kind | NLA_F_NESTED, &inner.bytes)
    }

    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

/// `struct nfgenmsg`.
#[must_use]
pub fn nfgenmsg(family: u8, res_id: u16) -> [u8; NFGENMSG_LEN] {
    let id = res_id.to_be_bytes();
    [family, NFNETLINK_V0, id[0], id[1]]
}

/// One nf_tables object message: the header, `nfgenmsg` for `family`, and
/// `attrs`.
#[must_use]
pub fn object(op: u16, flags: u16, seq: u32, family: u8, attrs: Attrs) -> Vec<u8> {
    let mut m = Message::new(message_type(op), flags, seq);
    m.push_payload(&nfgenmsg(family, 0));
    m.push_payload(&attrs.into_bytes());
    m.finish()
}

/// A batch delimiter: `NFNL_MSG_BATCH_BEGIN` or `_END`, addressed to the
/// nf_tables subsystem through `res_id`.
#[must_use]
pub fn delimiter(kind: u16, seq: u32) -> Vec<u8> {
    let mut m = Message::new(kind, wire::NLM_F_REQUEST, seq);
    m.push_payload(&nfgenmsg(NFPROTO_UNSPEC, NFNL_SUBSYS_NFTABLES));
    m.finish()
}

/// One expression, as a rule carries it. Every load writes [`NFT_REG_1`] and
/// every test reads it, which is all a rule of tests-then-verdict needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    /// `meta` into the register.
    Meta { key: u32 },
    /// Compare the register's first `data.len()` bytes with `data`.
    Cmp { op: u32, data: Vec<u8> },
    /// Copy header bytes into the register.
    Payload { base: u32, offset: u32, len: u32 },
    /// `register = (register & mask) ^ xor`, over `mask.len()` bytes.
    Bitwise { mask: Vec<u8>, xor: Vec<u8> },
    /// Connection tracking into the register.
    Ct { key: u32 },
    /// The rule's verdict.
    Verdict { code: u32 },
}

impl Expr {
    /// The expression's name, as the kernel registers it.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Meta { .. } => "meta",
            Self::Cmp { .. } => "cmp",
            Self::Payload { .. } => "payload",
            Self::Bitwise { .. } => "bitwise",
            Self::Ct { .. } => "ct",
            Self::Verdict { .. } => "immediate",
        }
    }

    /// The `NFTA_LIST_ELEM` contents: the name and the nested data.
    #[must_use]
    pub fn encode(&self) -> Attrs {
        let data = match self {
            Self::Meta { key } => Attrs::new()
                .be32(NFTA_META_KEY, *key)
                .be32(NFTA_META_DREG, NFT_REG_1),
            Self::Cmp { op, data } => Attrs::new()
                .be32(NFTA_CMP_SREG, NFT_REG_1)
                .be32(NFTA_CMP_OP, *op)
                .nest(NFTA_CMP_DATA, Attrs::new().bytes(NFTA_DATA_VALUE, data)),
            Self::Payload { base, offset, len } => Attrs::new()
                .be32(NFTA_PAYLOAD_DREG, NFT_REG_1)
                .be32(NFTA_PAYLOAD_BASE, *base)
                .be32(NFTA_PAYLOAD_OFFSET, *offset)
                .be32(NFTA_PAYLOAD_LEN, *len),
            Self::Bitwise { mask, xor } => Attrs::new()
                .be32(NFTA_BITWISE_SREG, NFT_REG_1)
                .be32(NFTA_BITWISE_DREG, NFT_REG_1)
                .be32(NFTA_BITWISE_LEN, u32::try_from(mask.len()).unwrap_or(0))
                .nest(NFTA_BITWISE_MASK, Attrs::new().bytes(NFTA_DATA_VALUE, mask))
                .nest(NFTA_BITWISE_XOR, Attrs::new().bytes(NFTA_DATA_VALUE, xor)),
            Self::Ct { key } => Attrs::new()
                .be32(NFTA_CT_KEY, *key)
                .be32(NFTA_CT_DREG, NFT_REG_1),
            Self::Verdict { code } => Attrs::new()
                .be32(NFTA_IMMEDIATE_DREG, NFT_REG_VERDICT)
                .nest(
                    NFTA_IMMEDIATE_DATA,
                    Attrs::new().nest(
                        NFTA_DATA_VERDICT,
                        Attrs::new().be32(NFTA_VERDICT_CODE, *code),
                    ),
                ),
        };
        Attrs::new()
            .string(NFTA_EXPR_NAME, self.name())
            .nest(NFTA_EXPR_DATA, data)
    }
}

/// `NFTA_RULE_EXPRESSIONS`' contents for `exprs`, in order.
#[must_use]
pub fn expressions(exprs: &[Expr]) -> Attrs {
    exprs
        .iter()
        .fold(Attrs::new(), |list, e| list.nest(NFTA_LIST_ELEM, e.encode()))
}

// ---------------------------------------------------------------------------
// Reading a rule back
// ---------------------------------------------------------------------------

/// One expression as the kernel reported it. Registers are not kept: a rule
/// is recognised by what it tests, and the kernel renumbers registers when it
/// dumps them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExprRead {
    Meta { key: u32 },
    Cmp { op: u32, data: Vec<u8> },
    Payload { base: u32, offset: u32, len: u32 },
    Bitwise { mask: Vec<u8>, xor: Vec<u8> },
    Ct { key: u32 },
    /// `None` when the immediate holds data rather than a verdict.
    Immediate { verdict: Option<u32> },
    /// An expression this crate writes none of, by name.
    Other { name: String },
}

/// A rule as the kernel reported it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleRead {
    pub table: String,
    pub chain: String,
    pub handle: Option<u64>,
    pub exprs: Vec<ExprRead>,
}

/// Decode a `NFT_MSG_NEWRULE` message's payload — `nfgenmsg` and the
/// attributes after it.
///
/// # Errors
///
/// A payload or attribute that does not parse, or an expression whose own
/// attributes are the wrong size. Refused rather than skipped: a rule read
/// back wrongly is a firewall reported as something it is not.
pub fn parse_rule(payload: &[u8]) -> Result<RuleRead, Malformed> {
    let attrs = payload.get(NFGENMSG_LEN..).ok_or(Malformed::ShortPayload {
        want: NFGENMSG_LEN,
        got: payload.len(),
    })?;
    let mut rule = RuleRead {
        table: String::new(),
        chain: String::new(),
        handle: None,
        exprs: Vec::new(),
    };
    for (kind, value) in wire::attributes(attrs)? {
        match kind & NLA_TYPE_MASK {
            NFTA_RULE_TABLE => rule.table = text(value),
            NFTA_RULE_CHAIN => rule.chain = text(value),
            NFTA_RULE_HANDLE => rule.handle = Some(be64(value)?),
            NFTA_RULE_EXPRESSIONS => {
                for (elem, contents) in wire::attributes(value)? {
                    if elem & NLA_TYPE_MASK == NFTA_LIST_ELEM {
                        rule.exprs.push(parse_expr(contents)?);
                    }
                }
            }
            _ => {}
        }
    }
    Ok(rule)
}

fn parse_expr(elem: &[u8]) -> Result<ExprRead, Malformed> {
    let mut name = String::new();
    let mut data: &[u8] = &[];
    for (kind, value) in wire::attributes(elem)? {
        match kind & NLA_TYPE_MASK {
            NFTA_EXPR_NAME => name = text(value),
            NFTA_EXPR_DATA => data = value,
            _ => {}
        }
    }
    let fields = wire::attributes(data)?;
    let field = |want: u16| {
        fields
            .iter()
            .find(|(kind, _)| kind & NLA_TYPE_MASK == want)
            .map(|(_, value)| *value)
    };
    let number = |want: u16| field(want).map_or(Ok(0), be32);
    Ok(match name.as_str() {
        "meta" => ExprRead::Meta {
            key: number(NFTA_META_KEY)?,
        },
        "cmp" => ExprRead::Cmp {
            op: number(NFTA_CMP_OP)?,
            data: field(NFTA_CMP_DATA).map_or(Ok(Vec::new()), data_value)?,
        },
        "payload" => ExprRead::Payload {
            base: number(NFTA_PAYLOAD_BASE)?,
            offset: number(NFTA_PAYLOAD_OFFSET)?,
            len: number(NFTA_PAYLOAD_LEN)?,
        },
        "bitwise" => ExprRead::Bitwise {
            mask: field(NFTA_BITWISE_MASK).map_or(Ok(Vec::new()), data_value)?,
            xor: field(NFTA_BITWISE_XOR).map_or(Ok(Vec::new()), data_value)?,
        },
        "ct" => ExprRead::Ct {
            key: number(NFTA_CT_KEY)?,
        },
        "immediate" => {
            let mut verdict = None;
            if let Some(value) = field(NFTA_IMMEDIATE_DATA) {
                for (kind, inner) in wire::attributes(value)? {
                    if kind & NLA_TYPE_MASK == NFTA_DATA_VERDICT {
                        for (k, code) in wire::attributes(inner)? {
                            if k & NLA_TYPE_MASK == NFTA_VERDICT_CODE {
                                verdict = Some(be32(code)?);
                            }
                        }
                    }
                }
            }
            ExprRead::Immediate { verdict }
        }
        _ => ExprRead::Other { name },
    })
}

/// The bytes inside an `NFTA_DATA_VALUE`, which is how a comparand and a mask
/// are carried.
fn data_value(nested: &[u8]) -> Result<Vec<u8>, Malformed> {
    Ok(wire::attributes(nested)?
        .into_iter()
        .find(|(kind, _)| kind & NLA_TYPE_MASK == NFTA_DATA_VALUE)
        .map(|(_, value)| value.to_vec())
        .unwrap_or_default())
}

/// A NUL-terminated name. Lossy rather than refused: a name is reported, and
/// a reader shown a replacement character is still shown what arrived.
fn text(value: &[u8]) -> String {
    let end = value.iter().position(|b| *b == 0).unwrap_or(value.len());
    String::from_utf8_lossy(value.get(..end).unwrap_or(&[])).into_owned()
}

fn be32(value: &[u8]) -> Result<u32, Malformed> {
    <[u8; 4]>::try_from(value)
        .map(u32::from_be_bytes)
        .map_err(|_| Malformed::ShortPayload {
            want: 4,
            got: value.len(),
        })
}

fn be64(value: &[u8]) -> Result<u64, Malformed> {
    <[u8; 8]>::try_from(value)
        .map(u64::from_be_bytes)
        .map_err(|_| Malformed::ShortPayload {
            want: 8,
            got: value.len(),
        })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// A table message, byte for byte. **Hand-composed from the UAPI
    /// headers**: it proves this encoder agrees with its author's reading of
    /// `nf_tables.h`, and nothing more — the read-back test against a real
    /// kernel's bytes is the one that proves the format.
    #[test]
    fn a_table_message_is_the_header_nfgenmsg_and_a_name() {
        let bytes = object(
            NFT_MSG_NEWTABLE,
            wire::NLM_F_REQUEST | wire::NLM_F_CREATE | wire::NLM_F_ACK,
            7,
            NFPROTO_INET,
            Attrs::new().string(NFTA_TABLE_NAME, "rivet"),
        );
        let mut want = Vec::new();
        want.extend_from_slice(&32u32.to_ne_bytes()); // 16 + 4 + 4 + "rivet\0" padded to 8
        want.extend_from_slice(&0x0a00u16.to_ne_bytes()); // NFNL_SUBSYS_NFTABLES << 8 | NEWTABLE
        want.extend_from_slice(&0x0405u16.to_ne_bytes()); // CREATE | ACK | REQUEST
        want.extend_from_slice(&7u32.to_ne_bytes());
        want.extend_from_slice(&0u32.to_ne_bytes());
        want.extend_from_slice(&[1, 0, 0, 0]); // inet, v0, res_id 0
        want.extend_from_slice(&10u16.to_ne_bytes()); // rta_len: 4 + 6
        want.extend_from_slice(&1u16.to_ne_bytes()); // NFTA_TABLE_NAME
        want.extend_from_slice(b"rivet\0\0\0");
        assert_eq!(bytes, want);
    }

    /// The batch delimiter names the subsystem in `res_id`, big-endian. A
    /// little-endian `res_id` is `0x0a00`, which no subsystem has, and the
    /// kernel refuses the whole batch with `EINVAL`.
    #[test]
    fn a_batch_delimiter_names_nf_tables_in_big_endian() {
        let bytes = delimiter(NFNL_MSG_BATCH_BEGIN, 1);
        assert_eq!(&bytes[4..6], &0x10u16.to_ne_bytes());
        assert_eq!(&bytes[16..20], &[0, 0, 0, 10]);
    }

    /// Nested attributes carry `NLA_F_NESTED`, and integers are big-endian.
    #[test]
    fn a_verdict_is_nested_twice_and_big_endian() {
        let elem = Expr::Verdict { code: NF_DROP }.encode().into_bytes();
        let top = wire::attributes(&elem).unwrap();
        assert_eq!(top[0], (NFTA_EXPR_NAME, &b"immediate\0"[..]));
        assert_eq!(top[1].0, NFTA_EXPR_DATA | NLA_F_NESTED);
        let data = wire::attributes(top[1].1).unwrap();
        assert_eq!(data[0], (NFTA_IMMEDIATE_DREG, &[0, 0, 0, 0][..]));
        assert_eq!(data[1].0, NFTA_IMMEDIATE_DATA | NLA_F_NESTED);
    }

    /// Every expression this crate writes reads back as itself — the
    /// encoder and the parser agree with each other. That is all a round trip
    /// proves; the real-kernel fixture is the test of the format.
    #[test]
    fn every_expression_this_crate_writes_parses_back_as_itself() {
        let exprs = [
            Expr::Meta { key: NFT_META_NFPROTO },
            Expr::Cmp { op: NFT_CMP_EQ, data: vec![NFPROTO_IPV4] },
            Expr::Payload { base: NFT_PAYLOAD_TRANSPORT_HEADER, offset: 2, len: 2 },
            Expr::Bitwise { mask: vec![0xff, 0, 0, 0], xor: vec![0; 4] },
            Expr::Ct { key: NFT_CT_STATE },
            Expr::Verdict { code: NF_DROP },
        ];
        let rule = object(
            NFT_MSG_NEWRULE,
            wire::NLM_F_REQUEST,
            1,
            NFPROTO_INET,
            Attrs::new()
                .string(NFTA_RULE_TABLE, "rivet")
                .string(NFTA_RULE_CHAIN, "c")
                .nest(NFTA_RULE_EXPRESSIONS, expressions(&exprs)),
        );
        let message = wire::messages(&rule).unwrap()[0];
        let read = parse_rule(message.payload).unwrap();
        assert_eq!(read.table, "rivet");
        assert_eq!(read.chain, "c");
        assert_eq!(
            read.exprs,
            vec![
                ExprRead::Meta { key: NFT_META_NFPROTO },
                ExprRead::Cmp { op: NFT_CMP_EQ, data: vec![NFPROTO_IPV4] },
                ExprRead::Payload { base: NFT_PAYLOAD_TRANSPORT_HEADER, offset: 2, len: 2 },
                ExprRead::Bitwise { mask: vec![0xff, 0, 0, 0], xor: vec![0; 4] },
                ExprRead::Ct { key: NFT_CT_STATE },
                ExprRead::Immediate { verdict: Some(NF_DROP) },
            ]
        );
    }

    #[test]
    fn an_expression_this_crate_does_not_write_is_named_rather_than_dropped() {
        let elem = Attrs::new()
            .string(NFTA_EXPR_NAME, "counter")
            .nest(NFTA_EXPR_DATA, Attrs::new())
            .into_bytes();
        assert_eq!(parse_expr(&elem), Ok(ExprRead::Other { name: "counter".to_string() }));
    }

    /// A key that is not four bytes is refused, not zero-extended into a
    /// plausible key.
    #[test]
    fn a_short_integer_attribute_is_refused() {
        let elem = Attrs::new()
            .string(NFTA_EXPR_NAME, "meta")
            .nest(NFTA_EXPR_DATA, Attrs::new().bytes(NFTA_META_KEY, &[0, 10]))
            .into_bytes();
        assert_eq!(parse_expr(&elem), Err(Malformed::ShortPayload { want: 4, got: 2 }));
    }

    #[test]
    fn a_rule_payload_shorter_than_nfgenmsg_is_refused() {
        assert_eq!(
            parse_rule(&[1, 0]),
            Err(Malformed::ShortPayload { want: NFGENMSG_LEN, got: 2 })
        );
    }

    /// The claim in this module's documentation, checked wherever `libc` can
    /// answer — the same guard [`crate::wire`] carries.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_constants_match_the_kernel_headers() {
        assert_eq!(NETLINK_NETFILTER, libc::NETLINK_NETFILTER);
        assert_eq!(i32::from(NFPROTO_INET), libc::NFPROTO_INET);
        assert_eq!(i32::from(NFPROTO_IPV4), libc::NFPROTO_IPV4);
        assert_eq!(i32::from(NFPROTO_IPV6), libc::NFPROTO_IPV6);
        assert_eq!(NF_INET_LOCAL_OUT, libc::NF_INET_LOCAL_OUT as u32);
        assert_eq!(NF_DROP, libc::NF_DROP as u32);
        assert_eq!(NF_ACCEPT, libc::NF_ACCEPT as u32);
        assert_eq!(i32::from(IPPROTO_TCP), libc::IPPROTO_TCP);
        assert_eq!(NLM_F_APPEND, libc::NLM_F_APPEND as u16);
        assert_eq!(NLA_F_NESTED, libc::NLA_F_NESTED as u16);
    }
}
