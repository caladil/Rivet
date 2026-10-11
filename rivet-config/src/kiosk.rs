//! `[kiosk]` — **which one of the offered applications actually gets the
//! screen.**
//!
//! The maintainer's ruling, 2026-08-31 (`.rulings/` R-321): any number of kiosk
//! applications may be installed and enabled, and exactly one of them can be the
//! one that launches automatically. `enabled` puts an application on the
//! launcher's list; the launcher itself holds one `auto-run`. An application can
//! be installed without being enabled, and the launcher then does not offer it.
//!
//! # Three states, and `enabled` moved
//!
//! | the document says | the launcher | the screen |
//! | :-- | :-- | :-- |
//! | nothing | does not offer it | — |
//! | `enabled = true` | offers it | — |
//! | `enabled = true` **and** `[kiosk] auto-run` names it | offers it | **it runs** |
//!
//! **`enabled` used to mean RUNS and now means OFFERED**, for display-declaring
//! applications only. Everything else in the document is untouched:
//! `[services.rivet-mcpd]`'s `enabled` still starts a daemon, because a daemon
//! declares no display and nothing arbitrates over it.
//!
//! # Why this is one key naming a service, rather than a flag on each
//!
//! `[services.<name>] auto-run = true` would be the obvious shape and it can
//! express a state that must not exist: **two of them saying `true`.** Something
//! would then have to decide which wins, in a rule an operator has to know
//! before reading the document. One key holding one name cannot say it — the
//! wrong state is unrepresentable rather than refused, which `ADR-007` §3.1
//! prefers wherever it is available.
//!
//! # What it does NOT do
//!
//! **It does not name a service that must exist.** A document is authored
//! anywhere, for a machine that may not have the application yet (`ADR-007`
//! §3.6.3), so a name matching nothing installed is kept and reported rather
//! than refused — the same rule `[services.<name>]` already follows for a
//! service this image does not define. What it costs is a screen with nothing
//! on it, which is visible.
//!
//! **It does not start anything on its own.** A named application that is not
//! `enabled` is not offered and does not run; `auto-run` selects from what
//! `enabled` offers rather than overriding it. Naming a disabled application is
//! a contradiction the operator can write, and it resolves the safe way: the
//! screen stays empty and the journal says which key disagreed with which.
//!
//! # `launcher` and `compositor` name who may hold the two display roles
//!
//! A manifest can ask to be the launcher or the compositor by declaring the
//! endpoint the role uses, and the supervisor gives the role only to the
//! service this table names for it. Both keys are optional and default to the
//! image's own services, which the supervisor holds, so a document that names
//! neither means what every document meant before they existed.

/// `[kiosk]` — the display's owner, by name.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Kiosk {
    auto_run: Option<String>,
    locked: bool,
    launcher: Option<String>,
    compositor: Option<String>,
}

impl Kiosk {
    /// Every key this table accepts. An unknown one is refused by name, on
    /// `crate::splash`'s rule: a misspelled setting that parses is a setting
    /// that silently does nothing.
    pub const DECLARED_KEYS: &'static [&'static str] =
        &["auto-run", "locked", "launcher", "compositor"];

    /// What the value has to be, for a refusal that says so.
    pub const RULE: &'static str = "the name of an installed application to give the screen to";

    /// What `launcher` and `compositor` have to be, for a refusal that says so.
    pub const ROLE_RULE: &'static str = "the name of a service on this machine";

    /// What `locked` has to be, for a refusal that says so.
    pub const LOCKED_RULE: &'static str =
        "true to run the auto-run application with no way back to the menu, or false";

    /// Build one.
    #[must_use]
    pub fn new(auto_run: Option<String>) -> Self {
        Self {
            auto_run,
            locked: false,
            launcher: None,
            compositor: None,
        }
    }

    /// Build one that is locked.
    #[must_use]
    pub fn locked_on(auto_run: Option<String>) -> Self {
        Self {
            auto_run,
            locked: true,
            launcher: None,
            compositor: None,
        }
    }

    /// The same table, naming which service is this machine's launcher.
    #[must_use]
    pub fn with_launcher(mut self, launcher: Option<String>) -> Self {
        self.launcher = launcher;
        self
    }

    /// The same table, naming which service is this machine's compositor.
    #[must_use]
    pub fn with_compositor(mut self, compositor: Option<String>) -> Self {
        self.compositor = compositor;
        self
    }

    /// **Which service is this machine's launcher**, if the document named
    /// one.
    ///
    /// A manifest that declares `connects = ["rivet-sysd:kiosk"]` is asking to
    /// be the launcher, and the supervisor honours that only for the service
    /// the document names here. Without this, any application could claim the
    /// role by declaring it, and a second claimant took the kiosk socket away
    /// from the real launcher. `None` means the image's own launcher; the
    /// supervisor holds that default, so a document written before this key
    /// existed keeps meaning what it did.
    #[must_use]
    pub fn launcher(&self) -> Option<&str> {
        self.launcher.as_deref()
    }

    /// **Which service is this machine's compositor** — the display arbiter
    /// whenever it is enabled — if the document named one.
    ///
    /// A manifest that declares `provides = ["socket:wayland"]` is offering to
    /// drive the screen, and the supervisor gives the screen only to the
    /// service named here. A desktop machine names a different compositor in
    /// its document and needs no supervisor change. `None` means the image's
    /// own compositor.
    #[must_use]
    pub fn compositor(&self) -> Option<&str> {
        self.compositor.as_deref()
    }

    /// **Whether there is any way back to the launcher's menu.**
    ///
    /// `PLAN.md` §108.2. A locked machine runs its [`Self::auto_run`]
    /// application and nothing else: the held back control does nothing, and so
    /// does `apply input long-back`, because `ADR-012` §9.3 rules that an agent
    /// may press what a person may press and no more. An operator is not stuck
    /// — the control plane can clear this key — but the lock does not weaken
    /// for whoever is asking, or it is not a lock.
    ///
    /// **`locked` without `auto-run` is a machine with nothing to show**, and
    /// it is kept rather than refused for the same reason a name matching
    /// nothing installed is: a document is authored anywhere (`ADR-007`
    /// §3.6.3). It resolves visibly, as `E_KIOSK_NO_AUTOLAUNCH` on the screen.
    #[must_use]
    pub const fn locked(&self) -> bool {
        self.locked
    }

    /// Which application should hold the display, if the document named one.
    #[must_use]
    pub fn auto_run(&self) -> Option<&str> {
        self.auto_run.as_deref()
    }

    /// **Whether this table says anything at all.**
    ///
    /// `[kiosk]` with no keys is a table that states nothing, and
    /// `crate::serialise` must not write one back — two documents meaning the
    /// same thing would otherwise produce different bytes (`ADR-007` §3.1).
    #[must_use]
    pub fn is_silent(&self) -> bool {
        self.auto_run.is_none()
            && !self.locked
            && self.launcher.is_none()
            && self.compositor.is_none()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;

    /// The whole point of the shape: one name, so two cannot be named.
    #[test]
    fn one_name_is_all_this_table_can_hold() {
        assert_eq!(
            Kiosk::DECLARED_KEYS,
            &["auto-run", "locked", "launcher", "compositor"]
        );
        let k = Kiosk::new(Some("aura".to_string()));
        assert_eq!(k.auto_run(), Some("aura"));
    }

    /// **The two role names are something the table says**, each on its own,
    /// so `crate::serialise` writes back a table that names only a launcher.
    #[test]
    fn a_named_launcher_or_compositor_alone_is_not_silence() {
        let launcher = Kiosk::default().with_launcher(Some("my-launcher".to_string()));
        assert!(!launcher.is_silent());
        assert_eq!(launcher.launcher(), Some("my-launcher"));
        assert_eq!(launcher.compositor(), None);

        let compositor = Kiosk::default().with_compositor(Some("cosmic-comp".to_string()));
        assert!(!compositor.is_silent());
        assert_eq!(compositor.compositor(), Some("cosmic-comp"));
    }

    /// An empty table states nothing, and `crate::serialise` uses this to
    /// decide not to write one back.
    #[test]
    fn a_table_with_no_keys_is_silent() {
        assert!(Kiosk::default().is_silent());
        assert!(!Kiosk::new(Some("x".to_string())).is_silent());
    }

    /// **`locked` alone is not silence.** A table saying only `locked = true`
    /// states something — a machine that will show an error rather than a menu
    /// — so `crate::serialise` has to write it back or the document would lose
    /// the key on the next edit.
    #[test]
    fn locked_alone_is_something_the_table_says() {
        assert!(!Kiosk::locked_on(None).is_silent());
        assert!(Kiosk::locked_on(None).locked());
        assert!(!Kiosk::new(Some("x".to_string())).locked());
    }
}
