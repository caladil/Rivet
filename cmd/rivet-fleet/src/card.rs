//! **The card runbook, owned by the tool instead of by a document.**
//!
//! `PLAN.md` §0.0 is the runbook and it is complete — and reading it today means
//! passing three superseded blocks, two struck paragraphs and a work order for a
//! session that ended, before reaching a live instruction. That is what a
//! project journal is *for*; it is not what somebody holding an SD card needs.
//!
//! The maintainer put it plainly on 2026-08-27: an agent is already known to be
//! able to do it, and that would show humans can too (.rulings/ R-347). This
//! module is that claim being made testable. An agent can build a card because it reads
//! `tools/card/build.sh` and this file's own history; a human should not have
//! to, and neither should the next agent.
//!
//! # What this does NOT do, and the line is `ADR-034` §3's
//!
//! **It builds nothing and it signs nothing.** `rivet-fleet` holds no signing
//! key, and that is a ruling rather than an omission. So every step below is a
//! command **named** rather than run, and each is named with **this device's own
//! values already in it** — the describe, the document, the hostname, the size
//! of the card — because a runbook with placeholders in it is one an operator
//! fills in from memory, and §13.4 is what that costs.
//!
//! **It checks only what it owns.** The register and the composed document are
//! this tool's; `out/`, `tools/` and a Buildroot tree are a checkout's, and a
//! `rivet-fleet` installed on a machine that has no checkout would be lying if
//! it claimed to have looked. Where a step cannot be checked from here, it says
//! what to look at instead of asserting.
//!
//! # The three gates, which are the whole reason this exists
//!
//! Each of them produced a card that booted and was wrong, and none announces
//! itself:
//!
//! 1. **`--device-mib` must match the physical card** (`PLAN.md` §13.4, *"the
//!    most expensive of the three"*). The image's `/state` is sized from this
//!    number, and nothing on the running machine grows it — so a 7.42 GiB card
//!    written from a 2 GiB image has a 232 MiB `/state` and a payload that will
//!    not fit, discovered later and remotely.
//! 2. **The provisioning document must carry a key whose private half you
//!    hold.** `tools/card/build.sh` refuses the example key by name, which is
//!    the one gate that catches itself.
//! 3. **A card is written to a block device**, and naming the wrong one takes
//!    the machine writing it. This module names no device path, ever.

use crate::compose;
use crate::register::{Device, Register};

/// Where the composed provisioning document should be written before a build.
///
/// **Under the register's own root**, not in the repository: it is composed from
/// the register, it carries this device's public keys, and a file that lands in
/// a checkout is one that gets committed by accident. `AGENTS.md` §8.3 is about
/// tracked files and this is how one avoids becoming tracked.
const DOCUMENT_DIRECTORY: &str = "provisioning";

/// The step list, rendered for one device.
///
/// Returns the whole plan as text. **Every step says what it proves**, because
/// a runbook that only says what to type is one nobody can tell has gone wrong
/// in the middle.
pub fn plan(
    device: &Device,
    register: &Register,
    card_mib: Option<u32>,
    register_root: &std::path::Path,
) -> Result<String, String> {
    // Composed first, and the whole plan is refused if it cannot be: every step
    // below is downstream of this document existing, so a plan that printed
    // steps 3 to 7 above a document that cannot be made would be a plan whose
    // first half is fiction. **The strict composer**: a card is a document
    // leaving the register for good, so `SPEC-05` §5's completeness gates
    // apply here exactly as they do to an export or a live write.
    let document = compose::provisioning_complete_toml(device, &register.fleet)
        .map_err(|e| format!("this device's provisioning document cannot be composed, so there \
                              is nothing to write a card with: {e}"))?;

    let doc_path = register_root
        .join(DOCUMENT_DIRECTORY)
        .join(format!("{}.toml", device.id))
        .display()
        .to_string();

    let mut out = String::new();
    out.push_str(&header(device, register));
    out.push_str(&preflight(device, register, &document, card_mib));
    out.push_str(&steps(device, &doc_path, card_mib));
    out.push_str(AFTER);
    Ok(out)
}

fn header(device: &Device, register: &Register) -> String {
    let hostname = compose::hostname(device).map_or_else(
        || "(none — this card will not name itself)".to_owned(),
        |name| name.to_string(),
    );
    format!(
        "Writing a card for `{}`\n\
         =========================\n\n\
         device      {} ({})\n\
         hostname    {hostname}\n\
         keys        {} from this device, {} from the fleet\n\n\
         Every command below is run from a checkout of the Rivet repository.\n\
         This tool builds nothing and signs nothing (ADR-034 §3): it composes\n\
         the document and tells you the rest, with your values already in it.\n\n",
        device.name,
        device.name,
        device.id,
        device.control_plane_keys.len(),
        register.fleet.control_plane_keys.len(),
    )
}

/// What this tool can check for itself, answered before any step is offered.
///
/// **A check that passes says so as plainly as one that fails.** A preflight
/// that printed only problems would leave an operator unable to tell *checked
/// and fine* from *not checked*, which is the distinction the whole section
/// exists to make.
fn preflight(
    device: &Device,
    register: &Register,
    document: &str,
    card_mib: Option<u32>,
) -> String {
    let mut out = String::from("Before you start\n----------------\n\n");

    out.push_str(&format!(
        "  [ok]   the register has this device, and its document composes ({} bytes)\n",
        document.len()
    ));

    // **The check that used to be here is gone, and its absence is the
    // finding.** A branch for *no interface* was written and could never fire:
    // `crate::compose` refuses before this function is reached, with a sentence
    // better than the one drafted here — *"a card burned from a document
    // without one boots and cannot be reached — there is no keyboard on a
    // headless board"*. A check that cannot fire is a check that lies about
    // being one, so the refusal is left where it already was and `plan` simply
    // returns it.
    let total_keys = device.control_plane_keys.len() + register.fleet.control_plane_keys.len();
    out.push_str(&format!(
        "  [ok]   {total_keys} control-plane key(s), composed into the card's peers file
"
    ));

    // Gate 1. Never guessed, and the refusal carries the cost rather than the
    // rule — PLAN.md §13.4 is a number somebody paid for.
    match card_mib {
        Some(mib) => out.push_str(&format!(
            "  [ok]   the card is {mib} MiB, and the build will size /state from it\n"
        )),
        None => out.push_str(
            "  [STOP] the size of the physical card is not known to this tool and it will\n\
             \x20        NOT guess. PLAN.md §13.4 is the most expensive mistake in this\n\
             \x20        runbook: /state is sized at build time from --device-mib, nothing\n\
             \x20        on the running machine grows it, and a 7.42 GiB card written from\n\
             \x20        a 2 GiB image has a 232 MiB /state — found later, and remotely.\n\
             \x20        Re-run with --card-mib <N>, N being the card's real size in MiB.\n",
        ),
    }

    // Not checkable from here, and said so rather than skipped.
    out.push_str(
        "  [look] the Buildroot tree and the nu binary are outside this checkout, so\n\
         \x20        this tool cannot see them. tools/card/build.sh names both and\n\
         \x20        refuses by name if either is missing — let it do that check.\n\n",
    );
    let _ = device;
    out
}

/// The same path, as the OTHER environment sees it.
///
/// **Found by reading the plan this module printed**, 2026-08-27, when the
/// build step ran under WSL. It runs in a **container** now (`ADR-026`, and
/// `tools/container/build-card.sh` is the entry point), and the container mounts
/// the repository at `/rivet` and rewrites the `RIVET_*` paths itself — so the
/// only path this still has to translate is the document, which lives outside
/// the repository under the register's root and is passed in from here.
///
/// Handing the build a `C:\Users\...` path produces a refusal to open a document
/// that is plainly there, and the operator has no reason to suspect the runbook.
/// So the build step is given `/mnt/c/Users/...`, which is what both a WSL shell
/// and Git Bash's `MSYS_NO_PATHCONV` world agree on. This is a **rendering** of
/// one path for one reader and never a second source of truth: the file is
/// written once, at the path step 1 names.
///
/// Anything that is not a `X:\` path is returned unchanged, which is every path
/// on a Linux host and is why this costs nothing there.
fn as_wsl_sees_it(path: &str) -> String {
    let bytes = path.as_bytes();
    let drive_letter = bytes.first().copied().filter(u8::is_ascii_alphabetic);
    let Some(drive) = drive_letter else {
        return path.to_owned();
    };
    if bytes.get(1) != Some(&b':') || !matches!(bytes.get(2), Some(&b'\\') | Some(&b'/')) {
        return path.to_owned();
    }
    format!(
        "/mnt/{}{}",
        drive.to_ascii_lowercase() as char,
        path[2..].replace('\\', "/")
    )
}

fn steps(device: &Device, doc_path: &str, card_mib: Option<u32>) -> String {
    let mib = match card_mib {
        Some(mib) => mib.to_string(),
        None => "<CARD-MiB>".to_owned(),
    };
    let wsl_doc = as_wsl_sees_it(doc_path);
    // Said only when the two differ, because on a Linux host they do not and a
    // paragraph explaining a difference that is not there is noise.
    let path_note = if wsl_doc == doc_path {
        String::new()
    } else {
        format!(
            "   The path is written differently below because step 4 runs inside a\n\
             \x20  container and step 1 does not. It is the SAME file:\n\
             \x20    step 1 writes  {doc_path}\n\
             \x20    step 4 reads   {wsl_doc}\n\n"
        )
    };

    format!(
        "The steps\n---------\n\n\
         1. Compose this device's provisioning document, and keep it OUT of the\n\
         \x20  repository - it carries this machine's keys.\n\n\
         \x20    rivet-fleet export provisioning --device {name} > \"{doc_path}\"\n\n\
         {path_note}\
         \x20  Proves: the register can describe this machine completely enough to\n\
         \x20  provision it. A register that cannot is one nobody should trust with\n\
         \x20  the next machine.\n\n\
         2. Mint this device's control-plane identity. ADR-025 3 puts\n\
         \x20  generation HERE rather than on the board, because a machine that\n\
         \x20  minted its own key would have to tell the fleet about it over a\n\
         \x20  network that does not exist until it is provisioned.\n\n\
         \x20    rivet-fleet add identity --device {name} --out <a directory>\n\n\
         \x20  It writes device.seed, device.crt and a peers file composed from\n\
         \x20  this register, and keeps only the PUBLIC half (ADR-034 3.5).\n\n\
         \x20  THE SEED IS A PRIVATE KEY and this is the only moment it exists\n\
         \x20  off a card. Not in the register, not in git, not in a backup. The\n\
         \x20  board erases it from the card on first boot (ADR-019 3.1), so the\n\
         \x20  card you write is the only copy that will exist.\n\n\
         \x20  SKIP THIS and the card still boots - rivet-mcpd refuses to start\n\
         \x20  and nothing else is affected. That is a choice, not an oversight.\n\n\
         3. Mint the version. ON THE WINDOWS SIDE, from Git Bash - the script\n\
         \x20  refuses to run under WSL, and AGENTS.md 10.5 is why: the two gits\n\
         \x20  disagree about whether this tree is clean, and Windows is right.\n\n\
         \x20    sh tools/card/bump.sh\n\n\
         \x20  It refuses a dirty tree, increments the patch version, commits that\n\
         \x20  one change alone, and prints the describe. Keep the describe - step 4\n\
         \x20  needs it, and a card named from a guess is ADR-027 3.2.1.\n\n\
         4. Build the card. IN THE CONTAINER - sgdisk, mkfs and mtools are in the\n\
         \x20  image and not on this workstation. ADR-026 made Docker the Linux\n\
         \x20  host; the WSL route this step used to name is superseded.\n\n\
         \x20    export RIVET_DESCRIBE=<the describe from step 3>\n\
         \x20    export RIVET_DOCUMENT={wsl_doc}\n\
         \x20    export RIVET_IDENTITY=<the directory from step 2>\n\
         \x20    tools/container/build-card.sh --device-mib {mib}\n\n\
         \x20  build-card.sh is a WRAPPER and not a second build: it mounts what the\n\
         \x20  build reads, rewrites the RIVET_* paths into container paths, and runs\n\
         \x20  tools/card/build.sh unmodified. It asks for no privilege.\n\n\
         \x20  Proves: the image exists, is named for a real commit, and carries a\n\
         \x20  document with a key you hold. The build prints the image path and its\n\
         \x20  sha256 - keep both.\n\n\
         5. Write the card.\n\n\
         \x20  THIS TOOL WILL NOT NAME A BLOCK DEVICE, and neither should you from\n\
         \x20  memory. Naming the wrong one overwrites the machine you are typing on,\n\
         \x20  and the mistake is silent until it is not. Identify the card with the\n\
         \x20  card OUT and then IN, compare the two listings, and write only to the\n\
         \x20  difference.\n\n\
         \x20  These two only LIST. Run each with the card out, then again with it\n\
         \x20  in, and the difference is your card:\n\n\
         \x20    Windows   powershell -Command \"Get-Disk\"\n\
         \x20    WSL       lsblk -o NAME,SIZE,TYPE,MODEL\n\n\
         \x20  Then write the image from step 4 to it with whichever imager you use,\n\
         \x20  and let the write FINISH. A card unplugged before its cache is flushed\n\
         \x20  is a card that boots wrong, and it boots wrong in a way that looks like\n\
         \x20  a bad build.\n\n\
         6. Boot it, and ask the machine rather than the card. THE BOOT THAT JUST\n\
         \x20  ENDED FIRST, before anything else touches it - it is the one with the\n\
         \x20  answer in it, and the next boot rotates it away.\n\n\
         \x20    cargo rivet run --host {host}:{port} \\\n\
         \x20      --identity <the directory from step 2> \\\n\
         \x20      --pin <the pin in `rivet-fleet get device --device {name}`> \\\n\
         \x20      -- get log previous all\n\n\
         \x20  `all` is not decoration: get log answers 200 records by default and\n\
         \x20  has hidden the very thing being looked for.\n\n\
         \x20  THIS TOOL HAS NO `get log`, and that is why the line above is the\n\
         \x20  general client. `rivet-fleet get` has exactly two subcommands, device\n\
         \x20  and status. What this tool answers is what the supervisor made of it:\n\n\
         \x20    rivet-fleet get status --device {name}\n\n\
         \x20  That reaches {host}:{port} over ADR-035's control plane, with the\n\
         \x20  identity from step 2 and the pin this register holds. That is the\n\
         \x20  ONLY route in, so a machine whose [control-plane] is unreachable is\n\
         \x20  one to rewrite the card for rather than one to log into.\n\n",
        name = device.name,
        host = first_address(device).unwrap_or_else(|| "<this machine>".to_owned()),
        port = device.control_plane_port(),
    )
}

/// The address to suggest for step 5, if the register knows one.
///
/// **`None` for a DHCP member, and that is not a gap** — the register genuinely
/// does not know where such a machine will be, and printing a plausible address
/// would be worse than printing none.
fn first_address(device: &Device) -> Option<String> {
    device
        .interfaces
        .values()
        .find_map(|i| i.address.as_ref())
        .map(|a| a.split('/').next().unwrap_or(a).to_owned())
}

const AFTER: &str = "What to check, and in this order\n\
                     --------------------------------\n\n\
                     \x20 1. `-- get log previous all` — the boot that ended. Do this first,\n\
                     \x20    and with `all`: the 200-record default hides what you came for.\n\
                     \x20 2. `rivet get status` — is the supervisor running, and what started?\n\
                     \x20 3. `rivet get storage` — is /state the size you asked for in step 4?\n\
                     \x20    If it is not, --device-mib did not match the card and the machine\n\
                     \x20    cannot fix it: the card has to be rewritten.\n\
                     \x20 4. `rivet get install` — expect 0 installs on a fresh card.\n\
                     \x20 5. `rivet get status` again - is rivet-mcpd `started`? If step 2\n\
                     \x20    was skipped it will be `failed`, and the message names the\n\
                     \x20    file it looked for. Nothing else is affected by that.\n\n\
                     If the machine does not answer at all, the card is not the first thing\n\
                     to suspect: a key that did not reach the document, or a document that\n\
                     did not reach the card, both look exactly like a machine that is down.\n\
                     Step 1's file is what tells them apart.\n";

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::register::{Device, Interface, Register};
    use std::path::PathBuf;

    fn device() -> Device {
        let mut d = Device {
            id: "1f0532852afc2842".to_owned(),
            name: "board".to_owned(),
            // Nothing in this module's tests is about SPEC-05 5's
            // completeness gates - `plan` composes through
            // `provisioning_complete` because a card is a document leaving
            // the register for good, so the fixture answers what that gate
            // asks for the same way every other fixture in this crate does.
            timezone: Some("UTC".to_owned()),
            ..Device::default()
        };
        d.interfaces.insert(
            "eth0".to_owned(),
            Interface {
                method: "static".to_owned(),
                address: Some("10.0.0.2/24".to_owned()),
                ..Interface::default()
            },
        );
        d
    }

    fn rendered(card_mib: Option<u32>) -> String {
        let register = Register::default();
        plan(&device(), &register, card_mib, &PathBuf::from("/reg"))
            .expect("the fixture composes")
    }

    /// **The gate PLAN.md §13.4 paid for.** A plan that guessed a card size
    /// would produce exactly the failure that section records, so the absence of
    /// the number has to stop the operator rather than be filled in.
    #[test]
    fn a_plan_without_a_card_size_refuses_to_guess_one() {
        let text = rendered(None);
        assert!(text.contains("[STOP]"), "{text}");
        assert!(text.contains("13.4"), "{text}");
        assert!(text.contains("<CARD-MiB>"), "{text}");
        // And it must not have invented a number anywhere in the build command.
        assert!(!text.contains("--device-mib 2048"), "{text}");
    }

    #[test]
    fn a_plan_with_a_card_size_puts_it_in_the_build_command() {
        let text = rendered(Some(7600));
        assert!(text.contains("--device-mib 7600"), "{text}");
        assert!(!text.contains("<CARD-MiB>"), "{text}");
    }

    /// **No block device is ever named**, by this tool or in an example. An
    /// operator copying a command with a device path in it is the accident this
    /// refusal exists to prevent.
    #[test]
    fn no_step_names_a_block_device() {
        let text = rendered(Some(7600));
        for hazard in ["/dev/sd", "/dev/mmcblk", "\\\\.\\PhysicalDrive", "of="] {
            assert!(!text.contains(hazard), "{hazard} appears in the plan:\n{text}");
        }
    }

    /// The document goes somewhere that is not a checkout, because a file
    /// carrying a machine's keys that lands in the repository gets committed.
    #[test]
    fn the_document_is_written_outside_the_repository() {
        let text = rendered(Some(7600));
        // Composed with `Path::join`, so the separator is the platform's — this
        // tool runs on Windows as much as on Linux (ADR-034 §10.3).
        let want = PathBuf::from("/reg")
            .join("provisioning")
            .join("1f0532852afc2842.toml");
        assert!(text.contains(&want.display().to_string()), "{text}");
    }

    /// **A machine nothing can reach is refused before a plan exists**, and by
    /// the composer rather than by this module. Asserted here because the whole
    /// value of the plan is that it cannot describe a card that would boot
    /// unreachable — wherever the refusal comes from.
    #[test]
    fn a_device_nothing_can_reach_gets_no_plan_at_all() {
        let mut d = device();
        d.interfaces.clear();
        let register = Register::default();
        let refused = plan(&d, &register, Some(7600), &PathBuf::from("/reg"))
            .expect_err("a document with no interface must not compose");
        assert!(refused.contains("cannot be reached"), "{refused}");
    }

    /// The check order is the runbook's most load-bearing detail: the boot that
    /// just ended is the one with the answer, and the next boot rotates it away.
    #[test]
    fn the_first_thing_to_ask_is_the_previous_boot() {
        let text = rendered(Some(7600));
        let previous = text.find("get log previous").expect("it is named");
        let status = text.find("get status").expect("it is named");
        assert!(previous < status, "the plan asks for status before the previous boot");
    }

    /// **The three instructions the plan used to get wrong**, `SPEC-05`'s History, 2026-09-07,
    /// each verified by running the plan and then running what it said.
    ///
    /// They are asserted together because they share a cause: this is a
    /// hand-maintained template describing tools that moved, and the plan is
    /// *"the closest thing to §3's script that exists"*, which is why its
    /// wrongness matters more than its size.
    #[test]
    fn the_plan_names_commands_that_exist() {
        let text = rendered(Some(7600));

        // 1. `rivet-fleet get log` is not a verb. `get` has `device` and
        //    `status`, and answers `unrecognized subcommand 'log'`.
        assert!(
            !text.contains("rivet-fleet get log"),
            "the plan still names `rivet-fleet get log`; this tool's `get` takes \
             device and status:\n{text}"
        );
        assert!(text.contains("get log previous all"), "{text}");
        assert!(text.contains("rivet-fleet get status --device"), "{text}");

        // 2. The build moved into the container. ADR-026.
        assert!(
            !text.contains("tools/card/build.sh --device-mib"),
            "the plan still invokes the WSL-era build directly:\n{text}"
        );
        assert!(text.contains("tools/container/build-card.sh --device-mib 7600"), "{text}");

        // 3. The identity is step 2; step 3 is bump.sh.
        assert!(
            !text.contains("identity from step 3"),
            "the plan still cites the identity as step 3:\n{text}"
        );
        assert_eq!(
            text.matches("from step 2").count(),
            3,
            "every reference to the identity should name step 2:\n{text}"
        );
    }

    /// A register that knows an address puts it in the command. A **DHCP**
    /// member has none to know — `PLAN.md` §38 says so — and the plan must not
    /// invent a plausible one, because an address that looks right and is wrong
    /// is worse than a placeholder.
    #[test]
    fn a_known_address_is_used_and_a_dhcp_member_gets_no_invented_one() {
        // **`10.0.0.2:7777`, and not a login on that host.** The plan names the
        // control plane and the port the record resolves (`ADR-034` §6.1),
        // which is the only address anything can be reached at.
        let text = rendered(Some(7600));
        assert!(text.contains("10.0.0.2:7777"), "address and port missing:\n{text}");

        let mut d = device();
        d.interfaces.insert(
            "eth0".to_owned(),
            Interface {
                method: "dhcp".to_owned(),
                ..Interface::default()
            },
        );
        let register = Register::default();
        let text = plan(&d, &register, Some(7600), &PathBuf::from("/reg"))
            .expect("a dhcp member still composes");
        assert!(text.contains("<this machine>"), "{text}");
        assert!(!text.contains("192.168"), "an address was invented:\n{text}");
    }

    /// **The defect that only reading the output could find.** Step 1 runs where
    /// this tool runs and step 4 runs under WSL, so a `C:\...` path in the WSL
    /// command is a build that refuses to open a file which is plainly there.
    #[test]
    fn the_wsl_step_gets_a_path_wsl_can_open() {
        assert_eq!(
            as_wsl_sees_it(r"C:\Users\someone\register\d.toml"),
            "/mnt/c/Users/someone/register/d.toml"
        );
        // Lowercased, because /mnt is lowercase and `D:` is not.
        assert_eq!(as_wsl_sees_it(r"D:\x"), "/mnt/d/x");
        assert_eq!(as_wsl_sees_it("C:/already/forward"), "/mnt/c/already/forward");
    }

    /// A Linux host pays nothing for it, and a path that is not a drive letter
    /// is never mangled into one.
    #[test]
    fn a_path_that_is_not_a_windows_one_is_returned_unchanged() {
        for path in ["/home/j/register/d.toml", "relative/d.toml", "", "C", "C:"] {
            assert_eq!(as_wsl_sees_it(path), path, "{path:?} was rewritten");
        }
    }

    /// And the note explaining the two spellings appears only when they differ.
    #[test]
    fn the_two_spellings_are_explained_only_when_there_are_two() {
        let text = rendered(Some(7600));
        let translated = text.contains("/mnt/");
        assert_eq!(
            translated,
            text.contains("It is the SAME file"),
            "the note and the translation disagree:\n{text}"
        );
    }
}
