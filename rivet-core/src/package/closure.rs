//! **What every installed application declares**, read once for the two things
//! on a machine that remove a library: retention at the install edge
//! (`SPEC-02` §12) and an operator's `apply uninstall-library`.
//!
//! # Why one reader, in the core, behind a port
//!
//! A library named in an application's declared closure (`SPEC-02` §5) is
//! load-bearing. The application resolves it by exact reference at every start,
//! so removing it stops that application at its next one. Retention must never
//! evict such a library, and `apply uninstall-library` must refuse to remove
//! one — and the second is checked twice, by the command that plans it and by
//! the supervisor that performs it. Three readers of one fact in two processes
//! would be three chances to disagree about which manifests count, so there is
//! one, here, and each process hands it its own view of the filesystem through
//! [`Tree`].
//!
//! # An unknown closure is never a partial one
//!
//! A manifest that cannot be read is an application whose closure is unknown,
//! and a guard computed without it is a guard with a hole in it — the hole is
//! the application that will not start after its next restart. So [`declared`]
//! answers for every application or for none: the first manifest it cannot read
//! ends the reading with [`Unknown`], and every caller then removes nothing.
//!
//! # Two directories that are not unknown closures
//!
//! An image service's home under the application root (`ADR-060`) and an
//! application uninstalled with its data kept both have no manifest by their
//! ordinary shape rather than by a fault. Neither declares anything, and
//! treating either as an unreadable manifest would stop every removal on every
//! machine that has one.

use crate::package::application::Manifest;
use crate::package::library::Reference;
use crate::service::{self, ServiceName};

/// A volume's tree, read-only, as [`declared`] needs it.
///
/// **A domain port** (`AGENTS.md` §3): paths and text, nothing a platform
/// names. Two adapters implement it — the supervisor's kernel and the
/// control-plane CLI's request store — and both list **directories only**,
/// passing over a symbolic link, because that is what the supervisor's own
/// discovery sees. An adapter that listed more would disagree with discovery
/// about which entries are applications.
pub trait Tree {
    /// The directories in `path`, by name, or `None` when `path` is not there.
    ///
    /// # Errors
    ///
    /// What the filesystem said, when `path` is there and cannot be listed —
    /// which is a different answer from `None`, and is never reported as one.
    fn directories(&mut self, path: &str) -> Result<Option<Vec<String>>, String>;

    /// Whether anything is at `path`.
    ///
    /// # Errors
    ///
    /// What the filesystem said, when the question could not be answered.
    fn exists(&mut self, path: &str) -> Result<bool, String>;

    /// A text file's contents.
    ///
    /// # Errors
    ///
    /// What the filesystem said — **including that nothing is there**, because
    /// every caller of this reads a file that should be.
    fn read(&mut self, path: &str) -> Result<String, String>;
}

/// One installed application, as far as a library's removal cares about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaring {
    /// The application's name, which is its directory under the application
    /// root.
    pub name: ServiceName,
    /// Its manifest's `library` key: the builds it resolves by exact reference
    /// at every start, in the order its author wrote them.
    pub library: Vec<Reference>,
    /// Its manifest's `accepts-class` key: the classes a library joins it by.
    pub accepts_classes: Vec<String>,
}

/// Why what the installed applications declare is not known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unknown {
    /// The application root is there and could not be listed.
    Unlisted { path: String, detail: String },
    /// An application with something installed has no manifest that reads.
    Unread {
        application: String,
        path: String,
        detail: String,
    },
    /// An application's manifest reads and does not parse. `detail` names a
    /// key, never a value.
    Unparsed {
        application: String,
        path: String,
        detail: String,
    },
}

impl Unknown {
    /// The path that could not be read.
    #[must_use]
    pub fn path(&self) -> &str {
        match self {
            Self::Unlisted { path, .. } | Self::Unread { path, .. } | Self::Unparsed { path, .. } => {
                path
            }
        }
    }

    /// What the filesystem or the parser said.
    #[must_use]
    pub fn detail(&self) -> &str {
        match self {
            Self::Unlisted { detail, .. }
            | Self::Unread { detail, .. }
            | Self::Unparsed { detail, .. } => detail,
        }
    }

    /// The application whose manifest is the hole, when one is.
    #[must_use]
    pub fn application(&self) -> Option<&str> {
        match self {
            Self::Unlisted { .. } => None,
            Self::Unread { application, .. } | Self::Unparsed { application, .. } => {
                Some(application)
            }
        }
    }

    /// One clause, naming what could not be read.
    #[must_use]
    pub fn why(&self) -> String {
        match self {
            Self::Unlisted { path, detail } => {
                format!("the application directory {path} could not be listed ({detail})")
            }
            Self::Unread {
                application,
                path,
                detail,
            } => format!(
                "the installed application `{application}` has no readable manifest at {path} \
                 ({detail})"
            ),
            Self::Unparsed {
                application,
                path,
                detail,
            } => format!(
                "the installed application `{application}`'s manifest at {path} does not parse \
                 ({detail})"
            ),
        }
    }
}

/// What every installed application on `volume` declares, or why that is not
/// known.
///
/// An entry that is not a service name is not an application this machine
/// would ever start and is passed over, as discovery passes it over. So are an
/// image service's home with no manifest and an application holding nothing but
/// its data — see this module's documentation.
///
/// # Errors
///
/// [`Unknown`], at the first application whose closure cannot be read. The
/// answer is never a partial list.
pub fn declared(tree: &mut dyn Tree, volume: &str) -> Result<Vec<Declaring>, Unknown> {
    let root = service::app_root(volume);
    let entries = match tree.directories(&root) {
        Ok(Some(entries)) => entries,
        // Nothing is installed, so nothing declares anything — the commonest
        // case, and discovery reads it the same way.
        Ok(None) => return Ok(Vec::new()),
        Err(detail) => return Err(Unknown::Unlisted { path: root, detail }),
    };

    let mut out = Vec::new();
    for entry in entries {
        let Some(name) = ServiceName::new(&entry) else {
            continue;
        };
        let path = service::manifest_path(volume, &name);
        // An image service's directory: no `current` and no manifest is its
        // ordinary shape. One that DOES hold a manifest is read as usual.
        if service::IMAGE_SERVICE_NAMES.contains(&entry.as_str()) && tree.exists(&path) == Ok(false)
        {
            continue;
        }
        // An application with no versions: an uninstall that kept the data
        // leaves `data` and whatever its removal set aside, and nothing else.
        if tree.exists(&path) == Ok(false)
            && holds_no_versions(tree, &service::app_dir(volume, &name))
        {
            continue;
        }
        let text = tree.read(&path).map_err(|detail| Unknown::Unread {
            application: name.to_string(),
            path: path.clone(),
            detail,
        })?;
        let manifest = Manifest::parse(text.as_bytes()).map_err(|fault| Unknown::Unparsed {
            application: name.to_string(),
            path: path.clone(),
            detail: fault.to_string(),
        })?;
        out.push(Declaring {
            library: manifest.library().to_vec(),
            accepts_classes: manifest.accepts_classes().to_vec(),
            name,
        });
    }
    Ok(out)
}

/// Whether an application's directory holds nothing but its `data` and what a
/// removal has set aside. `false` when it cannot be listed, so a directory this
/// cannot see into stays an unknown closure.
fn holds_no_versions(tree: &mut dyn Tree, app_dir: &str) -> bool {
    matches!(
        tree.directories(app_dir),
        Ok(Some(entries)) if entries
            .iter()
            .all(|e| e == service::DATA_DIR || e.contains(service::RECLAIM_MARK))
    )
}

/// A [`Tree`] in maps, for the tests of this crate. **Hand-composed**: every
/// entry is one a test put there.
#[cfg(test)]
#[derive(Debug, Default)]
pub(crate) struct MemoryTree {
    pub directories: std::collections::BTreeMap<String, Vec<String>>,
    pub files: std::collections::BTreeMap<String, String>,
    /// Paths whose listing fails rather than answering.
    pub unlistable: std::collections::BTreeMap<String, String>,
}

#[cfg(test)]
impl Tree for MemoryTree {
    fn directories(&mut self, path: &str) -> Result<Option<Vec<String>>, String> {
        if let Some(detail) = self.unlistable.get(path) {
            return Err(detail.clone());
        }
        Ok(self.directories.get(path).cloned())
    }

    fn exists(&mut self, path: &str) -> Result<bool, String> {
        Ok(self.files.contains_key(path) || self.directories.contains_key(path))
    }

    fn read(&mut self, path: &str) -> Result<String, String> {
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| "No such file or directory".to_string())
    }
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::package::application::ProgramKind;
    use crate::service::BUILT_IN_VOLUME;

    fn name(text: &str) -> ServiceName {
        ServiceName::new(text).expect("a service name")
    }

    fn reference(product: &str, byte: u8) -> Reference {
        Reference::compose(product, "1.0.0", &[byte; 32]).expect("a reference")
    }

    /// An application whose manifest is rendered by the manifest's own writer.
    fn install(tree: &mut MemoryTree, app: &str, library: &[Reference], accepts: &[&str]) {
        tree.directories
            .entry(service::app_root(BUILT_IN_VOLUME))
            .or_default()
            .push(app.to_string());
        let manifest = Manifest::new(ProgramKind::Daemon)
            .with_library(library.to_vec())
            .expect("a valid closure")
            .with_accepts_classes(accepts.iter().map(ToString::to_string).collect())
            .expect("valid classes");
        tree.files.insert(
            service::manifest_path(BUILT_IN_VOLUME, &name(app)),
            manifest.render(),
        );
    }

    /// No application root is nothing installed, not a fault.
    #[test]
    fn no_application_root_declares_nothing() {
        assert_eq!(declared(&mut MemoryTree::default(), BUILT_IN_VOLUME), Ok(Vec::new()));
    }

    /// The closure and the accepted classes come back exactly as the manifest
    /// declares them.
    #[test]
    fn an_applications_closure_and_classes_are_read_from_its_manifest() {
        let mut tree = MemoryTree::default();
        let lib = reference("libgreet", 0xaa);
        install(&mut tree, "greeter", std::slice::from_ref(&lib), &["aura-background-pack"]);

        let got = declared(&mut tree, BUILT_IN_VOLUME).expect("readable");
        assert_eq!(
            got,
            vec![Declaring {
                name: name("greeter"),
                library: vec![lib],
                accepts_classes: vec!["aura-background-pack".to_string()],
            }]
        );
    }

    /// **The hole.** An application directory with a version in it and no
    /// manifest is an unknown closure, and the answer names it.
    #[test]
    fn an_application_with_no_readable_manifest_is_unknown_and_named() {
        let mut tree = MemoryTree::default();
        tree.directories
            .insert(service::app_root(BUILT_IN_VOLUME), vec!["half".to_string()]);
        tree.directories.insert(
            service::app_dir(BUILT_IN_VOLUME, &name("half")),
            vec!["0.1.0".to_string()],
        );

        let got = declared(&mut tree, BUILT_IN_VOLUME).expect_err("a hole");
        assert!(matches!(got, Unknown::Unread { .. }), "{got:?}");
        assert_eq!(got.application(), Some("half"));
    }

    /// A manifest that reads and does not parse is unknown too.
    #[test]
    fn a_manifest_that_does_not_parse_is_unknown() {
        let mut tree = MemoryTree::default();
        tree.directories
            .insert(service::app_root(BUILT_IN_VOLUME), vec!["broken".to_string()]);
        tree.files.insert(
            service::manifest_path(BUILT_IN_VOLUME, &name("broken")),
            "this is = = not a manifest".to_string(),
        );
        let got = declared(&mut tree, BUILT_IN_VOLUME).expect_err("unparsed");
        assert!(matches!(got, Unknown::Unparsed { .. }), "{got:?}");
    }

    /// An application root that cannot be listed is unknown, never empty.
    #[test]
    fn an_application_root_that_cannot_be_listed_is_unknown_rather_than_empty() {
        let mut tree = MemoryTree::default();
        tree.unlistable.insert(
            service::app_root(BUILT_IN_VOLUME),
            "Input/output error".to_string(),
        );
        let got = declared(&mut tree, BUILT_IN_VOLUME).expect_err("unlisted");
        assert!(matches!(got, Unknown::Unlisted { .. }), "{got:?}");
    }

    /// An application uninstalled with its data kept, and an image service's
    /// home, declare nothing and are not holes.
    #[test]
    fn a_data_only_directory_and_an_image_service_home_declare_nothing() {
        let mut tree = MemoryTree::default();
        tree.directories.insert(
            service::app_root(BUILT_IN_VOLUME),
            vec![
                "photo-index".to_string(),
                service::PROXYD_SERVICE_NAME.to_string(),
            ],
        );
        tree.directories.insert(
            service::app_dir(BUILT_IN_VOLUME, &name("photo-index")),
            vec![
                service::DATA_DIR.to_string(),
                format!("{}1-1", service::RECLAIM_MARK),
            ],
        );
        assert_eq!(declared(&mut tree, BUILT_IN_VOLUME), Ok(Vec::new()));
    }
}
