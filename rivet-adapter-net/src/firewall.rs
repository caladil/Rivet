//! **Rivet's one table**: a loopback connection to a proxied backend's port is
//! accepted only from the uids that may make it. `ADR-023` §3 and §9.
//!
//! # What the table holds
//!
//! ```text
//! table inet rivet {
//!   chain proxied-backends {
//!     type filter hook output priority 0; policy accept;
//!     meta nfproto ipv4 meta l4proto tcp ip daddr 127.0.0.0/8 tcp dport P
//!         ct state new meta skuid != U1 meta skuid != U2 ... drop
//!     meta nfproto ipv6 meta l4proto tcp ip6 daddr ::1 tcp dport P
//!         ct state new meta skuid != U1 ... drop
//!     (two rules per port)
//!   }
//! }
//! ```
//!
//! **The output hook**, because a connection made on this machine to this
//! machine passes there first carrying the socket that made it, and the
//! socket's owner is the whole question. **`policy accept`**, because the
//! chain exists to refuse one thing; every packet the rules do not name
//! passes exactly as it would with no table at all, which is what makes a
//! defect here unable to cut the machine off its network.
//!
//! **`drop`, not `reject`.** `reject` is its own kernel module on the target
//! kernel and the card carries no modules, so a rule naming it would be
//! refused and the table would not exist. `drop` is a core verdict. Its cost
//! is that a refused caller's `connect` waits out its own timeout instead of
//! failing at once.
//!
//! # Rebuilt whole, atomically, and only this table
//!
//! One transaction: create the table if absent, delete it, then create it with
//! its chain and rules. Creating before deleting is what lets the delete never
//! fail on a first boot, and the batch is applied all or nothing, so there is
//! no moment with half a table. **Nothing this module sends names any other
//! table.** An empty policy is the same transaction without the second
//! creation, which leaves the machine with no Rivet table at all.
//!
//! # Read back, not assumed
//!
//! After the transaction the chain is dumped and every rule recognised from
//! what the kernel reports. [`Applied::matches`] compares that with the
//! policy, so a journal line saying the table is in place is a reading of the
//! kernel, not a restatement of the request.

use crate::nft::{
    self, Attrs, Expr, ExprRead, RuleRead, NFPROTO_INET, NFPROTO_IPV4, NFPROTO_IPV6,
};
use crate::port::{self, Failure, SendRecv, Sequence};
use crate::wire;

/// The one table Rivet owns, in the `inet` family.
pub const TABLE: &str = "rivet";
/// Its one chain.
pub const CHAIN: &str = "proxied-backends";

/// Who may connect to which loopback ports. Derived, never stored — the
/// caller builds it from installed manifests each time it applies one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    admitted: Vec<u32>,
    ports: Vec<u16>,
}

impl Policy {
    /// Both lists are sorted and deduplicated, so two policies built from the
    /// same facts in a different order are equal.
    #[must_use]
    pub fn new(admitted: &[u32], ports: &[u16]) -> Self {
        let mut admitted = admitted.to_vec();
        admitted.sort_unstable();
        admitted.dedup();
        let mut ports = ports.to_vec();
        ports.sort_unstable();
        ports.dedup();
        Self { admitted, ports }
    }

    #[must_use]
    pub fn admitted(&self) -> &[u32] {
        &self.admitted
    }

    #[must_use]
    pub fn ports(&self) -> &[u16] {
        &self.ports
    }

    /// The rules this policy is, one per port per family, in the order they
    /// are written.
    #[must_use]
    pub fn guards(&self) -> Vec<Guard> {
        self.ports
            .iter()
            .flat_map(|port| {
                [Family::Ipv4, Family::Ipv6].map(|family| Guard {
                    family,
                    port: *port,
                    admitted: self.admitted.clone(),
                })
            })
            .collect()
    }
}

/// Which loopback a guard covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Family {
    /// `127.0.0.0/8`.
    Ipv4,
    /// `::1`.
    Ipv6,
}

impl core::fmt::Display for Family {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::Ipv4 => "ipv4",
            Self::Ipv6 => "ipv6",
        })
    }
}

/// One rule: a new TCP connection to `port` on the loopback of `family` is
/// dropped unless its socket belongs to one of `admitted`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Guard {
    pub family: Family,
    pub port: u16,
    pub admitted: Vec<u32>,
}

impl core::fmt::Display for Guard {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let uids: Vec<String> = self.admitted.iter().map(u32::to_string).collect();
        write!(
            f,
            "{} loopback tcp/{} only from uid {}",
            self.family,
            self.port,
            uids.join(",")
        )
    }
}

impl Guard {
    /// The expressions, tests first and the verdict last.
    ///
    /// The uid and the conntrack state are compared in the machine's own byte
    /// order, because that is how the kernel puts them in the register; the
    /// port and the address in network order, because they are header bytes.
    #[must_use]
    pub fn expressions(&self) -> Vec<Expr> {
        let (nfproto, address) = match self.family {
            Family::Ipv4 => (
                NFPROTO_IPV4,
                vec![
                    Expr::Payload {
                        base: nft::NFT_PAYLOAD_NETWORK_HEADER,
                        offset: 16,
                        len: 4,
                    },
                    Expr::Bitwise {
                        mask: vec![0xff, 0, 0, 0],
                        xor: vec![0; 4],
                    },
                    Expr::Cmp {
                        op: nft::NFT_CMP_EQ,
                        data: vec![127, 0, 0, 0],
                    },
                ],
            ),
            Family::Ipv6 => (
                NFPROTO_IPV6,
                vec![
                    Expr::Payload {
                        base: nft::NFT_PAYLOAD_NETWORK_HEADER,
                        offset: 24,
                        len: 16,
                    },
                    Expr::Cmp {
                        op: nft::NFT_CMP_EQ,
                        data: std::net::Ipv6Addr::LOCALHOST.octets().to_vec(),
                    },
                ],
            ),
        };
        let mut exprs = vec![
            Expr::Meta {
                key: nft::NFT_META_NFPROTO,
            },
            Expr::Cmp {
                op: nft::NFT_CMP_EQ,
                data: vec![nfproto],
            },
            Expr::Meta {
                key: nft::NFT_META_L4PROTO,
            },
            Expr::Cmp {
                op: nft::NFT_CMP_EQ,
                data: vec![nft::IPPROTO_TCP],
            },
        ];
        exprs.extend(address);
        exprs.extend([
            Expr::Payload {
                base: nft::NFT_PAYLOAD_TRANSPORT_HEADER,
                offset: 2,
                len: 2,
            },
            Expr::Cmp {
                op: nft::NFT_CMP_EQ,
                data: self.port.to_be_bytes().to_vec(),
            },
            Expr::Ct {
                key: nft::NFT_CT_STATE,
            },
            Expr::Bitwise {
                mask: nft::NF_CT_STATE_NEW.to_ne_bytes().to_vec(),
                xor: vec![0; 4],
            },
            Expr::Cmp {
                op: nft::NFT_CMP_NEQ,
                data: vec![0; 4],
            },
            Expr::Meta {
                key: nft::NFT_META_SKUID,
            },
        ]);
        for uid in &self.admitted {
            exprs.push(Expr::Cmp {
                op: nft::NFT_CMP_NEQ,
                data: uid.to_ne_bytes().to_vec(),
            });
        }
        exprs.push(Expr::Verdict { code: nft::NF_DROP });
        exprs
    }

    /// The guard a rule the kernel reported is, or `None` when it is any
    /// other shape. Exact: an extra test, a missing one, or a verdict other
    /// than `drop` is not this module's rule.
    #[must_use]
    pub fn recognise(rule: &RuleRead) -> Option<Self> {
        let e = &rule.exprs;
        let mut at = 0;
        let mut next = || {
            let one = e.get(at);
            at += 1;
            one
        };
        let eq = |x: Option<&ExprRead>, data: &[u8]| {
            matches!(x, Some(ExprRead::Cmp { op, data: d }) if *op == nft::NFT_CMP_EQ && d == data)
        };

        if next() != Some(&ExprRead::Meta { key: nft::NFT_META_NFPROTO }) {
            return None;
        }
        let family = match next() {
            Some(ExprRead::Cmp { op, data }) if *op == nft::NFT_CMP_EQ && data == &[NFPROTO_IPV4] => {
                Family::Ipv4
            }
            Some(ExprRead::Cmp { op, data }) if *op == nft::NFT_CMP_EQ && data == &[NFPROTO_IPV6] => {
                Family::Ipv6
            }
            _ => return None,
        };
        if next() != Some(&ExprRead::Meta { key: nft::NFT_META_L4PROTO })
            || !eq(next(), &[nft::IPPROTO_TCP])
        {
            return None;
        }
        match family {
            Family::Ipv4 => {
                let load = ExprRead::Payload {
                    base: nft::NFT_PAYLOAD_NETWORK_HEADER,
                    offset: 16,
                    len: 4,
                };
                let mask = ExprRead::Bitwise {
                    mask: vec![0xff, 0, 0, 0],
                    xor: vec![0; 4],
                };
                if next() != Some(&load) || next() != Some(&mask) || !eq(next(), &[127, 0, 0, 0]) {
                    return None;
                }
            }
            Family::Ipv6 => {
                let load = ExprRead::Payload {
                    base: nft::NFT_PAYLOAD_NETWORK_HEADER,
                    offset: 24,
                    len: 16,
                };
                if next() != Some(&load) || !eq(next(), &std::net::Ipv6Addr::LOCALHOST.octets()) {
                    return None;
                }
            }
        }
        let port_load = ExprRead::Payload {
            base: nft::NFT_PAYLOAD_TRANSPORT_HEADER,
            offset: 2,
            len: 2,
        };
        if next() != Some(&port_load) {
            return None;
        }
        let port = match next() {
            Some(ExprRead::Cmp { op, data }) if *op == nft::NFT_CMP_EQ => {
                u16::from_be_bytes(<[u8; 2]>::try_from(data.as_slice()).ok()?)
            }
            _ => return None,
        };
        let new_only = ExprRead::Bitwise {
            mask: nft::NF_CT_STATE_NEW.to_ne_bytes().to_vec(),
            xor: vec![0; 4],
        };
        if next() != Some(&ExprRead::Ct { key: nft::NFT_CT_STATE })
            || next() != Some(&new_only)
            || next()
                != Some(&ExprRead::Cmp {
                    op: nft::NFT_CMP_NEQ,
                    data: vec![0; 4],
                })
            || next() != Some(&ExprRead::Meta { key: nft::NFT_META_SKUID })
        {
            return None;
        }
        let mut admitted = Vec::new();
        loop {
            match next() {
                Some(ExprRead::Cmp { op, data }) if *op == nft::NFT_CMP_NEQ => {
                    admitted.push(u32::from_ne_bytes(<[u8; 4]>::try_from(data.as_slice()).ok()?));
                }
                Some(ExprRead::Immediate {
                    verdict: Some(code),
                }) if *code == nft::NF_DROP => break,
                _ => return None,
            }
        }
        if next().is_some() {
            return None;
        }
        Some(Self {
            family,
            port,
            admitted,
        })
    }
}

/// One message of the transaction, and what it was for — so a refusal names
/// the step the kernel refused rather than a sequence number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub seq: u32,
    pub what: String,
}

/// The whole transaction as one datagram, and the steps it expects an
/// acknowledgement for.
#[must_use]
pub fn transaction(policy: &Policy, seq: &mut Sequence) -> (Vec<u8>, Vec<Step>) {
    let ack = wire::NLM_F_REQUEST | wire::NLM_F_ACK;
    let mut out = nft::delimiter(nft::NFNL_MSG_BATCH_BEGIN, seq.take());
    let mut steps = Vec::new();
    let mut push = |out: &mut Vec<u8>, steps: &mut Vec<Step>, what: String, build: &dyn Fn(u32) -> Vec<u8>| {
        let n = seq.take();
        out.extend(build(n));
        steps.push(Step { seq: n, what });
    };
    let table = || Attrs::new().string(nft::NFTA_TABLE_NAME, TABLE);

    push(&mut out, &mut steps, format!("make sure table inet {TABLE} exists"), &|n| {
        nft::object(nft::NFT_MSG_NEWTABLE, ack | wire::NLM_F_CREATE, n, NFPROTO_INET, table())
    });
    push(&mut out, &mut steps, format!("delete table inet {TABLE}"), &|n| {
        nft::object(nft::NFT_MSG_DELTABLE, ack, n, NFPROTO_INET, table())
    });

    let guards = policy.guards();
    if !guards.is_empty() {
        push(&mut out, &mut steps, format!("create table inet {TABLE}"), &|n| {
            nft::object(
                nft::NFT_MSG_NEWTABLE,
                ack | wire::NLM_F_CREATE | wire::NLM_F_EXCL,
                n,
                NFPROTO_INET,
                table(),
            )
        });
        push(&mut out, &mut steps, format!("create chain {CHAIN}"), &|n| {
            nft::object(
                nft::NFT_MSG_NEWCHAIN,
                ack | wire::NLM_F_CREATE | wire::NLM_F_EXCL,
                n,
                NFPROTO_INET,
                Attrs::new()
                    .string(nft::NFTA_CHAIN_TABLE, TABLE)
                    .string(nft::NFTA_CHAIN_NAME, CHAIN)
                    .nest(
                        nft::NFTA_CHAIN_HOOK,
                        Attrs::new()
                            .be32(nft::NFTA_HOOK_HOOKNUM, nft::NF_INET_LOCAL_OUT)
                            .be32(nft::NFTA_HOOK_PRIORITY, 0),
                    )
                    .be32(nft::NFTA_CHAIN_POLICY, nft::NF_ACCEPT)
                    .string(nft::NFTA_CHAIN_TYPE, "filter"),
            )
        });
        for guard in &guards {
            push(&mut out, &mut steps, format!("add the rule: {guard}"), &|n| {
                nft::object(
                    nft::NFT_MSG_NEWRULE,
                    ack | wire::NLM_F_CREATE | nft::NLM_F_APPEND,
                    n,
                    NFPROTO_INET,
                    Attrs::new()
                        .string(nft::NFTA_RULE_TABLE, TABLE)
                        .string(nft::NFTA_RULE_CHAIN, CHAIN)
                        .nest(nft::NFTA_RULE_EXPRESSIONS, nft::expressions(&guard.expressions())),
                )
            });
        }
    }
    out.extend(nft::delimiter(nft::NFNL_MSG_BATCH_END, seq.take()));
    (out, steps)
}

/// A transaction or a read-back the kernel did not accept, and which step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    pub what: String,
    pub failure: Failure,
}

impl core::fmt::Display for Refused {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}: {}", self.what, self.failure)?;
        // **ENOENT on a rule is a missing expression**, which is the one
        // refusal that names the kernel's configuration rather than this
        // code, so it says so.
        if self.failure.kernel_errno() == Some(port::Errno::ENOENT) && self.what.starts_with("add the rule") {
            write!(
                f,
                " — this kernel lacks an expression the rule uses (meta, payload, bitwise, cmp, ct \
                 or immediate)"
            )?;
        }
        Ok(())
    }
}

/// What the kernel holds after an apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    /// Every rule in Rivet's chain that is one of this module's guards.
    pub found: Vec<Guard>,
    /// Rules in the chain that are not — none, unless something other than
    /// this module wrote into Rivet's table.
    pub unrecognised: usize,
    /// The dump's datagrams, as the kernel sent them, for a record that wants
    /// the bytes.
    pub raw: Vec<Vec<u8>>,
}

impl Applied {
    /// Whether the kernel holds exactly the policy's rules and nothing else.
    #[must_use]
    pub fn matches(&self, policy: &Policy) -> bool {
        let mut want = policy.guards();
        want.sort();
        let mut have = self.found.clone();
        have.sort();
        self.unrecognised == 0 && want == have
    }
}

/// **Replace Rivet's table with `policy`, and read back what the kernel
/// holds.**
///
/// # Errors
///
/// The step the kernel refused, or the read-back. On a refusal the kernel has
/// applied nothing of this transaction — a batch is all or nothing — so the
/// machine's filtering is what it was before the call.
pub fn apply(nl: &mut dyn SendRecv, policy: &Policy) -> Result<Applied, Refused> {
    let mut seq = Sequence::new();
    let (batch, steps) = transaction(policy, &mut seq);
    nl.send(&batch).map_err(|e| Refused {
        what: "send the transaction".to_string(),
        failure: Failure::Socket(e),
    })?;
    collect_acks(nl, &steps)?;
    read_back(nl, &mut seq)
}

/// Read one acknowledgement per step, refusing on the first non-zero one.
///
/// A refused batch is aborted whole, and the kernel still answers each step it
/// was asked to acknowledge — the steps before the refusal with zero. So a
/// zero is only a success once every step has one; any refusal anywhere is
/// the answer.
fn collect_acks(nl: &mut dyn SendRecv, steps: &[Step]) -> Result<(), Refused> {
    let refused = |what: &str, failure: Failure| Refused {
        what: what.to_string(),
        failure,
    };
    let mut answered = vec![false; steps.len()];
    let mut datagrams = 0usize;
    while answered.iter().any(|a| !a) {
        if datagrams >= port::DUMP_DATAGRAM_LIMIT {
            return Err(refused(
                "read the acknowledgements",
                Failure::Unterminated { datagrams },
            ));
        }
        datagrams += 1;
        let datagram = nl
            .recv()
            .map_err(|e| refused("read the acknowledgements", Failure::Socket(e)))?;
        let messages = wire::messages(&datagram)
            .map_err(|m| refused("read the acknowledgements", Failure::Malformed(m)))?;
        for message in messages {
            match message.kind {
                wire::NLMSG_NOOP => {}
                wire::NLMSG_OVERRUN => return Err(refused("read the acknowledgements", Failure::Overrun)),
                wire::NLMSG_ERROR => {
                    let Some(at) = steps.iter().position(|s| s.seq == message.seq) else {
                        return Err(refused(
                            "read the acknowledgements",
                            Failure::WrongSequence {
                                expected: steps.first().map_or(0, |s| s.seq),
                                got: message.seq,
                            },
                        ));
                    };
                    let code = wire::error_code(message.payload)
                        .map_err(|m| refused(&steps[at].what, Failure::Malformed(m)))?;
                    if code != 0 {
                        return Err(refused(&steps[at].what, Failure::from_netlink_error(code)));
                    }
                    answered[at] = true;
                }
                kind => {
                    return Err(refused(
                        "read the acknowledgements",
                        Failure::Unexpected { kind },
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Dump Rivet's chain and recognise every rule in it.
fn read_back(nl: &mut dyn SendRecv, seq: &mut Sequence) -> Result<Applied, Refused> {
    let refused = |failure: Failure| Refused {
        what: format!("read back table inet {TABLE}"),
        failure,
    };
    let n = seq.take();
    let request = nft::object(
        nft::NFT_MSG_GETRULE,
        wire::NLM_F_REQUEST | wire::NLM_F_DUMP,
        n,
        NFPROTO_INET,
        Attrs::new()
            .string(nft::NFTA_RULE_TABLE, TABLE)
            .string(nft::NFTA_RULE_CHAIN, CHAIN),
    );
    let raw = port::dump(nl, request, n).map_err(refused)?;
    let mut found = Vec::new();
    let mut unrecognised = 0;
    for datagram in &raw {
        for message in wire::messages(datagram).map_err(|m| refused(Failure::Malformed(m)))? {
            if message.kind != nft::message_type(nft::NFT_MSG_NEWRULE) {
                continue;
            }
            let rule = nft::parse_rule(message.payload).map_err(|m| refused(Failure::Malformed(m)))?;
            // The dump is asked for by table and chain; a kernel that ignored
            // the filter would hand back other tables' rules, and those are
            // not counted for or against this one.
            if rule.table != TABLE || rule.chain != CHAIN {
                continue;
            }
            match Guard::recognise(&rule) {
                Some(guard) => found.push(guard),
                None => unrecognised += 1,
            }
        }
    }
    Ok(Applied {
        found,
        unrecognised,
        raw,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::port::Errno;
    use crate::testnetlink::{FakeNetlink, Script, dump_done, netlink_error};

    fn ack(seq: u32) -> Vec<u8> {
        netlink_error(seq, 0)
    }

    /// The kernel's NEWRULE for `guard`, as a dump carries it — built with
    /// this crate's own encoder, so it proves the module agrees with itself.
    fn reported(seq: u32, guard: &Guard) -> Vec<u8> {
        let mut m = wire::Message::new(
            nft::message_type(nft::NFT_MSG_NEWRULE),
            wire::NLM_F_MULTI,
            seq,
        );
        m.push_payload(&nft::nfgenmsg(NFPROTO_INET, 0));
        m.push_payload(
            &Attrs::new()
                .string(nft::NFTA_RULE_TABLE, TABLE)
                .string(nft::NFTA_RULE_CHAIN, CHAIN)
                .nest(nft::NFTA_RULE_EXPRESSIONS, nft::expressions(&guard.expressions()))
                .into_bytes(),
        );
        m.finish()
    }

    fn policy() -> Policy {
        Policy::new(&[0, 41_000], &[8080])
    }

    #[test]
    fn a_policy_is_one_guard_per_port_per_family_with_its_uids_sorted() {
        let p = Policy::new(&[41_000, 0, 41_000], &[9000, 8080, 8080]);
        assert_eq!(p.admitted(), &[0, 41_000]);
        assert_eq!(p.ports(), &[8080, 9000]);
        let guards = p.guards();
        assert_eq!(guards.len(), 4);
        assert_eq!(guards[0].to_string(), "ipv4 loopback tcp/8080 only from uid 0,41000");
        assert_eq!(guards[1].family, Family::Ipv6);
    }

    /// Every guard this module writes is recognised when it is read back — and
    /// one test or verdict different is not.
    #[test]
    fn a_guard_is_recognised_exactly_and_nothing_near_it_is() {
        for guard in policy().guards() {
            let rule = RuleRead {
                table: TABLE.into(),
                chain: CHAIN.into(),
                handle: Some(4),
                exprs: guard.expressions().iter().map(read_as).collect(),
            };
            assert_eq!(Guard::recognise(&rule), Some(guard.clone()));

            let mut accepting = rule.clone();
            *accepting.exprs.last_mut().unwrap() = ExprRead::Immediate {
                verdict: Some(nft::NF_ACCEPT),
            };
            assert_eq!(Guard::recognise(&accepting), None, "an accept is not this rule");

            let mut extra = rule.clone();
            extra.exprs.push(ExprRead::Other { name: "counter".into() });
            assert_eq!(Guard::recognise(&extra), None, "a trailing expression is not this rule");

            let mut no_ct = rule;
            no_ct.exprs.retain(|e| !matches!(e, ExprRead::Ct { .. }));
            assert_eq!(Guard::recognise(&no_ct), None, "a rule without ct state is not this rule");
        }
    }

    fn read_as(e: &Expr) -> ExprRead {
        match e.clone() {
            Expr::Meta { key } => ExprRead::Meta { key },
            Expr::Cmp { op, data } => ExprRead::Cmp { op, data },
            Expr::Payload { base, offset, len } => ExprRead::Payload { base, offset, len },
            Expr::Bitwise { mask, xor } => ExprRead::Bitwise { mask, xor },
            Expr::Ct { key } => ExprRead::Ct { key },
            Expr::Verdict { code } => ExprRead::Immediate { verdict: Some(code) },
        }
    }

    /// The transaction touches one table by name, and no message in it names
    /// any other — the property that keeps an operator's own tables safe.
    #[test]
    fn the_transaction_names_only_rivets_table() {
        let (batch, steps) = transaction(&policy(), &mut Sequence::new());
        let messages = wire::messages(&batch).unwrap();
        assert_eq!(messages.first().unwrap().kind, nft::NFNL_MSG_BATCH_BEGIN);
        assert_eq!(messages.last().unwrap().kind, nft::NFNL_MSG_BATCH_END);
        // begin, ensure, delete, create, chain, two rules, end.
        assert_eq!(messages.len(), 8);
        assert_eq!(steps.len(), 6);
        for message in &messages[1..messages.len() - 1] {
            let attrs = wire::attributes(&message.payload[nft::NFGENMSG_LEN..]).unwrap();
            let names: Vec<&[u8]> = attrs
                .iter()
                .filter(|(kind, _)| kind & nft::NLA_TYPE_MASK == 1)
                .map(|(_, v)| *v)
                .collect();
            assert_eq!(names, vec![&b"rivet\0"[..]], "{message:?}");
        }
        // Every step asks for an acknowledgement; the delimiters do not.
        for message in &messages[1..messages.len() - 1] {
            assert_ne!(message.flags & wire::NLM_F_ACK, 0);
        }
    }

    /// No proxied port: the table is ensured and deleted, nothing is created,
    /// and the machine is left with no Rivet table.
    #[test]
    fn an_empty_policy_deletes_the_table_and_creates_nothing() {
        let (batch, steps) = transaction(&Policy::new(&[0], &[]), &mut Sequence::new());
        let kinds: Vec<u16> = wire::messages(&batch).unwrap().iter().map(|m| m.kind).collect();
        assert_eq!(
            kinds,
            vec![
                nft::NFNL_MSG_BATCH_BEGIN,
                nft::message_type(nft::NFT_MSG_NEWTABLE),
                nft::message_type(nft::NFT_MSG_DELTABLE),
                nft::NFNL_MSG_BATCH_END
            ]
        );
        assert_eq!(steps.len(), 2);
    }

    /// The whole conversation against a fake kernel: every step acknowledged,
    /// the dump answered with the policy's rules, and the result matches.
    #[test]
    fn an_accepted_transaction_is_read_back_and_matches() {
        let p = policy();
        let (_, steps) = transaction(&p, &mut Sequence::new());
        let read_seq = steps.last().unwrap().seq + 2; // the END delimiter, then the dump
        let mut acks = Vec::new();
        for step in &steps {
            acks.extend(ack(step.seq));
        }
        let mut dumped = Vec::new();
        for guard in p.guards() {
            dumped.extend(reported(read_seq, &guard));
        }
        dumped.extend(dump_done(read_seq));

        let mut nl = FakeNetlink::new();
        nl.replies = Script::sequence(vec![Ok(acks), Ok(dumped)], Ok(Vec::new()));
        let applied = apply(&mut nl, &p).expect("accepted");
        assert!(applied.matches(&p), "{applied:?}");
        assert_eq!(nl.sent.len(), 2, "one transaction, one dump");
    }

    /// A refusal names its step, and ENOENT on a rule says the kernel is what
    /// is missing something.
    #[test]
    fn a_refused_rule_names_the_rule_and_the_missing_expression() {
        let p = policy();
        let (_, steps) = transaction(&p, &mut Sequence::new());
        let mut reply = Vec::new();
        for step in &steps[..4] {
            reply.extend(ack(step.seq));
        }
        reply.extend(netlink_error(steps[4].seq, -2));
        let mut nl = FakeNetlink::new();
        nl.replies = Script::always(Ok(reply));
        let refused = apply(&mut nl, &p).expect_err("refused");
        assert_eq!(refused.failure, Failure::Kernel(Errno::ENOENT));
        let text = refused.to_string();
        assert!(text.contains("add the rule: ipv4 loopback tcp/8080"), "{text}");
        assert!(text.contains("lacks an expression"), "{text}");
    }

    /// Zeros before the refusal are not success: the batch was aborted whole.
    #[test]
    fn earlier_acknowledgements_do_not_hide_a_later_refusal() {
        let p = policy();
        let (_, steps) = transaction(&p, &mut Sequence::new());
        let mut nl = FakeNetlink::new();
        nl.replies = Script::sequence(
            vec![
                Ok(ack(steps[0].seq)),
                Ok(ack(steps[1].seq)),
                Ok(netlink_error(steps[3].seq, -95)),
            ],
            Ok(Vec::new()),
        );
        let refused = apply(&mut nl, &p).expect_err("refused");
        assert_eq!(refused.failure, Failure::Kernel(Errno::EOPNOTSUPP));
        assert!(refused.what.contains("create chain"), "{refused}");
    }

    /// A kernel that goes quiet is a socket failure, not a table in place.
    #[test]
    fn silence_is_a_refusal_rather_than_success() {
        let mut nl = FakeNetlink::new();
        nl.replies = Script::always(Err(Errno::EAGAIN));
        let refused = apply(&mut nl, &policy()).expect_err("refused");
        assert_eq!(refused.failure, Failure::Socket(Errno::EAGAIN));
    }

    /// **The read-back of a real kernel.** Captured from a Raspberry Pi 4
    /// running a Rivet card (aarch64, little-endian), from the debug record
    /// the supervisor journals after an apply: the two datagrams the kernel
    /// sent for the `GETRULE` dump of the table this module had just written
    /// for one routed port, 8080, admitting uids 0 and 55448. Sequence 9 is
    /// the dump's, because the transaction for that policy uses 1 to 8.
    ///
    /// It proves the parser reads what a kernel writes — registers renumbered,
    /// `bitwise`'s extra operator attribute, `NLA_F_NESTED` on every nest, a
    /// rule handle — and not only what this crate's own encoder writes. The
    /// uid and conntrack bytes are the board's own order, so the fixture
    /// holds on a little-endian host only.
    #[cfg(target_endian = "little")]
    #[test]
    fn a_real_kernels_read_back_is_recognised_as_the_policy() {
        const DUMP_RULES: &str =
            "38030000060a02080900000091000000010000020a00010072697665740000001500020070726f786965642d6261636b656e6473000000000c0003000000000000000002f402040024000100090001006d6574610000000014000200080002000000000f08000100000000012c00010008000100636d700020000200080001000000000108000200000000000c000300050001000200000024000100090001006d6574610000000014000200080002000000001008000100000000012c00010008000100636d700020000200080001000000000108000200000000000c0003000500010006000000340001000c0001007061796c6f6164002400020008000100000000010800020000000001080003000000001008000400000000044c0001000c00010062697477697365003c00020008000100000000010800020000000001080003000000000408000600000000000c00040008000100ff0000000c00050008000100000000002c00010008000100636d700020000200080001000000000108000200000000000c000300080001007f000000340001000c0001007061796c6f6164002400020008000100000000010800020000000002080003000000000208000400000000022c00010008000100636d700020000200080001000000000108000200000000000c000300060001001f90000020000100070001006374000014000200080001000000000108000200000000004c0001000c00010062697477697365003c00020008000100000000010800020000000001080003000000000408000600000000000c00040008000100080000000c00050008000100000000002c00010008000100636d700020000200080001000000000108000200000000010c000300080001000000000024000100090001006d6574610000000014000200080002000000000a08000100000000012c00010008000100636d700020000200080001000000000108000200000000010c00030008000100000000002c00010008000100636d700020000200080001000000000108000200000000010c0003000800010098d80000300001000e000100696d6d6564696174650000001c0002000800010000000000100002000c000200080001000000000004030000060a02080900000091000000010000020a00010072697665740000001500020070726f786965642d6261636b656e6473000000000c00030000000000000000030c0006000000000000000002b402040024000100090001006d6574610000000014000200080002000000000f08000100000000012c00010008000100636d700020000200080001000000000108000200000000000c000300050001000a00000024000100090001006d6574610000000014000200080002000000001008000100000000012c00010008000100636d700020000200080001000000000108000200000000000c0003000500010006000000340001000c0001007061796c6f6164002400020008000100000000010800020000000001080003000000001808000400000000103800010008000100636d70002c00020008000100000000010800020000000000180003001400010000000000000000000000000000000001340001000c0001007061796c6f6164002400020008000100000000010800020000000002080003000000000208000400000000022c00010008000100636d700020000200080001000000000108000200000000000c000300060001001f90000020000100070001006374000014000200080001000000000108000200000000004c0001000c00010062697477697365003c00020008000100000000010800020000000001080003000000000408000600000000000c00040008000100080000000c00050008000100000000002c00010008000100636d700020000200080001000000000108000200000000010c000300080001000000000024000100090001006d6574610000000014000200080002000000000a08000100000000012c00010008000100636d700020000200080001000000000108000200000000010c00030008000100000000002c00010008000100636d700020000200080001000000000108000200000000010c0003000800010098d80000300001000e000100696d6d6564696174650000001c0002000800010000000000100002000c0002000800010000000000";
        const DUMP_DONE: &str = "1400000003000200090000009100000000000000";
        fn bytes(hex: &str) -> Vec<u8> {
            (0..hex.len())
                .step_by(2)
                .map(|at| u8::from_str_radix(&hex[at..at + 2], 16).unwrap())
                .collect()
        }

        let p = Policy::new(&[0, 55_448], &[8080]);
        let (_, steps) = transaction(&p, &mut Sequence::new());
        let mut acks = Vec::new();
        for step in &steps {
            acks.extend(ack(step.seq));
        }
        let mut nl = FakeNetlink::new();
        nl.replies = Script::sequence(
            vec![Ok(acks), Ok(bytes(DUMP_RULES)), Ok(bytes(DUMP_DONE))],
            Ok(Vec::new()),
        );
        let applied = apply(&mut nl, &p).expect("the kernel's own dump parses");
        assert_eq!(applied.unrecognised, 0, "{applied:?}");
        assert!(applied.matches(&p), "{:?}", applied.found);
    }

    /// A rule in Rivet's chain that is not one of its guards is counted, and
    /// the result does not match.
    #[test]
    fn a_foreign_rule_in_the_chain_is_counted_and_fails_the_match() {
        let p = policy();
        let (_, steps) = transaction(&p, &mut Sequence::new());
        let read_seq = steps.last().unwrap().seq + 2;
        let mut acks = Vec::new();
        for step in &steps {
            acks.extend(ack(step.seq));
        }
        let mut dumped = Vec::new();
        for guard in p.guards() {
            dumped.extend(reported(read_seq, &guard));
        }
        let mut stray = wire::Message::new(nft::message_type(nft::NFT_MSG_NEWRULE), wire::NLM_F_MULTI, read_seq);
        stray.push_payload(&nft::nfgenmsg(NFPROTO_INET, 0));
        stray.push_payload(
            &Attrs::new()
                .string(nft::NFTA_RULE_TABLE, TABLE)
                .string(nft::NFTA_RULE_CHAIN, CHAIN)
                .nest(nft::NFTA_RULE_EXPRESSIONS, nft::expressions(&[Expr::Verdict { code: nft::NF_ACCEPT }]))
                .into_bytes(),
        );
        dumped.extend(stray.finish());
        dumped.extend(dump_done(read_seq));
        let mut nl = FakeNetlink::new();
        nl.replies = Script::sequence(vec![Ok(acks), Ok(dumped)], Ok(Vec::new()));
        let applied = apply(&mut nl, &p).expect("accepted");
        assert_eq!(applied.unrecognised, 1);
        assert!(!applied.matches(&p));
    }
}
