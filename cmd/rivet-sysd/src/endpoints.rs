//! **`provides` and `connects` — application-to-application sockets.**
//!
//! `ADR-015` §5.3. An application declares `provides = ["socket:api"]` and
//! another declares `connects = ["photo-index:api"]`; this module decides which
//! of those declarations become grants, and which are refused and why.
//!
//! # What the ADR asks for, and what this does instead
//!
//! §5.3's mechanism is a bind mount: *"Rivet creates the socket and bind-mounts
//! it into only the mount namespaces of applications that declared and were
//! granted the connection."* **This build has no mount namespaces** (`ADR-015`
//! §5.1, and `rivet_core::service::LISTEN_SOCKET_MODE` says so in the place a
//! reader would look). So the mechanism here is the other one this tree already
//! took, for the same reason and with the same shape: `crate::devices`' §6.2
//! grant — **declare, chown, one owner at a time.**
//!
//! The socket is created root-owned `0600` like any other listener. When a
//! consumer is granted it, the *file* is chowned to the consumer's uid. The
//! provider is unaffected: it was handed a descriptor and never opens the path
//! (`ADR-015` §5.2), so ownership of the file is free to describe who may
//! **connect** rather than who may listen.
//!
//! What that buys is the sentence `LISTEN_SOCKET_MODE` says cannot be expressed:
//! *"B may connect to A"* — B, and not everyone. What it costs is stated below
//! and is not hidden: **one consumer per endpoint.**
//!
//! # One consumer per endpoint, unless the PROVIDER says otherwise
//!
//! `chown` names one uid, so an ordinary endpoint has one consumer. **When
//! more than one application claims it, every claimant is refused**, and each
//! refusal names them all: granting it to whichever claimant sorts first would
//! let an application installed later take an endpoint away from the one
//! holding it just by choosing its name.
//!
//! **A provider may declare `provides = ["socket:<name>:shared"]`**, and then
//! it may have many — `.rulings/` R-206, 2026-09-20. A shared socket is
//! `root:<endpoint gid>` at `0660` and every granted consumer is spawned with
//! that one supplementary group. `rivet_core::service::endpoint_gid` derives
//! the group; `Endpoint::shared` argues why the capability has to exist.
//!
//! **This paragraph used to say the opposite**, and the correction is worth
//! keeping because the reasoning was sound and the premise moved. It read:
//!
//! > Two consumers would need either a shared supplementary group — which this
//! > supervisor deliberately does not have, it calls `setgroups(0, NULL)`
//! > before it drops privilege — or a mode that grants everyone, which is what
//! > `LISTEN_SOCKET_MODE` refuses to do.
//!
//! Both halves were true. What was missing is that **dropping INHERITED groups
//! and granting one the manifest EARNED are different acts**: `setgroups(0,
//! NULL)` exists so that nothing arrives by accident, and a single gid handed
//! to a consumer the supervisor has just granted arrives by declaration. The
//! alternative rejected here — a mode that grants everyone — is still rejected,
//! and for the reason given: every application is granted `/run/rivet` by its
//! Landlock ruleset, so `0666` would turn a declaration into a comment.
//!
//! **What forced it** is `ADR-058`. A compositor exists so that several
//! applications hold surfaces and it arbitrates between them; one consumer per
//! endpoint makes that impossible by construction. Nobody had found it because
//! the compositor's only client was `rivet-launcher`, an image service running
//! as uid 0 that never tested the rule — the first application with an
//! identity of its own was refused on `rivet-01` within seconds.
//!
//! # Why the resolution is pure
//!
//! `crate::devices::contested`'s argument, unchanged: no single definition is
//! wrong. A `connects` naming a provider that is not installed is only visible
//! to something holding every definition at once, and so is a second consumer.
//! That makes the supervisor the arbiter (§6.2) and makes this a function.

use rivet_core::package::application::Endpoint;

/// A connection that will be made: `consumer` may open `path`, which is
/// `provider`'s `endpoint`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    pub provider: String,
    pub endpoint: String,
    pub consumer: String,
    pub path: String,
    /// **How this grant is applied**, which differs by kind and not by caller.
    ///
    /// `false` — the socket is chowned to the consumer's uid at `0600`.
    /// `true` — it is `root:`[`Self::gid`] at `0660` and the consumer is
    /// spawned with that supplementary group. See the module note.
    pub shared: bool,
}

impl Grant {
    /// The group a shared endpoint's socket carries, and that its consumers are
    /// given. Meaningless for an unshared grant, which is why it is derived on
    /// demand rather than stored: one fact, one owner.
    #[must_use]
    pub fn gid(&self) -> u32 {
        rivet_core::service::endpoint_gid(&self.provider, &self.endpoint)
    }
}

/// A `connects` that will not become a [`Grant`], and the reason in the shape a
/// diagnostic needs it.
///
/// Every variant names both sides. `AGENTS.md` §5: a refusal that says "the
/// endpoint is unavailable" sends its reader nowhere, and the two names are the
/// whole of what they need to look at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The named application is not installed, or is not enabled — either way
    /// this machine has no such provider.
    NoSuchProvider { consumer: String, wanted: String },
    /// The application exists and does not provide that endpoint. Distinct from
    /// [`Refusal::NoSuchProvider`] because the fixes are different: one is an
    /// install, the other is a manifest.
    NotProvided {
        consumer: String,
        provider: String,
        endpoint: String,
    },
    /// A one-to-one endpoint that more than one application claims. Every
    /// claimant is refused and every refusal carries the whole list, so each
    /// side's journal says who else is in the contest.
    Contested {
        consumer: String,
        provider: String,
        endpoint: String,
        /// Every claimant, this one included, in name order.
        claimants: Vec<String>,
    },
}

impl core::fmt::Display for Refusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            // **"service" and not "application", corrected 2026-09-05.** The
            // sentence said *application*, which was true of every provider
            // there had been and stopped being true when `rivet-bluetoothd`
            // became one (`ADR-041` §6.2): a SYSTEM service can provide an
            // endpoint, and the commonest way to reach this refusal is now an
            // image service that is present and **disabled**. An operator told
            // that no such application is installed would go looking for one to
            // install, which is the wrong shelf.
            Self::NoSuchProvider { consumer, wanted } => write!(
                f,
                "{consumer} declares connects = [\"{wanted}\"] and no such service is enabled on \
                 this machine. A system service that exists and is switched off reaches this \
                 too, and for that one the fix is [services.<name>] enabled = true in the \
                 configuration document rather than an install"
            ),
            Self::NotProvided {
                consumer,
                provider,
                endpoint,
            } => write!(
                f,
                "{consumer} wants {provider}:{endpoint}, and {provider} is installed but its \
                 manifest does not declare provides = [\"socket:{endpoint}\"]"
            ),
            Self::Contested {
                consumer,
                provider,
                endpoint,
                claimants,
            } => write!(
                f,
                "{consumer} wants {provider}:{endpoint}, and so does every application in \
                 [{}]. That endpoint is not shared, so it has one consumer, and an endpoint \
                 more than one application claims is given to NONE of them: granting it to \
                 whichever sorts first would let any application installed later take it \
                 from the one that holds it. Remove the claim from all but one, or have \
                 {provider} declare provides = [\"socket:{endpoint}:shared\"]",
                claimants.join(", ")
            ),
        }
    }
}

/// Decide every grant and every refusal, given every definition at once.
///
/// `defs` is `(name, provides, connects)` for each **enabled** service, in a
/// stable order. No winner depends on it — rival consumers of an unshared
/// endpoint are all refused — but the order of the grants, the refusals and the
/// claimant lists in them does, and the journal should say the same thing on
/// every boot. The caller passes them sorted by name for that reason.
#[must_use]
pub fn resolve(defs: &[(String, Vec<Endpoint>, Vec<Endpoint>)]) -> (Vec<Grant>, Vec<Refusal>) {
    let mut grants: Vec<Grant> = Vec::new();
    let mut refusals = Vec::new();

    for (consumer, _, connects) in defs {
        for wanted in connects {
            let provider = wanted.owner();
            let endpoint = wanted.name();

            // **The supervisor provides one endpoint itself**, and it is not an
            // installed application, so it will never be in `defs`.
            //
            // `rivet_core::kiosk` is the kiosk control socket (`ADR-036`
            // §5.4, ruled 2026-09-01). It is bound and chowned by `crate::main`,
            // which reads the same declarations this does — so the grant is
            // already made by the time anything here runs, and there is nothing
            // for this module to add.
            //
            // Skipped rather than granted: producing a `Grant` would make
            // `crate::supervise` try to chown a path whose owner is decided at
            // boot from the whole set, and two things chowning one socket is the
            // second-authority failure `ADR-011` names. The refusal it replaces
            // was worse — the launcher declared the endpoint, was told no such
            // provider exists, and was NOT STARTED.
            if provider == rivet_core::kiosk::PROVIDER
                && endpoint == rivet_core::kiosk::ENDPOINT
            {
                continue;
            }

            let Some((_, provides, _)) = defs.iter().find(|(n, _, _)| n == provider) else {
                refusals.push(Refusal::NoSuchProvider {
                    consumer: consumer.clone(),
                    wanted: format!("{provider}:{endpoint}"),
                });
                continue;
            };

            // A `provides` entry is `socket:<name>`; its owner is the literal
            // word and its name is the endpoint. `Endpoint` keeps the two
            // halves apart so this is a comparison and not a string split.
            let Some(offered) = provides
                .iter()
                .find(|p| p.owner() == "socket" && p.name() == endpoint)
            else {
                refusals.push(Refusal::NotProvided {
                    consumer: consumer.clone(),
                    provider: provider.to_string(),
                    endpoint: endpoint.to_string(),
                });
                continue;
            };

            // **The provider decides, and it decided in its manifest.** A
            // shared endpoint takes as many consumers as declare it; see the
            // module note for what that costs and what it deliberately does
            // not do.
            let shared = offered.shared();

            // **An unshared endpoint more than one application claims is
            // given to none of them.** Counted over every definition rather
            // than over the grants made so far, so the answer does not depend
            // on which claimant the loop reaches first: name order deciding a
            // winner would let any application installed with an earlier name
            // take the endpoint from the one holding it.
            if !shared {
                let claimants: Vec<String> = defs
                    .iter()
                    .filter(|(_, _, c)| {
                        c.iter().any(|e| e.owner() == provider && e.name() == endpoint)
                    })
                    .map(|(n, _, _)| n.clone())
                    .collect();
                if claimants.len() > 1 {
                    refusals.push(Refusal::Contested {
                        consumer: consumer.clone(),
                        provider: provider.to_string(),
                        endpoint: endpoint.to_string(),
                        claimants,
                    });
                    continue;
                }
            }

            // The same endpoint declared twice by one application is one
            // claim, granted once.
            if grants
                .iter()
                .any(|g| &g.consumer == consumer && g.provider == provider && g.endpoint == endpoint)
            {
                continue;
            }

            // The path is a function of the provider's name, which is why a
            // consumer has to be told it rather than deriving it —
            // `rivet_core::service::CONNECT_ENV`.
            let Some(name) = rivet_core::service::ServiceName::new(provider) else {
                refusals.push(Refusal::NoSuchProvider {
                    consumer: consumer.clone(),
                    wanted: format!("{provider}:{endpoint}"),
                });
                continue;
            };
            grants.push(Grant {
                provider: provider.to_string(),
                endpoint: endpoint.to_string(),
                consumer: consumer.clone(),
                path: rivet_core::service::provided_socket_path(&name, endpoint),
                shared,
            });
        }
    }

    (grants, refusals)
}

/// **Who is given the supervisor's own kiosk socket**, out of every service
/// that declares `connects = ["rivet-sysd:kiosk"]`.
///
/// The socket goes to the claimant the machine document names as its launcher
/// (`crate::document::DisplayAuthority`) and to no other. Every other claimant
/// is returned to be refused by name, so an application cannot take the menu
/// away from the launcher by declaring the same endpoint. A named launcher
/// that does not declare the endpoint is not a claimant, so the socket then
/// goes to nobody.
///
/// `claimants` carries whatever the caller needs back with the name — the
/// identity to chown the socket to, in `crate::main`.
#[must_use]
pub fn kiosk_holder<T: Clone>(
    claimants: &[(String, T)],
    launcher: &str,
) -> (Option<(String, T)>, Vec<String>) {
    let holder = claimants.iter().find(|(name, _)| name == launcher).cloned();
    let refused = claimants
        .iter()
        .filter(|(name, _)| name != launcher)
        .map(|(name, _)| name.clone())
        .collect();
    (holder, refused)
}

/// The grants one consumer holds, in declaration order, as
/// [`rivet_core::service::connect_env`] wants them.
#[must_use]
pub fn env_for(consumer: &str, grants: &[Grant]) -> Option<String> {
    let mine: Vec<(String, String, String)> = grants
        .iter()
        .filter(|g| g.consumer == consumer)
        .map(|g| (g.provider.clone(), g.endpoint.clone(), g.path.clone()))
        .collect();
    rivet_core::service::connect_env(&mine)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// `provides`/`connects` are parsed from a manifest, so building one by
    /// hand here would be testing a shape the parser might not produce.
    fn endpoint(text: &str) -> Endpoint {
        Endpoint::parse("connects", text).expect("a valid owner:name")
    }

    /// **Parsed under the key it would really appear under**, which matters
    /// since 2026-09-20: `:shared` is accepted on a `provides` and refused on
    /// a `connects`, so a helper that parsed both the same way would be unable
    /// to build the fixture these tests need — and would quietly assert that
    /// the guard does not exist.
    fn provided(text: &str) -> Endpoint {
        Endpoint::parse("provides", text).expect("a valid socket:name[:shared]")
    }

    fn defs(
        rows: &[(&str, &[&str], &[&str])],
    ) -> Vec<(String, Vec<Endpoint>, Vec<Endpoint>)> {
        rows.iter()
            .map(|(name, provides, connects)| {
                (
                    (*name).to_string(),
                    provides.iter().map(|s| provided(s)).collect(),
                    connects.iter().map(|s| endpoint(s)).collect(),
                )
            })
            .collect()
    }

    /// **A SHARED endpoint takes every consumer that declares it** —
    /// `.rulings/` R-206, and the case that was impossible until it.
    ///
    /// `ADR-058` gives a compositor several clients by definition. Found on
    /// `rivet-01` when the second application ever to want a surface was
    /// refused by name, with the launcher having never tested the rule because
    /// it runs as uid 0.
    #[test]
    fn a_shared_endpoint_grants_every_consumer_that_declares_it() {
        let (grants, refusals) = resolve(&defs(&[
            ("rivet-displayd", &["socket:wayland:shared"], &[]),
            ("aura-renderer", &[], &["rivet-displayd:wayland"]),
            ("gpu-surface", &[], &["rivet-displayd:wayland"]),
        ]));
        assert!(refusals.is_empty(), "{refusals:?}");
        assert_eq!(grants.len(), 2, "both consumers must be granted");
        assert!(grants.iter().all(|g| g.shared), "both grants are shared");
        // One socket, and the group is what distinguishes who may open it.
        assert_eq!(grants[0].path, grants[1].path);
        assert_eq!(grants[0].gid(), grants[1].gid());
        assert_ne!(
            grants[0].gid(),
            0,
            "root is not an endpoint group: a gid of 0 would grant the root group"
        );
    }

    /// **And an UNSHARED endpoint is unchanged**, which is the half that keeps
    /// `connects` meaning something. Sharing is the provider's declaration and
    /// never a default.
    #[test]
    fn an_unshared_endpoint_takes_at_most_one_consumer() {
        let (grants, refusals) = resolve(&defs(&[
            ("rivet-displayd", &["socket:capture"], &[]),
            ("watcher-a", &[], &["rivet-displayd:capture"]),
        ]));
        assert_eq!(grants.len(), 1, "one consumer, as ever");
        assert!(!grants[0].shared);
        assert!(refusals.is_empty(), "{refusals:?}");

        let (grants, refusals) = resolve(&defs(&[
            ("rivet-displayd", &["socket:capture"], &[]),
            ("watcher-a", &[], &["rivet-displayd:capture"]),
            ("watcher-b", &[], &["rivet-displayd:capture"]),
        ]));
        assert!(grants.is_empty(), "two claimants, and neither is given it");
        assert_eq!(refusals.len(), 2, "and both are told who else claims it");
    }

    /// **A consumer cannot declare an endpoint shared.** Whether two readers
    /// are possible is a fact about what is listening; letting the asker
    /// assert it would be a consumer granting itself a property of the
    /// provider.
    #[test]
    fn shared_is_refused_on_a_connects() {
        assert!(Endpoint::parse("connects", "rivet-displayd:wayland:shared").is_err());
        assert!(Endpoint::parse("provides", "socket:wayland:shared").is_ok());
    }

    #[test]
    fn a_matched_pair_is_granted() {
        let (grants, refusals) = resolve(&defs(&[
            ("aura-controller", &["socket:kiosk.sock"], &[]),
            ("aura-renderer", &[], &["aura-controller:kiosk.sock"]),
        ]));
        assert!(refusals.is_empty(), "{refusals:?}");
        assert_eq!(grants.len(), 1);
        assert_eq!(grants[0].consumer, "aura-renderer");
        assert_eq!(grants[0].provider, "aura-controller");
        assert_eq!(
            grants[0].path,
            "/run/rivet/listen/aura-controller/kiosk.sock"
        );
    }

    /// **A one-to-one endpoint two applications claim is refused to BOTH**,
    /// and each refusal names every claimant.
    ///
    /// First-come in name order would let any application installed with a
    /// name sorting earlier take an endpoint from the one that holds it, so
    /// the contest itself is the fault and neither side wins it.
    #[test]
    fn a_one_to_one_endpoint_two_applications_claim_is_refused_to_both() {
        let (grants, refusals) = resolve(&defs(&[
            ("a-provider", &["socket:api"], &[]),
            ("b-first", &[], &["a-provider:api"]),
            ("c-second", &[], &["a-provider:api"]),
        ]));
        assert!(grants.is_empty(), "a contested endpoint was granted: {grants:?}");
        let claimants = vec!["b-first".to_string(), "c-second".to_string()];
        assert_eq!(
            refusals,
            vec![
                Refusal::Contested {
                    consumer: "b-first".to_string(),
                    provider: "a-provider".to_string(),
                    endpoint: "api".to_string(),
                    claimants: claimants.clone(),
                },
                Refusal::Contested {
                    consumer: "c-second".to_string(),
                    provider: "a-provider".to_string(),
                    endpoint: "api".to_string(),
                    claimants,
                },
            ]
        );
        for refusal in &refusals {
            let text = refusal.to_string();
            assert!(
                text.contains("b-first") && text.contains("c-second"),
                "the refusal must name both claimants: {text}"
            );
        }
    }

    /// The contest is per endpoint: an uncontested endpoint of the same
    /// provider is still granted, and one application declaring the same
    /// endpoint twice is one claimant, not two.
    #[test]
    fn a_contest_touches_only_the_contested_endpoint() {
        let (grants, refusals) = resolve(&defs(&[
            ("a-provider", &["socket:api", "socket:other"], &[]),
            ("b-first", &[], &["a-provider:api"]),
            ("c-second", &[], &["a-provider:api"]),
            ("d-alone", &[], &["a-provider:other", "a-provider:other"]),
        ]));
        assert_eq!(refusals.len(), 2, "{refusals:?}");
        assert_eq!(grants.len(), 1, "{grants:?}");
        assert_eq!(grants[0].consumer, "d-alone");
        assert_eq!(grants[0].endpoint, "other");
    }

    #[test]
    fn an_absent_provider_is_distinguished_from_one_that_does_not_provide() {
        let (grants, refusals) = resolve(&defs(&[
            ("installed", &["socket:other"], &[]),
            ("wants-missing", &[], &["not-here:api"]),
            ("wants-wrong", &[], &["installed:api"]),
        ]));
        assert!(grants.is_empty());
        assert_eq!(refusals.len(), 2);
        assert!(matches!(refusals[0], Refusal::NoSuchProvider { .. }));
        assert!(matches!(refusals[1], Refusal::NotProvided { .. }));
    }

    /// **The supervisor's own endpoint is not refused**, and the board found
    /// this before a test did: the launcher declared
    /// `connects = ["rivet-sysd:kiosk"]`, `rivet-sysd` is not an installed
    /// application so it was not in the set, and the launcher was NOT STARTED.
    #[test]
    fn the_supervisors_own_endpoint_is_neither_granted_nor_refused() {
        let wanted = format!(
            "{}:{}",
            rivet_core::kiosk::PROVIDER,
            rivet_core::kiosk::ENDPOINT
        );
        let (grants, refusals) = resolve(&defs(&[("rivet-launcher", &[], &[&wanted])]));
        assert!(refusals.is_empty(), "the launcher was refused: {refusals:?}");
        // And no grant either: `crate::main` chowns that socket, and a second
        // thing doing it would be two authorities over one file.
        assert!(grants.is_empty(), "something else tried to grant it: {grants:?}");
    }

    /// A `provides` nobody connects to is not a fault. The socket is still
    /// created — it is a `listen` — and the application is simply alone.
    #[test]
    fn an_unconsumed_endpoint_is_not_a_refusal() {
        let (grants, refusals) = resolve(&defs(&[("lonely", &["socket:api"], &[])]));
        assert!(grants.is_empty());
        assert!(refusals.is_empty());
    }

    /// **The kiosk socket goes to the named launcher and to nobody else**,
    /// however many applications declare it, and every other claimant is
    /// handed back to be refused by name.
    #[test]
    fn the_kiosk_socket_goes_only_to_the_named_launcher() {
        let claimants = vec![
            ("a-rogue".to_string(), 1),
            ("rivet-launcher".to_string(), 2),
            ("z-rogue".to_string(), 3),
        ];
        let (holder, refused) = kiosk_holder(&claimants, "rivet-launcher");
        assert_eq!(holder, Some(("rivet-launcher".to_string(), 2)));
        assert_eq!(refused, vec!["a-rogue".to_string(), "z-rogue".to_string()]);

        // The named launcher alone is the ordinary machine.
        let (holder, refused) = kiosk_holder(&claimants[1..2], "rivet-launcher");
        assert_eq!(holder, Some(("rivet-launcher".to_string(), 2)));
        assert!(refused.is_empty());

        // A launcher that does not declare the endpoint is not a claimant, and
        // the socket does not fall to whoever does.
        let (holder, refused) = kiosk_holder(&claimants[..1], "rivet-launcher");
        assert_eq!(holder, None);
        assert_eq!(refused, vec!["a-rogue".to_string()]);
    }

    #[test]
    fn the_environment_names_only_this_consumer_s_grants() {
        let (grants, _) = resolve(&defs(&[
            ("p-one", &["socket:a"], &[]),
            ("p-two", &["socket:b"], &[]),
            ("mine", &[], &["p-one:a"]),
            ("theirs", &[], &["p-two:b"]),
        ]));
        assert_eq!(
            env_for("mine", &grants).as_deref(),
            Some("p-one:a=/run/rivet/listen/p-one/a")
        );
        assert_eq!(env_for("nobody", &grants), None);
    }
}
