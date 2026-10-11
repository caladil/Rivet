//! **The repository half of fleet-as-code** — `ADR-044`, `ADR-035`.
//!
//! The fleet's desired-state documents live in a bare repository on this fleet
//! server, named by the `fleet-repository` setting and declared by this
//! server's own document under `[repos.<name>]`. **This program runs no git.**
//! A confined application is refused `/system/bin` and `/state/git`
//! (`ADR-042`), so every question about the repository is one of
//! `rivet_core::fleet_as_code`'s operations, asked of this machine's own
//! `rivet-mcpd` with the operator identity this program already presents to
//! every machine it manages.
//!
//! # Which control plane is this machine's
//!
//! The register record whose role is `rivet_core::fleet_as_code::SERVER_ROLE`,
//! which provisioning writes as the one record a server's card carries. Its pin
//! is this machine's key. Its `listen` says where `rivet-mcpd` binds: an
//! unspecified or loopback address is reached on loopback, and a specific
//! address is reached there. Exactly one such record is required; none, or
//! more than one, is refused by name rather than guessed between.
//!
//! # Nothing here is a clone
//!
//! There is no copy of the repository on this side, so there is nothing to
//! fetch and nothing to leave half-written. A repository that cannot be reached
//! is a pass that does not happen, and the next cycle asks again.

use rivet_core::fleet_as_code::{
    self as wire, Branch, CommitId, CommitRequest, Committed, DocumentPath, FileRequest, Head,
    HeadRequest, Kind, Machines, MachinesRequest, Refusal, Request,
};
use rivet_core::repo::RepoName;
use rivetos_register::{Device, Register};

/// Where a machine's document lives in the repository, for a label.
#[must_use]
pub fn path_of(label: &str) -> String {
    wire::path_of(label)
}

/// **Something that answers fleet-as-code's operations.** The control plane on
/// a machine; an in-memory repository in a test.
pub trait ControlPlane {
    /// Send `body` to `route` and hand back the status and the body that came
    /// back.
    ///
    /// # Errors
    ///
    /// The transport's words: nothing readable came back.
    fn post(&self, route: &str, body: &[u8]) -> Result<(u16, Vec<u8>), String>;
}

/// This machine's own control plane, over `ADR-035`'s TLS with this program's
/// operator identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tls {
    /// `host:port`.
    pub address: String,
    /// This machine's own key.
    pub pin: String,
}

impl ControlPlane for Tls {
    fn post(&self, route: &str, body: &[u8]) -> Result<(u16, Vec<u8>), String> {
        crate::node::exchange(&self.address, &self.pin, route, body)
    }
}

/// **This fleet server's own control plane**, out of its register.
///
/// # Errors
///
/// No record with the server role, more than one, a record with no pin, or a
/// `listen` that is not an address — each a sentence naming the fix.
pub fn this_server(register: &Register) -> Result<Tls, String> {
    let mine: Vec<&Device> = register
        .devices
        .values()
        .filter(|device| device.role.as_deref() == Some(wire::SERVER_ROLE))
        .collect();
    let device = match mine.as_slice() {
        [one] => *one,
        [] => {
            return Err(format!(
                "this server's register holds no record with role = \"{}\", so it cannot tell which \
                 control plane is its own, and fleet-as-code's repository is reached through that \
                 control plane. Provisioning writes that record onto a fleet server's card \
                 (ADR-034); add role = \"{}\" to this machine's own record.",
                wire::SERVER_ROLE,
                wire::SERVER_ROLE
            ));
        }
        several => {
            let names: Vec<&str> = several.iter().map(|d| d.name.as_str()).collect();
            return Err(format!(
                "this server's register holds {} records with role = \"{}\" ({}), and this program \
                 will not choose which of them is itself. Exactly one record is this machine's.",
                several.len(),
                wire::SERVER_ROLE,
                names.join(", ")
            ));
        }
    };
    let pin = device.pin.clone().ok_or_else(|| {
        format!(
            "{}'s record carries no pin, so there is nothing to trust at this machine's own \
             control plane. ADR-025 section 2 makes the pinned key the whole trust model.",
            device.name
        )
    })?;
    let address = own_address(device.listen.as_deref(), device.control_plane_port())?;
    Ok(Tls { address, pin })
}

/// Where this machine's `rivet-mcpd` answers a caller on this machine.
///
/// # Errors
///
/// A `listen` that is not an address and a port.
fn own_address(listen: Option<&str>, port: u16) -> Result<String, String> {
    let Some(listen) = listen else {
        return Ok(format!("127.0.0.1:{port}"));
    };
    let address: std::net::SocketAddr = listen.parse().map_err(|_| {
        format!("this server's record says listen = {listen:?}, which is not an address and a port")
    })?;
    let ip = address.ip();
    Ok(match ip {
        std::net::IpAddr::V4(v4) if v4.is_unspecified() || v4.is_loopback() => format!("127.0.0.1:{}", address.port()),
        std::net::IpAddr::V6(v6) if v6.is_unspecified() || v6.is_loopback() => format!("[::1]:{}", address.port()),
        _ => address.to_string(),
    })
}

/// Fleet-as-code's repository, reached through a [`ControlPlane`].
pub struct Repository<'a> {
    /// The repository's name, as the setting spells it.
    pub name: &'a str,
    pub control: &'a dyn ControlPlane,
}

impl Repository<'_> {
    fn repository(&self) -> Result<RepoName, String> {
        wire::repository(self.name).map_err(|refusal| {
            format!(
                "`{}` is {:?}. {} (ADR-044 section 5.4)",
                crate::code::setting::REPOSITORY,
                self.name,
                refusal.why
            )
        })
    }

    /// One operation, and its answer's body or the refusal's words.
    fn ask(&self, request: &Request) -> Result<String, Refusal> {
        let route = request.operation().route();
        let (status, body) = self
            .control
            .post(route, &request.render())
            .map_err(|why| Refusal::new(Kind::Failed, format!("this server's control plane did not answer {route}: {why}")))?;
        let text = String::from_utf8(body)
            .map_err(|_| Refusal::new(Kind::Failed, format!("{route} answered with bytes that are not text")))?;
        if status == 200 {
            return Ok(text);
        }
        Err(Refusal::parse(&text).unwrap_or_else(|| {
            Refusal::new(
                Kind::Failed,
                format!(
                    "{route} answered {status} with a body that is not a refusal this build reads. The \
                     machine's rivet-mcpd may predate fleet-as-code's operations. It said: {}",
                    text.trim()
                ),
            )
        }))
    }

    /// The commit the branch points at, and its whole message.
    ///
    /// # Errors
    ///
    /// The setting's value, the branch, or the control plane's refusal.
    pub fn head(&self, branch: &str) -> Result<Head, String> {
        let request = Request::Head(HeadRequest {
            repository: self.repository()?,
            branch: Branch::new(branch).map_err(|r| r.why)?,
        });
        let text = self.ask(&request).map_err(|refusal| refusal.to_string())?;
        Head::parse(&text)
    }

    /// Every machine the repository names at `commit`, sorted as git lists a
    /// tree, and the names in `fleet/machines/` that are not labels.
    ///
    /// # Errors
    ///
    /// The control plane's refusal.
    pub fn machines(&self, commit: &str) -> Result<Machines, String> {
        let request = Request::Machines(MachinesRequest {
            repository: self.repository()?,
            commit: CommitId::new(commit).map_err(|r| r.why)?,
        });
        let text = self.ask(&request).map_err(|refusal| refusal.to_string())?;
        Machines::parse(&text)
    }

    /// **A document, as some commit had it** — or `None` if that commit did not
    /// have one.
    ///
    /// # Errors
    ///
    /// The control plane's refusal, a commit the repository does not hold
    /// included.
    pub fn file(&self, commit: &str, path: &str) -> Result<Option<String>, String> {
        let request = Request::File(FileRequest {
            repository: self.repository()?,
            commit: CommitId::new(commit).map_err(|r| r.why)?,
            path: DocumentPath::new(path).map_err(|r| r.why)?,
        });
        let text = self.ask(&request).map_err(|refusal| refusal.to_string())?;
        match wire::File::parse(&text)? {
            wire::File::Present(text) => Ok(Some(text)),
            wire::File::Absent => Ok(None),
        }
    }

    /// **Commit `new_content` as `path`'s text on `branch`** — the write behind
    /// `POST /api/commit/<device>`.
    ///
    /// **Two compare-and-swaps, and each catches a different arrival.**
    /// `expected` is what the caller believes `path` holds: `Some(text)` for an
    /// edit, `None` for a document that must not exist yet. It is compared with
    /// the file at the branch's tip, which refuses a change composed against a
    /// document somebody has since changed. The commit is then asked for on
    /// that tip, and the control plane refuses it as a stale head if the branch
    /// moved in between, which refuses the race the first comparison cannot
    /// see.
    ///
    /// # Errors
    ///
    /// Either refusal, each saying `REFUSED` and that nothing was committed, or
    /// the control plane's words.
    pub fn commit_file(
        &self,
        branch: &str,
        path: &str,
        expected: Option<&str>,
        new_content: &str,
        message: &str,
    ) -> Result<String, String> {
        let head = self.head(branch)?;
        let current = self.file(head.commit.as_str(), path)?;
        match (expected, current.as_deref()) {
            (Some(want), Some(have)) if want == have => {}
            (None, None) => {}
            (Some(_), Some(have)) => {
                return Err(format!(
                    "{path} has changed since it was read — REFUSED, nothing was committed. \
                     what is there now is {} bytes and does not match what this write expected.",
                    have.len()
                ));
            }
            (Some(_), None) => {
                return Err(format!(
                    "{path} no longer exists at the tip of {branch} — REFUSED, nothing was committed."
                ));
            }
            (None, Some(_)) => {
                return Err(format!(
                    "{path} already exists at the tip of {branch}, and this write expected to create it \
                     new — REFUSED, nothing was committed."
                ));
            }
        }

        let request = Request::Commit(CommitRequest {
            repository: self.repository()?,
            branch: Branch::new(branch).map_err(|r| r.why)?,
            expected_head: head.commit,
            path: DocumentPath::new(path).map_err(|r| r.why)?,
            message: message.to_owned(),
            document: new_content.to_owned(),
        });
        let text = self.ask(&request).map_err(|refusal| refusal.to_string())?;
        Ok(Committed::parse(&text)?.commit.as_str().to_owned())
    }
}

#[cfg(test)]
#[path = "repo_tests.rs"]
mod tests;
