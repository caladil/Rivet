//! **A directory held open, and every name below it resolved against the
//! descriptor rather than against a path.**
//!
//! This daemon runs as root and writes into two stores: the machine's own
//! artifact store and a fleet server's repository store (`ADR-052` §7). The
//! second sits inside `rivet-fleetd`'s data directory, which that confined
//! application owns and can rearrange at any moment. A path that is checked and
//! then used is two lookups, and an entry swapped for a link between them sends
//! the second lookup wherever the link points — with root's authority.
//!
//! So a store is opened **once**, from a parent the application cannot write,
//! and everything after that is relative to the held descriptor: `mkdirat`,
//! `openat` with `O_NOFOLLOW`, `renameat`, `unlinkat`, `fchownat` with
//! `AT_SYMLINK_NOFOLLOW`. Renaming the store away and planting a link in its
//! place changes nothing about a [`Dir`] already open: it still names the real
//! directory, wherever that now is. A link found where a directory is expected
//! is refused ([`is_not_real`]), never followed.
//!
//! **Reading is not covered here**, and does not need to be for what this
//! closes: a listing or a verification that reads through a planted link reads
//! bytes, and writes and owner changes are what a link must never redirect.
//!
//! On a platform without these calls the same operations go by path. That is a
//! development host, where the store holds test fixtures and nothing confined
//! shares it.

#![allow(unsafe_code)]

use std::io;

/// What an entry below a [`Dir`] is, read without following it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Directory,
    File,
    Link,
    Other,
}

/// The refusal for a name that is a link, or not a directory, where a
/// directory was expected.
#[derive(Debug)]
struct NotReal;

impl std::fmt::Display for NotReal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a symbolic link or not a directory, so it was not followed")
    }
}

impl std::error::Error for NotReal {}

fn not_real() -> io::Error {
    io::Error::other(NotReal)
}

/// Whether `e` is the refusal to treat a link, or a non-directory, as a
/// directory.
#[must_use]
pub fn is_not_real(e: &io::Error) -> bool {
    e.get_ref().is_some_and(|inner| inner.is::<NotReal>())
}

/// One name, never a path: no separator, not `.` or `..`, not empty.
fn one_name(name: &str) -> io::Result<&str> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\0') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{name:?} is not a single entry name"),
        ));
    }
    Ok(name)
}

/// How deep [`Dir::remove`] descends. The stores this daemon writes are
/// `<root>/{objects,incoming}/<digest>/<file>`, so a directory removed from
/// either holds files and nothing deeper; the bound stops a removal walking
/// whatever else somebody built there.
const REMOVE_DEPTH: u32 = 3;

/// A directory held open.
#[derive(Debug)]
pub struct Dir {
    /// The path it was reached by, for messages only. Nothing is resolved
    /// against it after the directory is open.
    shown: String,
    #[cfg(target_os = "linux")]
    fd: std::os::fd::OwnedFd,
    #[cfg(not(target_os = "linux"))]
    path: std::path::PathBuf,
}

impl Dir {
    /// The path this directory was reached by.
    #[must_use]
    pub fn shown(&self) -> &str {
        &self.shown
    }

    fn shown_child(&self, name: &str) -> String {
        format!("{}/{name}", self.shown.trim_end_matches('/'))
    }

    /// **`path`, created if absent, held open.** Its parents are created by
    /// path and must be directories the confined applications cannot write;
    /// the last component is created with `mkdirat` and opened without
    /// following a link.
    ///
    /// # Errors
    ///
    /// The parent cannot be created or opened, or `path` is a link or not a
    /// directory ([`is_not_real`]).
    pub fn anchor(path: &str) -> io::Result<Self> {
        let (parent, last) = split(path)?;
        std::fs::create_dir_all(parent)?;
        let parent = Self::open_path(parent)?;
        parent.make(last)
    }

    /// **`path`, which must already exist, held open**, without following a
    /// link at its last component.
    ///
    /// # Errors
    ///
    /// `NotFound` when it is absent; [`is_not_real`] when it is a link or not
    /// a directory.
    pub fn existing(path: &str) -> io::Result<Self> {
        let (parent, last) = split(path)?;
        Self::open_path(parent)?.open(last)
    }

    /// `name` below this directory, created if absent, held open.
    ///
    /// # Errors
    ///
    /// As [`Dir::open`], and whatever `mkdirat` refuses other than an entry
    /// that already exists.
    pub fn make(&self, name: &str) -> io::Result<Self> {
        let name = one_name(name)?;
        match self.mkdir(name) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
        self.open(name)
    }

    /// `name` below this directory, which must be a real directory, held open.
    ///
    /// # Errors
    ///
    /// `NotFound` when it is absent; [`is_not_real`] when it is a link or not
    /// a directory.
    pub fn open(&self, name: &str) -> io::Result<Self> {
        let name = one_name(name)?;
        self.open_child(name)
    }

    /// **Remove `name`**, a file, a link (the link itself, never its target)
    /// or a directory and what it holds, without following a link anywhere.
    ///
    /// # Errors
    ///
    /// `NotFound` when it is absent, or the first entry that could not be
    /// removed.
    pub fn remove(&self, name: &str) -> io::Result<()> {
        self.remove_within(one_name(name)?, REMOVE_DEPTH)
    }

    fn remove_within(&self, name: &str, depth: u32) -> io::Result<()> {
        if self.kind(name)? != Kind::Directory {
            return self.unlink(name, false);
        }
        if depth == 0 {
            return Err(io::Error::other(format!(
                "{} is nested deeper than this store lays out, so it was not removed",
                self.shown_child(name)
            )));
        }
        let child = self.open_child(name)?;
        for entry in child.names()? {
            child.remove_within(&entry, depth - 1)?;
        }
        drop(child);
        self.unlink(name, true)
    }

    /// What `name` is, read without following it.
    ///
    /// # Errors
    ///
    /// `NotFound` when it is absent.
    pub fn kind(&self, name: &str) -> io::Result<Kind> {
        self.kind_of(one_name(name)?)
    }
}

/// `path` as its parent and its last component.
fn split(path: &str) -> io::Result<(&str, &str)> {
    let trimmed = path.trim_end_matches('/');
    let (parent, last) = trimmed.rsplit_once('/').ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, format!("{path} has no parent directory"))
    })?;
    let parent = if parent.is_empty() { "/" } else { parent };
    Ok((parent, one_name(last)?))
}

#[cfg(target_os = "linux")]
mod sys {
    use std::ffi::{CStr, CString};
    use std::io;
    use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};

    use super::{Dir, Kind, not_real};

    const DIRECTORY: libc::c_int = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;

    fn c(name: &str) -> io::Result<CString> {
        CString::new(name).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "a name holding NUL"))
    }

    fn checked(rc: libc::c_int) -> io::Result<()> {
        if rc < 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
    }

    fn owned(fd: libc::c_int) -> io::Result<OwnedFd> {
        if fd < 0 {
            Err(io::Error::last_os_error())
        } else {
            // SAFETY: `fd` was just returned by the kernel and is owned by
            // nothing else.
            Ok(unsafe { OwnedFd::from_raw_fd(fd) })
        }
    }

    /// `ELOOP` (a link, refused by `O_NOFOLLOW`) and `ENOTDIR` (not a
    /// directory, refused by `O_DIRECTORY`) are both the same answer here.
    fn directory_refusal(e: io::Error) -> io::Error {
        match e.raw_os_error() {
            Some(libc::ELOOP | libc::ENOTDIR) => not_real(),
            _ => e,
        }
    }

    impl Dir {
        pub(super) fn open_path(path: &str) -> io::Result<Self> {
            let at = c(path)?;
            // SAFETY: `at` is a NUL-terminated string that outlives the call.
            let fd = owned(unsafe { libc::open(at.as_ptr(), DIRECTORY) }).map_err(directory_refusal)?;
            Ok(Self { shown: path.to_owned(), fd })
        }

        pub(super) fn open_child(&self, name: &str) -> io::Result<Self> {
            let at = c(name)?;
            // SAFETY: a held descriptor and a NUL-terminated name.
            let fd = owned(unsafe { libc::openat(self.fd.as_raw_fd(), at.as_ptr(), DIRECTORY) })
                .map_err(directory_refusal)?;
            Ok(Self { shown: self.shown_child(name), fd })
        }

        pub(super) fn mkdir(&self, name: &str) -> io::Result<()> {
            let at = c(name)?;
            // SAFETY: a held descriptor and a NUL-terminated name.
            checked(unsafe { libc::mkdirat(self.fd.as_raw_fd(), at.as_ptr(), 0o755) })
        }

        /// **A new file**, `O_EXCL` and `O_NOFOLLOW`: an entry already there,
        /// a link included, is refused rather than truncated or followed.
        ///
        /// # Errors
        ///
        /// `AlreadyExists` when anything is already named `name`.
        pub fn create_new(&self, name: &str) -> io::Result<std::fs::File> {
            let at = c(super::one_name(name)?)?;
            let flags = libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC;
            let mode: libc::c_uint = 0o644;
            // SAFETY: a held descriptor, a NUL-terminated name, and the mode
            // `O_CREAT` reads.
            let fd = owned(unsafe { libc::openat(self.fd.as_raw_fd(), at.as_ptr(), flags, mode) })?;
            Ok(std::fs::File::from(fd))
        }

        pub(super) fn unlink(&self, name: &str, directory: bool) -> io::Result<()> {
            let at = c(name)?;
            let flags = if directory { libc::AT_REMOVEDIR } else { 0 };
            // SAFETY: a held descriptor and a NUL-terminated name.
            checked(unsafe { libc::unlinkat(self.fd.as_raw_fd(), at.as_ptr(), flags) })
        }

        /// **Move `name` here to `to_name` in `to`**, one `renameat(2)`
        /// between two held directories.
        ///
        /// # Errors
        ///
        /// Whatever `renameat` refuses.
        pub fn rename_to(&self, name: &str, to: &Self, to_name: &str) -> io::Result<()> {
            let from = c(super::one_name(name)?)?;
            let into = c(super::one_name(to_name)?)?;
            // SAFETY: two held descriptors and two NUL-terminated names.
            checked(unsafe {
                libc::renameat(self.fd.as_raw_fd(), from.as_ptr(), to.fd.as_raw_fd(), into.as_ptr())
            })
        }

        /// `fsync(2)` of the directory itself, which makes an entry renamed
        /// into it durable.
        ///
        /// # Errors
        ///
        /// Whatever `fsync` refuses.
        pub fn sync(&self) -> io::Result<()> {
            // SAFETY: a held descriptor.
            checked(unsafe { libc::fsync(self.fd.as_raw_fd()) })
        }

        /// The names this directory holds, `.` and `..` left out.
        ///
        /// # Errors
        ///
        /// The directory cannot be read.
        pub fn names(&self) -> io::Result<Vec<String>> {
            let here = c".";
            // A second descriptor for the stream, because `closedir` closes
            // the one it is given and this one stays held.
            // SAFETY: a held descriptor and a NUL-terminated name.
            let raw = unsafe {
                libc::openat(self.fd.as_raw_fd(), here.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC)
            };
            if raw < 0 {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: `raw` is an open directory descriptor, handed to the
            // stream, which owns it from here.
            let stream = unsafe { libc::fdopendir(raw) };
            if stream.is_null() {
                let e = io::Error::last_os_error();
                // SAFETY: the stream did not take `raw`, so it is still ours.
                unsafe { libc::close(raw) };
                return Err(e);
            }
            let mut out = Vec::new();
            let result = loop {
                // SAFETY: `__errno_location` is this thread's errno, and
                // `readdir` reports an error only through it.
                unsafe { *libc::__errno_location() = 0 };
                // SAFETY: `stream` is open until the `closedir` below.
                let entry = unsafe { libc::readdir(stream) };
                if entry.is_null() {
                    // SAFETY: as above.
                    let errno = unsafe { *libc::__errno_location() };
                    break if errno == 0 { Ok(()) } else { Err(io::Error::from_raw_os_error(errno)) };
                }
                // SAFETY: `readdir` returned a valid entry whose `d_name` is
                // NUL-terminated and lives until the next call.
                let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
                let name = name.to_string_lossy();
                if name != "." && name != ".." {
                    out.push(name.into_owned());
                }
            };
            // SAFETY: `stream` is open and closed exactly once.
            unsafe { libc::closedir(stream) };
            result.map(|()| out)
        }

        pub(super) fn kind_of(&self, name: &str) -> io::Result<Kind> {
            let at = c(name)?;
            // SAFETY: `stat` is plain data the call fills in.
            let mut st: libc::stat = unsafe { std::mem::zeroed() };
            // SAFETY: a held descriptor, a NUL-terminated name and a `stat`
            // that outlives the call.
            checked(unsafe {
                libc::fstatat(self.fd.as_raw_fd(), at.as_ptr(), &raw mut st, libc::AT_SYMLINK_NOFOLLOW)
            })?;
            Ok(match st.st_mode & libc::S_IFMT {
                libc::S_IFDIR => Kind::Directory,
                libc::S_IFREG => Kind::File,
                libc::S_IFLNK => Kind::Link,
                _ => Kind::Other,
            })
        }

        /// The owner of this directory itself, set on the held descriptor.
        ///
        /// # Errors
        ///
        /// Whatever `fchown` refuses.
        pub fn set_owner(&self, uid: u32, gid: u32) -> io::Result<()> {
            // SAFETY: a held descriptor.
            checked(unsafe { libc::fchown(self.fd.as_raw_fd(), uid, gid) })
        }

        /// The mode of this directory itself, set on the held descriptor.
        ///
        /// # Errors
        ///
        /// Whatever `fchmod` refuses.
        pub fn set_mode(&self, mode: u32) -> io::Result<()> {
            // SAFETY: a held descriptor.
            checked(unsafe { libc::fchmod(self.fd.as_raw_fd(), mode) })
        }

        /// The owner of `name` below this directory, `AT_SYMLINK_NOFOLLOW`:
        /// a link has its own owner changed, never its target's.
        ///
        /// # Errors
        ///
        /// Whatever `fchownat` refuses.
        pub fn set_entry_owner(&self, name: &str, uid: u32, gid: u32) -> io::Result<()> {
            let at = c(super::one_name(name)?)?;
            // SAFETY: a held descriptor and a NUL-terminated name.
            checked(unsafe {
                libc::fchownat(self.fd.as_raw_fd(), at.as_ptr(), uid, gid, libc::AT_SYMLINK_NOFOLLOW)
            })
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod sys {
    use std::io;

    use super::{Dir, Kind, not_real};

    impl Dir {
        fn child(&self, name: &str) -> std::path::PathBuf {
            self.path.join(name)
        }

        pub(super) fn open_path(path: &str) -> io::Result<Self> {
            let meta = std::fs::symlink_metadata(path)?;
            if !meta.is_dir() {
                return Err(not_real());
            }
            Ok(Self { shown: path.to_owned(), path: path.into() })
        }

        pub(super) fn open_child(&self, name: &str) -> io::Result<Self> {
            let path = self.child(name);
            let meta = std::fs::symlink_metadata(&path)?;
            if !meta.is_dir() {
                return Err(not_real());
            }
            Ok(Self { shown: self.shown_child(name), path })
        }

        pub(super) fn mkdir(&self, name: &str) -> io::Result<()> {
            std::fs::create_dir(self.child(name))
        }

        /// A new file; an entry already there is refused.
        ///
        /// # Errors
        ///
        /// `AlreadyExists` when anything is already named `name`.
        pub fn create_new(&self, name: &str) -> io::Result<std::fs::File> {
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(self.child(super::one_name(name)?))
        }

        pub(super) fn unlink(&self, name: &str, directory: bool) -> io::Result<()> {
            if directory {
                std::fs::remove_dir(self.child(name))
            } else {
                std::fs::remove_file(self.child(name))
            }
        }

        /// Move `name` here to `to_name` in `to`.
        ///
        /// # Errors
        ///
        /// Whatever the rename refuses.
        pub fn rename_to(&self, name: &str, to: &Self, to_name: &str) -> io::Result<()> {
            std::fs::rename(self.child(super::one_name(name)?), to.child(super::one_name(to_name)?))
        }

        /// Nothing: a directory cannot be synced here.
        ///
        /// # Errors
        ///
        /// None.
        pub fn sync(&self) -> io::Result<()> {
            Ok(())
        }

        /// The names this directory holds.
        ///
        /// # Errors
        ///
        /// The directory cannot be read.
        pub fn names(&self) -> io::Result<Vec<String>> {
            std::fs::read_dir(&self.path)?
                .map(|entry| entry.map(|e| e.file_name().to_string_lossy().into_owned()))
                .collect()
        }

        pub(super) fn kind_of(&self, name: &str) -> io::Result<Kind> {
            let kind = std::fs::symlink_metadata(self.child(name))?.file_type();
            Ok(if kind.is_symlink() {
                Kind::Link
            } else if kind.is_dir() {
                Kind::Directory
            } else if kind.is_file() {
                Kind::File
            } else {
                Kind::Other
            })
        }

        /// Nothing: this platform has no owners to set.
        ///
        /// # Errors
        ///
        /// None.
        pub fn set_owner(&self, _uid: u32, _gid: u32) -> io::Result<()> {
            Ok(())
        }

        /// Nothing: this platform has no modes to set.
        ///
        /// # Errors
        ///
        /// None.
        pub fn set_mode(&self, _mode: u32) -> io::Result<()> {
            Ok(())
        }

        /// Nothing: this platform has no owners to set.
        ///
        /// # Errors
        ///
        /// None.
        pub fn set_entry_owner(&self, _name: &str, _uid: u32, _gid: u32) -> io::Result<()> {
            Ok(())
        }
    }
}

#[cfg(test)]
#[path = "beneath_tests.rs"]
mod tests;
