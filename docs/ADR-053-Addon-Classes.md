# ADR-053: Addon Classes — a library joins an application by a signed class, not a signed reference

* **Status:** Accepted and built, and **proven on hardware 2026-09-19**. §2's
  primitive is implemented in `rivet-core` and `rivet-sysd`, `accepts-class` as
  a list per `.rulings/` R-162 R28. **§5's boundary is now built too**:
  `rivetos-addon` carries the crossing (`.rulings/` R-196 to R-198), and the
  first article opened a class-matched object on a board — which is also where
  this ADR's own gap was found, since the supervisor had been putting an
  addon's directory on `LD_LIBRARY_PATH` and into no Landlock rule. A host
  learns **which** objects it was joined to through `RIVET_ADDONS`, and **a
  library carrying several objects names the one it joins by** — `.rulings/`
  R-201, built, `SPEC-02` §11.3.
* **Date:** 2026-09-13
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Ruled:** `PLAN-Feature-Set-2026-09-12` §2 decision 10 — *"Addon classes
  are a platform primitive and land here"* — with the shape given in §6.
* **Answers:** how an addon can be installed onto a machine without
  republishing the application it joins, while keeping every join a decision
  two signed manifests already agreed to rather than one made by a
  coincidence of names.
* **Supersedes nothing.** It adds one optional key to each of the two
  manifest kinds `SPEC-02` already defines, and changes nothing about how an
  application's *declared* closure (`SPEC-02` §5) resolves.
* **Builds on:** `ADR-015` §3.2 and §3.3 (the manifest as a permission
  request, and that a default leans closed), `ADR-011` (one authority per
  fact), `SPEC-02` (what a library artifact is and how it installs).

---

## 1. Context

A manifest is signed and cannot be edited on the machine (`ADR-015` §4.1),
and `SPEC-02` §5 makes a library available to an application because that
application's own manifest names its exact reference — a digest of one
artifact's bytes. That is the right rule for a dependency an author chose at
build time, and it is the wrong rule for an addon: installing a new one would
mean republishing every application it should reach, which is exactly the
friction the addon design (`PLAN-Feature-Set-2026-09-12` §13) exists to
remove.

**The addon feature set is not this release** (§13 again), but the primitive
it needs is core Rivet and is decided here so that work is genuinely
contained when it starts.

---

## 2. Decision

* **An application declares the classes of library it accepts**, as a list in
  its signed manifest (`.rulings/` R-162 R28).
* **A library declares the class it provides**, in its own signed manifest —
  one string, never a list; a library provides one kind of thing.
* **The supervisor makes every installed library whose class appears in that
  list available** to that application at discovery, and again after an
  install, the way it already re-runs discovery
  (`crate::installed::Watch::look`).

**Nothing joins by being present.** The library's one declared class must
appear somewhere in the application's accepted-class list, or nothing
happens:

| Application's `accepts-class` | A library's `provides-class` | Result |
| :-- | :-- | :-- |
| `[]` (absent or empty) | (anything) | unchanged — resolves its explicit `library` closure exactly as `SPEC-02` §6 already does, and nothing else |
| `["x", …]` | absent | that library is not a candidate; it is reported by `crate::libraries::discover_installed` and filtered by `rivet_core::service::addon_library_path` |
| `["x", …]` | `"y"` (`y` not in the list) | not joined — this is exact equality per entry, never a prefix or a fuzzy match |
| `["x", "y"]` | `"y"` | joined: the library's object directory is added to the application's `LD_LIBRARY_PATH`, after its explicit closure |
| `["x", "x"]` | `"x"` | joined ONCE — matching walks the installed libraries, not the accepted list, so a repeated entry cannot join one library twice |

A library with no class and an application accepting none (an empty list)
each behave exactly as before this ADR. An application's existing explicit
`library` dependencies (`SPEC-02` §5) keep resolving unchanged, and an addon
join is additive to them, never a replacement.

---

## 3. The manifest keys

Two new optional keys, one per kind, each appended at the **end** of its
kind's `KEYS` array (`rivet_core::package::application::Manifest`'s
`accepts-class`, `rivet_core::package::library::Manifest`'s
`provides-class`). Appending rather than inserting is what lets every `.rvt`
signed before this ADR keep parsing — `SPEC-02` §5's own `library` key set
the precedent, and `cpu`'s 2026-09-09 arrival is the most recent one: a key
added to the end of a canonical-order array cannot put an older manifest out
of order, and its absence is read as the closed end of `ADR-015` §3.3's axis
rather than as a manifest an older build could not produce.

```toml
# an application's [package.metadata.rivet]
accepts-class = ["temperature-sensor", "humidity-sensor"]

# a library's manifest, from `cargo rivet library --provides-class`
provides-class = "temperature-sensor"
```

* **Optional, and absence (or an empty list) means the closed end of the
  axis** — *takes no addon* for an application, *joins no application by
  class* for a library. Every manifest signed before this ADR carries
  neither key and is refused by nothing: `Manifest::parse` treats an
  absent optional key as `None` (`provides-class`) or an empty list
  (`accepts-class`) rather than as a missing required one, so no existing
  `.rvt` is invalidated (proven by
  `a_manifest_from_before_accepts_class_existed_still_parses` and
  `a_manifest_from_before_provides_class_existed_still_parses`).
* **Inside the signature, like every other permission request** (`ADR-015`
  §4.1). An application's accepted-class list is a capability it is granted
  at build time by whoever signed it, not something an operator can widen
  after the fact by installing an unrelated addon; a library's provided
  class is the same claim about what that artifact is, made by its own
  author.
* **`accepts-class` is a LIST; `provides-class` stays one string** —
  `.rulings/` R-162 R28, §4 below. A library provides one kind of thing; an
  application may accept several.
* **The same alphabet as `settings` and `proxy-path`**
  (`rivet_core::package::application::is_class_name`, `CLASS_RULE`):
  lowercase letters, digits and hyphens, beginning with a letter. Kept as its
  own predicate rather than reusing `is_setting_name`'s name, for the reason
  `is_proxy_path` is kept separate from it — two rules about two different
  things that happen to agree today should not become one rule that a later
  change to either silently changes for both.
* **`cargo rivet package` and `cargo rivet library` write these keys** from
  `Cargo.toml` metadata and a CLI flag respectively, following the path every
  other manifest key already flows through — `crate::crate_meta::DECLARED`
  and the top-level usage text list every key this build accepts
  (`cmd/cargo-rivet/src/main.rs`'s `the_usage_lists_every_manifest_key_this_
  build_accepts` enforces the second half of that).

---

## 4. The decision boundary this ADR settles

`PLAN-Feature-Set-2026-09-12` §6.2 named two questions as open: the class
name's format, and whether one application may accept several. The
maintainer ruled on both (`.rulings/` R-162):

* **Free text, not reverse-DNS — R27 stays open.** A class is compared for
  exact equality by two authors who must already coordinate on every other
  byte the join depends on — the manifest key's spelling, the loader's
  SONAME, the addon's own shape once §5's interface is built. A namespacing
  scheme buys nothing a socket name or a URL path needs it for, because
  those are compared against strangers and a class is not: the
  application's author and the library's author are, by construction, the
  two parties who have to agree on the string. `SETTING_RULE`'s alphabet is
  reused rather than invented, for the same reason every other short
  machine-read token in this manifest uses it. The maintainer did not rule
  on this question, so free text remains as built.
* **`accepts-class` is a list — R28.** An application may accept several
  classes at once; `provides-class` was not ruled and stays one string, a
  library still provides one kind of thing. **This went against the
  recommendation** (keep one class, widen later if a real application needs
  it), **and the ruling takes the change while no signed manifest carries
  the key** — the list form is therefore the only form that ever ships, with
  no compatibility shim for a single-string form that never shipped.
  Discovery joins a library whose `provides-class` is ANY of the
  application's accepted classes (§2's table); a repeated entry cannot join
  one installed library twice, because matching walks the installed
  libraries rather than the accepted list.

---

## 5. The boundary this ADR does NOT cross

This ADR's primitive answers *which* addon is available to *which*
application. **How one calls the other is `rivetos-addon`'s**, built in its
own pass with its own tests, and nothing here assumes a shape for it.

What that crate settled is worth naming so a reader of this ADR is not left
with a gap where a contract is. It has **two** entries, and they are
different in kind:

* **A serialized one.** `Describe`, `Configure` and `Poll` cross as bytes in
  a versioned envelope, because Rust promises no type layout between
  separate compilations and a direct call would bet the host's address space
  on two builds agreeing — the argument §6 restates.
* **A C-layout one, for drawing.** A background writes into the host's own
  surface through a versioned `#[repr(C)]` descriptor, confirmed by the
  maintainer as a deliberate exception to the rule above (`.rulings/` R-199
  D2). The reason is arithmetic rather than preference: thirty frames a
  second into a 1080p surface is 249 MB a second, which no serialization
  carries. The exception's limit is its argument — `repr(C)` over
  fixed-width scalars and a pointer is the **C ABI of the target**, which is
  a platform's guarantee rather than a language's, and is therefore the one
  layout two separately compiled Rust builds do agree on.

Neither entry changes anything this ADR decides. The class is still what
joins a library to an application, and a host that opens an addon still asks
it what it is before it trusts it.

### 5.1 A theme's card has three answers, and `null` is one of them

A background pack describes, per theme, the card a host should draw over it
(`rivetos_addon::Theme::cards`, of type `rivetos_addon::Cards`). The pack has
three different things to say, and the wire keeps all three apart
(`.rulings/` R-210 D23):

| On the wire | Means | A host draws |
| :-- | :-- | :-- |
| `cards` omitted | no opinion | the fallback `Background::prefers_dark_cards` names: a plain dark card when it is set, no card when it is not |
| `"cards": null` | no card over this theme | no rectangle, whatever the background prefers |
| `"cards": { … }` | this card | that card |

A pack that has no opinion **omits** the key; it never writes `null` for it,
and the type refuses to serialize *no opinion* on its own rather than write
the `null` that means something else.

**This took no `rivetos_addon::abi::VERSION` bump, and the reasoning is
the whole decision.** A host and a pack built on opposite sides of the change
disagree about exactly one combination: `null` over a background whose
`prefers_dark_cards` is set.

* An older host reads a newer pack's `null` as *no opinion* and draws its
  plain dark fallback card where none was asked for. The picture is plainer
  than the pack wanted and stays legible.
* A newer host reads an older pack's `null` as *no card*, because the older
  crate wrote *no opinion* as `null`. The shipped background pack sent `null`
  only over Solid Black, which does not prefer dark cards, so its picture is
  identical under either host; the pack's own test
  `no_row_asks_for_no_card_where_an_older_host_would_draw_one` keeps every
  row in the combinations both hosts read alike.

Every other combination reads the same both ways. A version bump would have
made every installed addon — the reading sources included, which never
describe a theme at all — unloadable by a new host until rebuilt, which is a
far larger cost than the one combination it protects. The soname
(`lib<class>.so.1`) is unchanged for the same reason.

---

## 6. Rejected alternatives

| | why not |
| :-- | :-- |
| **§6.3's direct Rust interface between an addon and its host** | Rust gives no cross-compilation-unit layout guarantee, so a direct call bets that the host and every addon were built by the same compiler with the same flags — a bet that fails as memory corruption, not as a refusal. It would also couple an addon's build to its host's, defeating separate repositories. This is §6.3's own argument; it is restated here because it is also the reason this ADR stops at §2 and does not reach for the same shortcut |
| **A data-only install that sidesteps signing** | an addon that could be dropped onto the machine without carrying a signature would be a second, weaker distribution channel beside every other artifact kind `SPEC-02` §2 already covers. Reusing the same container, the same verifier and the same trust store (`SPEC-02` §7.3) is what makes a library "a payload, not a new distribution channel" — an addon is a library, and this ADR does not invent an exception for it |
| **Reverse-DNS or versioned class names** | §4. Two authors who already have to agree on every other byte of the join gain nothing from a namespacing scheme designed for strangers |
| **A `provides-class` list, mirroring `accepts-class`** | not ruled on (`.rulings/` R-162); a library provides one kind of thing, so widening the provide side has no argument behind it the way widening the accept side did |
| **Matching by soname or by product name instead of a declared class** | either would let a library join an application by *coincidence* — a name chosen for a human reader, never signed as a joining contract. `SPEC-02` §3.2 already makes exactly this point about a reference's name-and-version half: "nothing resolves anything with that split." A class is a value both authors sign specifically to mean *these two join*, which a soname or a product name was never asked to mean |

---

## 7. Version skew

Both keys are optional and appended last, so the direction that matters is
free: **a card built before this ADR still runs every application and every
library exactly as it did**, because neither key exists in a manifest that
predates it and absence is the closed end of the axis on both sides.

The other direction costs nothing new either. An application declaring
`accepts-class` and a library declaring `provides-class` still install and
resolve their explicit `library` closures under a machine's existing rules;
the new behaviour — a class-matched addition to `LD_LIBRARY_PATH`, a matching
addition to the Landlock ruleset, and the `RIVET_ADDONS` variable that names
what joined (`SPEC-02` §11.4) — is additive on every one of the three, so a
machine with no matching library installed behaves exactly as if neither
manifest had ever mentioned a class.

---

## 8. What is proved, by what method, and what is not

`AGENTS.md` §6.

**Proved, by unit tests with no I/O (`AGENTS.md` §4), in
`rivet-core/src/service.rs`:**

* a matching class is made available (`a_matching_class_is_made_available`);
* a mismatched class is not joined (`a_mismatched_class_is_not_joined`);
* a library with no class is not joined
  (`a_library_with_no_class_is_not_joined`);
* an application accepting no class is unchanged
  (`an_application_accepting_no_class_is_unchanged`), and an explicitly empty
  list joins nothing either (`an_explicitly_empty_class_list_joins_nothing`);
* **R-162 R28: an application accepting two classes joins a library of each**
  (`an_application_accepting_two_classes_joins_a_library_of_each`); a library
  matching neither of two accepted classes is still excluded
  (`a_library_matching_neither_of_two_accepted_classes_is_not_joined`); a
  duplicate entry in the accepted list does not join one installed library
  twice (`duplicate_classes_in_the_accepted_list_do_not_double_join_a_library`);
* the join is deterministic regardless of listing order
  (`two_matches_join_sorted_by_reference_regardless_of_input_order`);
* an old-format application manifest and an old-format library manifest each
  still parse (`rivet-core/src/package/application/tests.rs`,
  `rivet-core/src/package/library/tests.rs`), and a single class still
  round-trips as a one-element list
  (`a_single_accepted_class_round_trips_as_a_one_element_list`).

**Proved, by unit tests against a fake kernel, in
`cmd/rivet-sysd/src/applications.rs` and `.../libraries.rs`:**

* the whole primitive through the real discovery path — a matching library
  joins `LD_LIBRARY_PATH` with no explicit reference naming it
  (`an_installed_library_of_a_matching_class_joins_at_discovery`);
* a mismatch survives the real discovery path too
  (`a_mismatched_class_does_not_join_at_discovery`);
* **R-162 R28 through the real discovery path too**: an application
  accepting two classes joins a library of each
  (`an_application_accepting_two_classes_joins_a_library_of_each_at_discovery`);
* **discovery re-runs after an install and picks up a newly installed
  matching library** — the acceptance criterion the plan named explicitly —
  proved by calling `discover` a second time over the same definitions after
  the library arrives, on `crate::installed::Watch::look`'s own footing
  (`a_library_installed_after_first_discovery_joins_on_the_next_pass`);
* an unreadable `/state/lib`, an unparseable entry and an unreadable library
  manifest each degrade to *no addon joins*, never to a fault that stops
  discovery (`libraries.rs`'s own suite).

**Proved, by unit tests, in `cmd/cargo-rivet`:** `accepts-class` flows from
`Cargo.toml` into the packaged manifest as a list — one entry or several —
and is refused by the same rule the core parser enforces (`crate_meta.rs`);
`--provides-class` is parsed into `cargo rivet library`'s `Args` and is
optional (`library.rs`).

**Not proved, and stated plainly:**

* **No library has been published into, deployed to, or discovered on a real
  machine or in QEMU.** Every test above runs against `FakeKernel` or with no
  I/O at all. `SPEC-02` §10.6 already carries the same limit for the fleet
  repository half of a library's life; this ADR adds the same limit for the
  class-matching half.
* **No addon has ever called into a host, or been called by one** — §5
  deliberately stops at *which* addon is available, and leaves *how one
  calls the other* to its own pass. This ADR proves only that the right
  library becomes *available*, never that anything on either side of §6.3's
  interface can use it.
* **No addon class has been published into, deployed to, or matched on a
  real machine or in QEMU** — see the fleet-repository limit above, restated
  for R28's list widening: the list form has run only under `FakeKernel` and
  with no I/O.

**Fixed since this ADR was written, by `.rulings/` R-162:**
`cargo rivet package`'s emitted usage text and `crate_meta::DECLARED` were
missing `cpu` — found while adding `accepts-class` beside it, left alone at
the time because fixing an unrelated pre-existing gap in the same change
would have been two changes wearing one commit, and closed in the change
that built R28. `cmd/cargo-rivet/src/crate_meta.rs` now reads `cpu` from
`Cargo.toml` metadata the way it reads `memory`, and the usage text lists it.

---

## History

**2026-10-04 — a theme's `cards: null` stopped meaning the same as an
omitted `cards` (§5.1, `.rulings/` R-210 D23).** The field was an `Option`,
and `rivetos_addon`'s own documentation said ~~`null` and an omitted field
read the same way~~ — both took the `prefers_dark_cards` fallback, so a pack
could not ask for no card over a background that prefers dark ones. The
recommendation was to wait for the next add-on protocol bump; the ruling
changed the type at once and left the addon repositories to rebuild. §5.1
records the three states and why the change itself needed no protocol
version.

**2026-09-19 — §5 rewritten: the call boundary exists, and it has a C-layout
exception in it.** §5 read *"decided in the plan but deliberately not built
here"* and closed with *"it must be a minimal ABI boundary, not a direct Rust
interface, because Rust does not promise type layout between separate
compilations."* Both halves have since been built, and the second sentence is
no longer the whole rule: `.rulings/` R-199 D2 confirmed a versioned
`#[repr(C)]` frame descriptor for drawing, knowing it is an exception to it.
The old text would have left a reader believing a layout never crosses, which
is the one belief that makes the exception dangerous rather than narrow. The
reasoning it carried is kept, in the body, as the reason the **serialized**
entry is serialized — that argument did not weaken, it merely stopped being
the only one.

**2026-09-13 — `accepts-class` widened from one string to a list
(`.rulings/` R-162 R28).** §4 originally settled *"one class, not several"*
alongside the free-text question, arguing that a `Vec` would cost nothing to
declare but that nothing in the plan needed it, and that a single value was
the smaller permission request. Put to the maintainer alongside R27 (the
class name format) and the carry-pin questions, he ruled against that
recommendation: *"Make `accepts-class` a list now."* The ruling took the
change immediately, while no signed manifest yet carried the key — so
`ADR-053`'s original "smaller permission request, widen later if needed"
argument does not survive it: there was no installed base to protect, and
the whole cost of shipping the list form from the start was a wider array
type in one place, against the cost of a second manifest-key migration once
a real application needed two addon families. `provides-class` was not part
of the ruling and stays one string, on the "a library provides one kind of
thing" argument the original ADR always made about the provide side alone.
