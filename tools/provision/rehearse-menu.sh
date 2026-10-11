#!/bin/sh
# Drive `rivet-provision`'s MENU under a pty and hold its certificate beside the
# one the COMMAND LINE makes from the same answers.
#
# Usage:
#   tools/provision/rehearse-menu.sh            # in the container; needs a pty
#
# ## Why this exists
#
# `rehearse.sh` exhausts one adapter, the command line, and that is not enough.
# `ADR-006` §3.1's co-equality is not satisfied by both interfaces merely HAVING
# an operation: these two once composed *different* certificates from the same
# request, each internally consistent, so neither adapter tested alone could
# have shown it. **Only a test spanning both catches that shape of defect**, and
# until this one ran the menu's wiring was proven by unit test and by reading,
# never by running.
#
# `the_menu_and_the_command_line_compose_the_same_names` already holds the two
# NAME LISTS side by side. This holds the two CERTIFICATES side by side, which
# is a different claim: a list can agree and the thing minted from it still
# differ, because between the list and the certificate lie `mint_for_host`,
# `mint_server`, the suffix reader and two different callers of `issue_from`.
#
# ## A pty, and not a pipe
#
# The menu reads with `read_line`, so a pipe would drive it. A pty is used
# anyway because it is what a person has: `main.rs` asks
# `IsTerminal::is_terminal(&stdout)` and renders the whole screen in colour when
# the answer is yes, and that is the code path nothing had ever executed. Under
# a pipe the colour branch stays dark and the run proves less than it looks.
#
# `script -q -c` is the pty: it allocates one, runs the command inside it, and
# copies its own stdin into the master. That makes the keystrokes below arrive
# the way typed characters arrive.
#
# ## HOME is moved, and that is the whole safety story
#
# `authority_directory()` and `save::directory()` both read `HOME` (then
# `USERPROFILE`) and join `.rivet/`. Nothing else decides where a mint lands.
# So pointing `HOME` at a throwaway directory puts the ENTIRE ceremony there —
# the root, its key, the browser leaf, the server leaf and any saved answers —
# and `~/.rivet/ca`, which holds the real fleet's trust, is never opened.
#
# This is also why the menu run needs a fresh `HOME` rather than a fresh
# directory argument: `ensure_authority()` returns immediately when
# `$HOME/.rivet/ca/root.crt` exists, so a run against a populated home would
# skip the mint and silently test nothing.
#
# ## …and `--no-install`, which is the other half of the safety story
#
# HOME moves the FILES. It does not move the Windows certificate store, and the
# `ca` verb below enrolls the operator's identity into it with `certutil` —
# resolved from the system directory, so removing it from `PATH` does not stop
# it. Every run of this harness used to leave a `CN=Rehearsal Operator`
# certificate in the operator's real Personal store (`SPEC-05` §9, and
# `Fleet-Bring-Up-Findings.md` §3.4 is three of them). `--no-install` writes
# `browser.pfx` and opens no store, which is what makes rehearsal free.
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)

FLEET=${1:-rehearsal-fleet}
SUFFIXES=${2:-local,rehearsal-fleet.home.arpa}
HOST=${3:-arbor}
OPERATOR=${4:-Rehearsal Operator}
# **The zone, and the typo that must be refused before it.** `SPEC-05` §5.0: a
# well-formed name for a zone no image carries parses, projects, and leaves the
# machine on UTC with nothing in any log — so the refusal is the feature, and it
# is driven here rather than only unit-tested.
ZONE=${5:-America/Chicago}
TYPO=${6:-America/Chigago}

WORK=${RIVET_REHEARSE_DIR:-$(mktemp -d 2>/dev/null || echo "/tmp/rivet-menu.$$")}
MENU_HOME="$WORK/menu-home"
CLI_HOME="$WORK/cli-home"
MENU_CA="$MENU_HOME/.rivet/ca"
CLI_CA="$CLI_HOME/.rivet/ca"

FAILED=""
fail() {
    echo "  FAIL: $1" >&2
    FAILED="yes"
}
ok() { echo "  ok   $1"; }

cleanup() {
    # Kept on failure, deliberately: the artifact is the evidence.
    if [ -z "$FAILED" ] && [ -z "${RIVET_REHEARSE_KEEP:-}" ]; then
        rm -rf "$WORK"
    else
        echo ""
        echo "  the artifacts are in $WORK"
    fi
}
trap cleanup EXIT

echo "rehearse-menu.sh: fleet=$FLEET suffixes=$SUFFIXES host=$HOST"
echo "rehearse-menu.sh: working in $WORK"
echo ""

# --- what this run cannot do without ------------------------------------------
#
# Both are refusals rather than skips. A harness whose only assertion quietly
# turns itself off passes while proving nothing, which is the same failure as
# the defect it exists to catch.
command -v script >/dev/null 2>&1 \
    || { echo "  FAIL: no script(1), so there is no pty to drive the menu in" >&2; exit 1; }
command -v openssl >/dev/null 2>&1 \
    || { echo "  FAIL: no openssl, so the two certificates cannot be compared" >&2; exit 1; }

echo "rehearse-menu.sh: building rivet-provision from this tree..."
( cd "$REPO" && cargo build --release -p rivetos-provision >/dev/null 2>&1 ) \
    || { echo "  FAIL: rivet-provision does not build" >&2; exit 1; }
# **`CARGO_TARGET_DIR` first, and it is not a nicety here.** The container sets
# it to `/target` so the build cache outlives the mount, so `$REPO/target` does
# not exist inside it — and this harness only runs there, because `script(1)` is
# the pty and Windows has none.
TARGET=${CARGO_TARGET_DIR:-$REPO/target}
PROVISION="$TARGET/release/rivet-provision"
[ -x "$PROVISION" ] || PROVISION="$TARGET/release/rivet-provision.exe"
[ -x "$PROVISION" ] || { echo "  FAIL: no rivet-provision binary under $TARGET/release" >&2; exit 1; }

mkdir -p "$MENU_HOME" "$CLI_HOME"

# **A fixture fleet repository, one published version per product a fleet
# server carries** — `.rulings/` R-178 P3: the menu now asks which published
# version of `rivet-fleetd`/`rivet-proxyd` to carry, against
# `RIVET_FLEET_REPOSITORY`, and refuses by name when nothing is published for
# a product the role needs. Exactly one candidate each means
# `choose_carried_version` picks it with no prompt (`cmd/cargo-rivet/src/
# write_image.rs`'s own "one candidate is not a question" convention), so the
# keystroke sequence below needs no addition for it. `versions.rs` reads only
# `product`/`version`/`kind` out of `manifest.toml` — no `objects/` and no
# real bytes behind the fictional digest are needed.
FLEET_REPO="$WORK/fleet-repo"
mkdir -p "$FLEET_REPO"
# `versions.rs` reads only `kind`/`product`/`version` off each `[[artifact]]`
# — `digest` is never consulted (this fixture publishes no bytes under
# `objects/` and nothing here resolves one), so a fixed, obviously-fictional
# value is enough.
DIGEST="ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
for product in rivet-fleetd rivet-proxyd; do
    {
        printf '\n[[artifact]]\n'
        printf 'digest = "%s"\n' "$DIGEST"
        printf 'product = "%s"\n' "$product"
        printf 'version = "0.1.0"\n'
        printf 'arch = "aarch64"\n'
        printf 'kind = "application"\n'
        printf 'size = 1\n'
        printf 'added = "2026-09-13"\n'
    } >> "$FLEET_REPO/manifest.toml"
done

# --- 1. drive the menu --------------------------------------------------------
#
# Every line below is a keystroke a person makes, in order, and the comment
# beside it is the prompt it answers. The order is `main.rs`: the authority is
# offered first because `ensure_authority()` runs before `questions()`, and
# `offer_to_resume()` asks nothing because a fresh HOME has no saved answers.
#
# Field 6 is `Fleet` and choice 1 is `server`. That is the answer the whole
# script turns on: `w` mints a host certificate ONLY when Fleet is `server`
# (`main.rs`, the `"w"` arm), so any other answer would write a document and
# leave nothing to compare. The choices are `rivet_config::FleetRole::ALL`, in
# that order — `server`, `member` — so this number moves if that list does.
KEYS="$WORK/keys"
{
    echo "1"          # new fleet, or join one? -- 1 = start a new fleet (ADR-063 8)
    echo "$FLEET"     # fleet name
    echo "$SUFFIXES"  # suffixes  -- THIS CANNOT BE CHANGED LATER
    echo "y"          # keep these? [Y/n] -- the list is rendered and confirmed
    echo "$OPERATOR"  # operator name
    echo "1"          # menu: edit field 1, Hostname
    echo "$HOST"      # Hostname >
    echo "6"          # menu: edit field 6, Fleet
    echo "1"          # Fleet > 1 = server
    # ---- SPEC-05 5.0, the timezone, and it is driven WRONG first -------------
    echo "7"          # menu: edit field 7, Timezone
    echo "$TYPO"      # a well-formed name for a zone the image has no file for
    echo "w"          # write -- MUST be refused while a field is invalid
    echo "7"          # menu: edit field 7 again
    echo "$ZONE"      # the real zone
    echo "w"          # write -- composes the document AND mints the certificate
} > "$KEYS"

echo ""
echo "--- 1. the menu, in a pty"
# `HOME` is set on the command itself so the override reaches the child inside
# the pty rather than only `script`. `RIVET_FLEET_REPOSITORY` points it at the
# fixture repository above rather than `$MENU_HOME/.rivet/fleet` (which has no
# manifest at all), so R-178 P3's version question has something to answer.
if script -q -c "HOME=$MENU_HOME RIVET_FLEET_REPOSITORY=$FLEET_REPO $PROVISION" /dev/null \
    < "$KEYS" > "$WORK/menu.log" 2>&1
then
    ok "the menu ran to completion"
else
    cat "$WORK/menu.log" >&2
    fail "the menu exited non-zero"
fi

# **The screen was actually rendered in colour**, which is the reason for the
# pty. `screen::render` emits SGR sequences only when stdout is a terminal, so
# their absence means this ran down the pipe path and the person's code path is
# still unexecuted.
if grep -q "$(printf '\033\[')" "$WORK/menu.log"; then
    ok "stdout was a terminal — the colour branch ran"
else
    fail "no escape sequences: this was not a pty, and the rendered path is still unproven"
fi

for f in root.crt root.key browser.crt browser.key suffix; do
    [ -s "$MENU_CA/$f" ] && ok "menu wrote $f" || fail "the menu did not write $f"
done

# --- 1a. `w` WROTE the document, and the parser read it back -------------------
#
# `SPEC-05` §4.6 and §4.7. Until 2026-09-07 `w` minted the certificate, printed
# the document and exited successfully having written NOTHING, so a person who
# typed a key labelled `write` and watched output scroll past reasonably
# believed a document existed. Both halves are asserted: the file, and the fact
# that the tool handed the file back to `rivet-config`.
MENU_DOC="$MENU_HOME/.rivet/provision/$HOST.provisioning.toml"
if [ -s "$MENU_DOC" ]; then
    ok "w wrote $(basename "$MENU_DOC")"
else
    echo "  what is in the provision directory: \
$(ls "$MENU_HOME/.rivet/provision" 2>/dev/null | tr '\n' ' ')" >&2
    fail "w wrote no document — a menu key labelled as a write must write"
fi
grep -q "parsed by rivet-config" "$WORK/menu.log" \
    && ok "the written file was read back and parsed by the board's own parser" \
    || fail "w did not parse the file it wrote (SPEC-05 §4.7)"
# The document is not the notebook, and the two must not share a path.
[ -e "$MENU_HOME/.rivet/provision/$HOST.toml" ] \
    && fail "the document landed on the answers notebook's path" \
    || ok "the document and the answers notebook are different files"

# --- 1b. the TIMEZONE was asked for, and a wrong answer was refused ------------
#
# `SPEC-05` §5.0, ruled 2026-09-07. Three separate claims, because a menu that
# merely CARRIES the key would satisfy none of them:
#
#   * the tool said what it was checking the name against -- a real zone
#     database inside this container, rather than the list it falls back to on a
#     workstation that has none;
#   * a well-formed name for a zone the image has no file for was REFUSED, and the
#     refusal named the zone the operator meant. Accepting it is the defect: the
#     document parses, /etc/localtime is projected, musl maps nothing, and the
#     machine keeps UTC with nothing in any log saying so;
#   * `w` REFUSED TO WRITE while that answer stood. This is the half that makes
#     the question unskippable -- a field that could be left blank is a field
#     that gets left blank.
grep -q "checked against the zone database" "$WORK/menu.log" \
    && ok "the zone was checked against this container's real database" \
    || fail "the zone was not checked against a database (SPEC-05 5.0)"
grep -q "did you mean $ZONE" "$WORK/menu.log" \
    && ok "$TYPO was refused and $ZONE was named" \
    || fail "a well-formed name for a zone the image has no file for was not refused"
grep -q "Timezone is not valid" "$WORK/menu.log" \
    && ok "w refused to write while the zone was unanswered" \
    || fail "w wrote a document with an invalid timezone in it"

# --- 1c. the document carries what the ROLE needs ------------------------------
#
# `SPEC-05` §5. Every line here was ABSENT from the document hand-written on
# 2026-09-07 and every one cost a diagnosis cycle; the listener cost a card.
# Asserted against the file on disk rather than against the tool's own output,
# because what goes on a card is bytes.
if [ -s "$MENU_DOC" ]; then
    grep -q "^timezone = \"$ZONE\"$" "$MENU_DOC" \
        && ok "the document carries timezone = \"$ZONE\"" \
        || fail "the document has no timezone key (SPEC-05 5.0)"

    # **Before the first table header, or it is not a top-level key.** TOML puts
    # a bare key into whichever table precedes it, so a `timezone` written after
    # `[network.eth0]` would be `network.eth0.timezone` and refused -- and there
    # is no appending one to a finished document, which is the trap Aura's own
    # instruction walks a person into.
    ZONE_LINE=$(grep -n "^timezone = " "$MENU_DOC" | cut -d: -f1)
    TABLE_LINE=$(grep -n "^\[" "$MENU_DOC" | head -1 | cut -d: -f1)
    if [ -n "$ZONE_LINE" ] && [ -n "$TABLE_LINE" ] && [ "$ZONE_LINE" -lt "$TABLE_LINE" ]; then
        ok "timezone is line $ZONE_LINE, the first table is line $TABLE_LINE"
    else
        fail "timezone is not written above the first table (line $ZONE_LINE vs $TABLE_LINE)"
    fi

    # **The listener that cost a card.** `127.0.0.1:7777` produced a headless
    # board that answered nothing and the only recovery was rebuilding it.
    grep -q '^listen = "0.0.0.0:7777"$' "$MENU_DOC" \
        && ok "the fleet server listens on 0.0.0.0:7777" \
        || fail "the fleet server's document does not bind a reachable address (SPEC-05 5)"
    grep -q "^\[control-plane\]$" "$MENU_DOC" \
        && ok "the [control-plane] table is written out rather than left to the default" \
        || fail "the document has no [control-plane] table to edit"

    for s in rivet-fleetd rivet-proxyd rivet-mdnsd; do
        grep -q "^\[services\.$s\]$" "$MENU_DOC" \
            && ok "the document enables $s" \
            || fail "the document does not enable $s (SPEC-05 5)"
    done

    # **Addressing is left at dhcp here, so no nameservers are asked and no
    # resolver is declared**: the lease fills it. The static half, asked and
    # composed, is `rehearse-member.sh`'s server.
    grep -q '^\[network\.resolver\]' "$MENU_DOC" \
        && fail "a dhcp document declares [network.resolver], overriding its lease" \
        || ok "a dhcp document declares no resolver"
    grep -q "Nameservers" "$WORK/menu.log" \
        && fail "the menu showed or asked Nameservers for a dhcp machine" \
        || ok "the menu did not ask a dhcp machine for nameservers"

    # And the tool's own read-back agrees about the listener, which is the line
    # a person actually reads before building a card.
    grep -q "control plane  0.0.0.0:7777" "$WORK/menu.log" \
        && ok "the parser read the listener back as 0.0.0.0:7777" \
        || fail "the read-back did not report a reachable control plane"

    echo ""
    echo "  the composed document:"
    sed 's/^/    /' "$MENU_DOC"
    echo ""
fi

MENU_LEAF="$MENU_CA/server-$HOST.$(echo "$SUFFIXES" | cut -d, -f1).crt"
if [ -s "$MENU_LEAF" ]; then
    ok "menu minted $(basename "$MENU_LEAF")"
else
    # The menu names the file after the FIRST permitted suffix, so a miss here
    # is worth showing rather than asserting blind.
    echo "  what the menu left: $(ls "$MENU_CA" 2>/dev/null | tr '\n' ' ')" >&2
    fail "the menu minted no server certificate"
fi

# --- 2. the same answers, down the command line -------------------------------
echo ""
echo "--- 2. the command line, same answers"
mkdir -p "$CLI_CA"
HOME="$CLI_HOME" "$PROVISION" ca "$SUFFIXES" "$FLEET" "$CLI_CA" "$OPERATOR" --no-install \
    >"$WORK/cli-ca.log" 2>&1 \
    || { cat "$WORK/cli-ca.log" >&2; fail "the command-line mint was refused"; }

# **The store was not opened, and this asserts it rather than trusting it.**
# `browser::place` is the only caller of `certutil`, and it says `filed into`
# when it succeeds. A rehearsal that filed something is a rehearsal that has to
# be cleaned up by hand, so it is a failure here.
if grep -q "filed into" "$WORK/cli-ca.log"; then
    cat "$WORK/cli-ca.log" >&2
    fail "the rehearsal INSTALLED a certificate — --no-install did not take"
else
    ok "nothing was filed into a certificate store"
fi
[ -s "$CLI_CA/browser.pfx" ] && ok "browser.pfx was still written" \
    || fail "--no-install skipped the bundle as well as the store"

# **The names are composed the way `every_name_for` composes them**, because
# that is what the command line asks a person to type. If this list and the
# menu's disagree, the certificates below will say so.
WANT=$(echo "$SUFFIXES" | tr ',' '\n' | while read -r s; do
    [ -n "$s" ] || continue
    printf '%s.%s\n' "$HOST" "$s"
done | tr '\n' ',' | sed 's/,$//')
echo "  asking for: $WANT"

# **The composing default, from a BARE hostname, reaches the names a person
# would have typed.** This is findings §3.3's fix: `ca server rivet-01.local`
# used to take that literally and silently replace a three-name leaf with a
# one-name one on the production fleet server.
#
# Run before the literal form so that the leaf compared in §3 is unambiguously
# the one `--names` minted.
HOME="$CLI_HOME" "$PROVISION" ca server "$HOST" "$CLI_CA" >"$WORK/cli-composed.log" 2>&1 \
    || { cat "$WORK/cli-composed.log" >&2; fail "the composing form of ca server was refused"; }
COMPOSED=$(sed -n 's/^  minting for //p' "$WORK/cli-composed.log" | tr -d ' ')
if [ "$COMPOSED" = "$(echo "$WANT" | tr -d ' ')" ]; then
    ok "ca server $HOST composed $COMPOSED with no list typed"
else
    echo "  composed: $COMPOSED" >&2
    echo "  typed:    $WANT" >&2
    fail "the composing form did not reach the names a person would have typed"
fi

# **`--names` is the LITERAL list**, and that is deliberately the path exercised
# for the comparison below. Composed-versus-composed would be one code path held
# against itself; told-versus-worked-out is the claim worth making.
HOME="$CLI_HOME" "$PROVISION" ca server --names "$WANT" "$CLI_CA" >"$WORK/cli-server.log" 2>&1 \
    || { cat "$WORK/cli-server.log" >&2; fail "the command-line server certificate was refused"; }

CLI_LEAF="$CLI_CA/server-$(echo "$WANT" | cut -d, -f1).crt"
[ -s "$CLI_LEAF" ] && ok "command line minted $(basename "$CLI_LEAF")" \
    || fail "the command line minted no server certificate"

# --- 3. the two certificates, side by side ------------------------------------
#
# **Not a byte diff.** Two roots minted seconds apart have different keys,
# serials and validity, so every file differs and none of that is a defect. What
# must agree is what a browser reads and what the authority permits: the subject
# alternative names, the subject, and the root's name constraint.
echo ""
echo "--- 3. the artifacts, held side by side"

sans() {
    openssl x509 -in "$1" -noout -ext subjectAltName 2>/dev/null \
        | tr ',' '\n' | sed -n 's/.*\(DNS:[^ ,]*\).*/\1/p' | sort -u
}
constraint() {
    openssl x509 -in "$1" -noout -ext nameConstraints 2>/dev/null \
        | tr ',' '\n' | sed -n 's/.*\(DNS:[^ ,]*\).*/\1/p' | sort -u
}
subject_cn() {
    openssl x509 -in "$1" -noout -subject 2>/dev/null | sed 's/.*CN *= *//'
}

if [ -s "$MENU_LEAF" ] && [ -s "$CLI_LEAF" ]; then
    sans "$MENU_LEAF" > "$WORK/menu.sans"
    sans "$CLI_LEAF" > "$WORK/cli.sans"

    # A leaf with no names at all would compare equal to another with none, and
    # that is §156.2's defect passing this test. Assert there is something.
    [ -s "$WORK/menu.sans" ] || fail "the menu's leaf carries NO subject alternative name"

    # **Compared as strings, because this image has no `diff`.** The lists are
    # sorted, de-duplicated and a handful of lines long, so a `=` says exactly
    # what a diff would and needs nothing installed. Reaching for a tool that is
    # not in the container is how the first run of this script "failed".
    if [ "$(cat "$WORK/menu.sans")" = "$(cat "$WORK/cli.sans")" ]; then
        ok "both leaves carry exactly: $(tr '\n' ' ' < "$WORK/menu.sans")"
    else
        echo "  menu: $(tr '\n' ' ' < "$WORK/menu.sans")" >&2
        echo "  cli:  $(tr '\n' ' ' < "$WORK/cli.sans")" >&2
        fail "the menu and the command line minted DIFFERENT names — the two adapters have diverged again"
    fi

    # **Every permitted suffix reached the leaf.** The lists agreeing with each
    # other is not the same claim as either being right: two adapters can be
    # wrong together, and §149.4 makes a short list unfixable later.
    echo "$SUFFIXES" | tr ',' '\n' | while read -r s; do
        [ -n "$s" ] || continue
        if grep -qx "DNS:$HOST.$s" "$WORK/menu.sans"; then
            echo "  ok   the leaf carries $HOST.$s"
        else
            echo "  FAIL: the leaf does NOT carry $HOST.$s" >&2
            echo "fail" > "$WORK/.failed"
        fi
    done
    [ -f "$WORK/.failed" ] && FAILED=yes

    [ "$(subject_cn "$MENU_LEAF")" = "$(subject_cn "$CLI_LEAF")" ] \
        && ok "both leaves have the subject $(subject_cn "$MENU_LEAF")" \
        || fail "subjects differ: menu '$(subject_cn "$MENU_LEAF")' vs cli '$(subject_cn "$CLI_LEAF")'"
fi

constraint "$MENU_CA/root.crt" > "$WORK/menu.nc"
constraint "$CLI_CA/root.crt" > "$WORK/cli.nc"
[ -s "$WORK/menu.nc" ] || fail "the menu's root constrains NOTHING — it could vouch for any site"
if [ "$(cat "$WORK/menu.nc")" = "$(cat "$WORK/cli.nc")" ]; then
    ok "both roots permit exactly: $(tr '\n' ' ' < "$WORK/menu.nc")"
else
    echo "  menu: $(tr '\n' ' ' < "$WORK/menu.nc")" >&2
    echo "  cli:  $(tr '\n' ' ' < "$WORK/cli.nc")" >&2
    fail "the two roots carry DIFFERENT name constraints"
fi

# The suffix note is read by nothing (Q13) and is still written by both paths;
# a note a person might `cat` and believe is worth holding to the same standard.
if [ -s "$MENU_CA/suffix" ] && [ -s "$CLI_CA/suffix" ]; then
    [ "$(cat "$MENU_CA/suffix")" = "$(cat "$CLI_CA/suffix")" ] \
        && ok "both suffix notes say the same thing" \
        || fail "the suffix notes disagree: menu '$(tr '\n' ',' < "$MENU_CA/suffix")' vs cli '$(tr '\n' ',' < "$CLI_CA/suffix")'"
fi

echo ""
if [ -n "$FAILED" ]; then
    echo "rehearse-menu.sh: FAIL — the two adapters do not compose the same certificate."
    exit 1
fi
echo "rehearse-menu.sh: PASS — a person's keystrokes and a script's arguments mint the same thing."
