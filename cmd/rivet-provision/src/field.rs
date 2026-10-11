//! **What a provisioning document can say, as a list of questions.**
//!
//! # Validation happens HERE, at entry
//!
//! Ruled 2026-09-01, and it is the reason red is worth having: *red at write
//! time is a rejection, red already on the screen is a state you can see and
//! fix.* A field checks its own answer the moment it is given one, so
//! `560.10.22.384` is refused while the operator is still looking at it.
//!
//! # And the document is validated by the SAME parser the board refuses with
//!
//! Per-field checks catch a typo. They cannot catch a document that is
//! well-formed and wrong — `method = "dhcp"` beside an `address`, say. Only
//! `rivet_config::Document::parse` knows that, and [`crate::compose`] runs it.
//!
//! `tools/card/provisioning.example.toml` states the defect this closes:
//! *"`cmd/rivet-image` does not validate this… the refusal comes from the
//! board, on first boot, which is the wrong end."*

use crate::screen::State;

/// What a field accepts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// Free text, checked by the `check` function passed to [`Field::set`].
    Text,
    /// One of a fixed set. The menu offers them by number.
    Choice(Vec<String>),
}

/// One question.
#[derive(Debug, Clone)]
pub struct Field {
    /// One or two words. Never a sentence.
    pub label: &'static str,
    /// What it holds now.
    pub value: String,
    /// What it held before anybody touched it.
    pub default: String,
    /// What it accepts.
    pub kind: Kind,
    /// Why the current value will not do, if it will not.
    pub error: Option<String>,
    /// The prose, behind `?` and never on the screen.
    pub explain: &'static str,
}

impl Field {
    /// A text field with a default.
    #[must_use]
    pub fn text(label: &'static str, default: &str, explain: &'static str) -> Self {
        Self {
            label,
            value: default.to_owned(),
            default: default.to_owned(),
            kind: Kind::Text,
            error: None,
            explain,
        }
    }

    /// A field that takes one of a fixed set. The first is the default.
    #[must_use]
    pub fn choice(label: &'static str, of: &[&str], explain: &'static str) -> Self {
        Self {
            label,
            value: of.first().map_or_else(String::new, |f| (*f).to_owned()),
            default: of.first().map_or_else(String::new, |f| (*f).to_owned()),
            kind: Kind::Choice(of.iter().map(|s| (*s).to_owned()).collect()),
            error: None,
            explain,
        }
    }

    /// How this field should be coloured.
    #[must_use]
    pub fn state(&self) -> State {
        if self.error.is_some() {
            State::Invalid
        } else if self.value == self.default {
            State::Default
        } else {
            State::Changed
        }
    }

    /// Take an answer, and say whether it stands.
    ///
    /// **The value is stored either way.** A refused answer that vanishes
    /// leaves the operator retyping something they can no longer see, and the
    /// screen is where the error belongs.
    pub fn set(&mut self, answer: &str, check: fn(&str) -> Result<(), String>) {
        self.value = answer.trim().to_owned();
        self.error = match &self.kind {
            Kind::Choice(of) if !of.iter().any(|o| o == &self.value) => {
                Some(format!("one of {}", of.join(", ")))
            }
            _ => check(&self.value).err(),
        };
    }
}

/// **Is this an address with its prefix?** — checked by `rivet-core`'s own
/// type, not by a rule of ours.
///
/// The maintainer's example is the case that must not get through: an invalid
/// address such as `560.10.22.384` is rejected (.rulings/ R-340). But the
/// subtler one is a bare `192.168.50.101` — four valid octets and **no
/// prefix**, which this tool accepted on its first run and the schema would
/// then have refused at write time. `Ipv4Cidr::RULE` says *"the prefix is required"*, so a check
/// here that did not know that put the refusal back on the far end this whole
/// crate exists to move.
///
/// # Errors
///
/// A short reason. It goes on a screen, so it is a fragment and not a
/// paragraph.
pub fn ipv4_cidr(text: &str) -> Result<(), String> {
    match rivet_core::net::Ipv4Cidr::parse(text) {
        Ok(_) => Ok(()),
        Err(_) if !text.contains('/') => Err("needs a prefix, e.g. /24".to_owned()),
        Err(_) => Err("not an address".to_owned()),
    }
}

/// **Is this a plain address?** For a gateway, which carries no prefix.
///
/// # Errors
///
/// A short reason, for the same place.
pub fn ipv4(text: &str) -> Result<(), String> {
    if text.contains('/') {
        return Err("no prefix on a gateway".to_owned());
    }
    let octets: Vec<&str> = text.split('.').collect();
    if octets.len() != 4 {
        return Err("four octets".to_owned());
    }
    for o in octets {
        match o.parse::<u16>() {
            Ok(n) if n <= 255 && !o.is_empty() => {}
            _ => return Err(format!("{o} is not 0-255")),
        }
    }
    Ok(())
}

/// **The gateway this address probably implies.**
///
/// `192.168.50.101/24` gives `192.168.50.1`. Not a guess about the network so
/// much as about the person: the router is at `.1` on essentially every home
/// LAN, and a default the operator overwrites costs them nothing while a blank
/// one costs them a lookup.
///
/// `None` when the address is not usable yet, because a suggestion derived
/// from nonsense is worse than none.
#[must_use]
pub fn implied_gateway(address: &str) -> Option<String> {
    let bare = address.split_once('/')?.0;
    let mut octets: Vec<&str> = bare.split('.').collect();
    if octets.len() != 4 || ipv4(bare).is_err() {
        return None;
    }
    octets[3] = "1";
    Some(octets.join("."))
}

/// The entries of a Nameservers answer, in the order typed.
///
/// **Separated by commas or whitespace**, as the authority's suffix list is: a
/// person typing two of something types them the way they think of them.
/// Empty entries fall out, so a trailing comma is a typo rather than a fourth
/// nameserver.
#[must_use]
pub fn nameserver_list(text: &str) -> Vec<&str> {
    text.split([',', ' ', '\t'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect()
}

/// **Is this a list of nameservers `[network.resolver]` will take?**
///
/// The refusals are `rivet_config`'s own, made at the prompt rather than at
/// `w`: every entry must be an IPv4 address, parsed the way its parser parses
/// one, and there may be at most [`rivet_config::resolver::Resolver::MAX`]
/// of them, because musl asks three and ignores the rest.
///
/// **One refusal is this tool's and not the parser's: an empty list.** The
/// parser accepts `nameservers = []` as a table that says nothing, which on a
/// static address is a machine whose every lookup fails. This field is asked
/// only for a static address, and a bare Return takes the gateway, so an empty
/// answer is reachable only when there is no gateway either.
///
/// # Errors
///
/// A short reason naming the entry, or the count, that will not do.
pub fn nameservers(text: &str) -> Result<(), String> {
    let list = nameserver_list(text);
    if list.is_empty() {
        return Err("at least one, e.g. the router".to_owned());
    }
    if let Some(bad) = list.iter().find(|one| one.parse::<std::net::Ipv4Addr>().is_err()) {
        return Err(format!("{bad} is not an IPv4 address"));
    }
    let max = rivet_config::resolver::Resolver::MAX;
    if list.len() > max {
        return Err(format!("{} given, at most {max}: musl asks no more", list.len()));
    }
    Ok(())
}

/// **Is this a hostname?** RFC 1123's letters, digits and hyphens, not starting
/// or ending with one.
///
/// # Errors
///
/// A short reason, for the same place.
pub fn hostname(text: &str) -> Result<(), String> {
    if text.is_empty() || text.len() > 63 {
        return Err("1-63 characters".to_owned());
    }
    if text.starts_with('-') || text.ends_with('-') {
        return Err("no leading or trailing -".to_owned());
    }
    if !text.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err("letters, digits and - only".to_owned());
    }
    Ok(())
}

/// Anything goes.
///
/// # Errors
///
/// Never.
pub const fn anything(_: &str) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;

    /// **The maintainer's example, and it is the test.**
    #[test]
    fn the_impossible_address_is_refused() {
        assert!(ipv4("560.10.22.384").is_err());
        assert!(ipv4("192.168.1.50").is_ok());
        assert!(ipv4("192.168.1").is_err());
        assert!(ipv4("192.168.1.").is_err());
    }

    /// **A bare address is refused, and this is the one the tool got wrong.**
    /// Four valid octets with no prefix passed the first version and the schema
    /// would have refused it at write time.
    #[test]
    fn an_address_without_its_prefix_is_refused_here_not_later() {
        assert!(ipv4_cidr("192.168.50.101").is_err());
        assert!(ipv4_cidr("192.168.50.101/24").is_ok());
        let why = ipv4_cidr("192.168.50.101").unwrap_err();
        assert!(why.contains("prefix"), "{why}");
    }

    /// A gateway carries no prefix, and saying so beats a confusing refusal
    /// from somewhere else.
    #[test]
    fn a_gateway_with_a_prefix_is_refused() {
        assert!(ipv4("192.168.50.1/24").is_err());
        assert!(ipv4("192.168.50.1").is_ok());
    }

    /// **The prediction the maintainer asked for.**
    #[test]
    fn the_gateway_is_implied_by_the_address() {
        assert_eq!(
            implied_gateway("192.168.50.101/24").as_deref(),
            Some("192.168.50.1")
        );
        assert_eq!(implied_gateway("192.168.50.101"), None);
        assert_eq!(implied_gateway("nonsense/24"), None);
    }

    /// Hostnames, including the two ends people actually get wrong.
    #[test]
    fn hostnames_are_checked_at_both_ends() {
        assert!(hostname("rivet-401ac").is_ok());
        assert!(hostname("-rivet").is_err());
        assert!(hostname("rivet-").is_err());
        assert!(hostname("rivet_401").is_err());
        assert!(hostname("").is_err());
    }

    /// **A refused value stays on the screen.** Losing it means retyping
    /// something you can no longer see.
    #[test]
    fn a_refused_answer_is_kept_and_marked() {
        let mut f = Field::text("Gateway", "192.168.1.1", "");
        f.set("560.10.22.384", ipv4);
        assert_eq!(f.value, "560.10.22.384");
        assert_eq!(f.state(), State::Invalid);
        assert!(f.error.is_some());
    }

    /// Back to the default is back to dim — a field you changed and changed
    /// back is not a change.
    #[test]
    fn returning_to_the_default_is_not_a_change() {
        let mut f = Field::text("Hostname", "rivet-401ac", "");
        assert_eq!(f.state(), State::Default);
        f.set("lounge", hostname);
        assert_eq!(f.state(), State::Changed);
        f.set("rivet-401ac", hostname);
        assert_eq!(f.state(), State::Default);
    }

    /// A choice refuses anything outside the set, and says the set.
    #[test]
    fn a_choice_names_what_it_would_have_taken() {
        let mut f = Field::choice("Mode", &["headless", "kiosk"], "");
        f.set("desktop", anything);
        assert_eq!(f.state(), State::Invalid);
        assert!(f.error.as_deref().unwrap_or_default().contains("headless"));
    }
}
