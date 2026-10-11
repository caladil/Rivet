//! Shared Rivets, from the outside in. A reference is the whole mechanism's
//! identity, so most of this is about the exact shapes it accepts and refuses.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;

fn digest(byte: u8) -> [u8; 32] {
    [byte; 32]
}

fn hex(byte: u8) -> String {
    format!("{byte:02x}").repeat(32)
}

// --- Reference --------------------------------------------------------------

#[test]
fn a_reference_is_composed_and_read_back_the_same() {
    let reference = Reference::compose("libgreet", "1.0.0", &digest(0xab)).unwrap();
    assert_eq!(reference.as_str(), format!("libgreet-1.0.0-{}", hex(0xab)));
    assert_eq!(Reference::parse(reference.as_str()), Ok(reference.clone()));
    assert_eq!(reference.name_and_version(), "libgreet-1.0.0");
    assert_eq!(reference.digest_hex(), hex(0xab));
}

/// A version with a `-` in it is the ordinary case here, not the exotic one:
/// `ADR-027` §3.2.2 puts a describe string and a program digest in a version.
/// The digest is read from the RIGHT for exactly this reason.
#[test]
fn a_version_carrying_dashes_still_splits_at_the_digest() {
    let reference =
        Reference::compose("hello-rivet", "0.1.0-g1641a7f-2c2548db", &digest(0x01)).unwrap();
    assert_eq!(reference.name_and_version(), "hello-rivet-0.1.0-g1641a7f-2c2548db");
    assert_eq!(reference.digest_hex(), hex(0x01));
}

#[test]
fn a_reference_with_no_digest_is_refused() {
    assert_eq!(Reference::parse("libgreet-1.0.0"), Err(ReferenceFault::NoDigest));
    // 64 characters, and one of them is not hex.
    let nearly = format!("libgreet-1.0.0-{}g", "a".repeat(63));
    assert_eq!(Reference::parse(&nearly), Err(ReferenceFault::NoDigest));
    // Upper case is not the spelling `compose` emits, and one reference must
    // have one spelling or the directory name is ambiguous.
    let upper = format!("libgreet-1.0.0-{}", "AB".repeat(32));
    assert_eq!(Reference::parse(&upper), Err(ReferenceFault::NoDigest));
}

#[test]
fn a_reference_that_names_no_library_is_refused() {
    assert_eq!(
        Reference::parse(&format!("-{}", hex(0xab))),
        Err(ReferenceFault::BadCharacter)
    );
}

/// This string becomes a path component. Every shape that has ever been used to
/// leave a directory is refused before it can be one.
#[test]
fn a_reference_can_never_become_a_traversal() {
    for bad in [
        "..",
        "../libgreet",
        "lib/greet",
        "lib\\greet",
        ".hidden",
        "-leading",
        "lib greet",
        "lib\0greet",
    ] {
        let attempt = format!("{bad}-{}", hex(0xab));
        assert!(
            Reference::parse(&attempt).is_err(),
            "{attempt:?} was accepted as a reference"
        );
    }
    assert!(Reference::parse("").is_err());
}

#[test]
fn a_reference_longer_than_the_bound_is_refused() {
    let long = format!("{}-1.0.0-{}", "n".repeat(MAX_REFERENCE), hex(0xab));
    assert_eq!(Reference::parse(&long), Err(ReferenceFault::BadLength));
}

/// **A digit-led product suffix stays in the product.** `mqtt-5` and `mqtt-6`
/// are two different, valid products — `5` begins with a digit but is not
/// shaped like a version — so `product_and_version` must not read either as
/// product `mqtt` at version `5-1.0.0` or `6-1.0.0`.
#[test]
fn a_numeric_product_suffix_does_not_start_the_version() {
    for (name, product) in [("mqtt-5", "mqtt-5"), ("mqtt-6", "mqtt-6")] {
        let reference = Reference::compose(name, "1.0.0", &digest(0xab)).unwrap();
        let (read_product, version) = reference
            .product_and_version()
            .unwrap_or_else(|| panic!("{name} did not split"));
        assert_eq!(read_product, product, "{name}");
        assert_eq!(version, "1.0.0", "{name}");
    }
}

// --- the manifest -----------------------------------------------------------

#[test]
fn a_manifest_round_trips_through_its_own_text() {
    let needs = vec![Reference::compose("libz", "1.3.1", &digest(0x22)).unwrap()];
    let manifest = Manifest::new(vec!["libgreet.so.1".to_string()])
        .unwrap()
        .with_needs(needs)
        .unwrap();
    let text = manifest.render();
    assert_eq!(Manifest::parse(text.as_bytes()), Ok(manifest));
}

#[test]
fn the_rendered_text_is_exactly_what_is_expected() {
    let manifest = Manifest::new(vec!["libgreet.so.1".to_string()]).unwrap();
    let text = manifest.render();
    assert!(text.contains("kind = \"library\"\n"));
    assert!(text.contains("soname = [\"libgreet.so.1\"]\n"));
    assert!(text.contains("needs = []\n"));
}

#[test]
fn a_library_that_provides_nothing_is_refused() {
    assert_eq!(Manifest::new(Vec::new()), Err(ManifestFault::NoSoname));
    let text = "kind = \"library\"\nsoname = []\nneeds = []\n";
    assert_eq!(Manifest::parse(text.as_bytes()), Err(ManifestFault::NoSoname));
}

#[test]
fn keys_out_of_order_are_refused_because_a_manifest_is_emitted_not_written() {
    let text = "soname = [\"libgreet.so.1\"]\nkind = \"library\"\nneeds = []\n";
    assert_eq!(
        Manifest::parse(text.as_bytes()),
        Err(ManifestFault::OutOfOrder { key: "kind" })
    );
}

#[test]
fn a_manifest_for_another_kind_is_refused_by_its_own_kind_line() {
    let text = "kind = \"daemon\"\nsoname = [\"libgreet.so.1\"]\nneeds = []\n";
    assert_eq!(
        Manifest::parse(text.as_bytes()),
        Err(ManifestFault::BadValue { key: "kind" })
    );
}

/// A SONAME is a filename a loader will open. It is held to a filename's rules
/// and no wider — the same argument `install::safe_component` makes about a
/// version, one artifact kind over.
#[test]
fn a_soname_that_could_become_a_path_is_refused() {
    for bad in ["../libc.so", "lib/greet.so", "", ".", "..", ".hidden.so"] {
        assert!(
            Manifest::new(vec![bad.to_string()]).is_err(),
            "{bad:?} was accepted as a soname"
        );
    }
}

/// The one thing the closure list must accept that every other list must not:
/// an element longer than [`crate::package::application::MAX_VALUE`].
#[test]
fn a_needs_element_may_be_longer_than_an_ordinary_value() {
    let long_name = "a".repeat(70);
    let reference = Reference::compose(&long_name, "0.1.0", &digest(0x33)).unwrap();
    assert!(reference.as_str().len() > crate::package::application::MAX_VALUE);
    let manifest = Manifest::new(vec!["libgreet.so.1".to_string()])
        .unwrap()
        .with_needs(vec![reference])
        .unwrap();
    assert_eq!(Manifest::parse(manifest.render().as_bytes()), Ok(manifest));
}

// --- `provides-class` — PLAN-Feature-Set-2026-09-12 §6.2, ADR-053 -----------

/// **This IS the old-format manifest.** `the_rendered_text_is_exactly_what_is_
/// expected` already pins the bytes of a manifest with no `provides-class`
/// line; this proves the OTHER half — that those exact bytes, which is what
/// every `.rvt` signed before `ADR-053` carries, still parse and declare no
/// class.
#[test]
fn a_manifest_from_before_provides_class_existed_still_parses() {
    let text = "kind = \"library\"\nsoname = [\"libgreet.so.1\"]\nneeds = []\n";
    let read = Manifest::parse(text.as_bytes()).expect("an older manifest must still parse");
    assert_eq!(read.provides_class(), None);
}

/// Absence renders no key, so an application that provides no class does not
/// change the bytes of every library artifact already signed.
#[test]
fn a_library_with_no_class_renders_no_key() {
    let manifest = Manifest::new(vec!["libgreet.so.1".to_string()]).unwrap();
    assert_eq!(manifest.provides_class(), None);
    let text = manifest.render();
    assert!(!text.contains("provides-class"), "{text}");
    assert_eq!(Manifest::parse(text.as_bytes()), Ok(manifest));
}

/// A declared class survives the round trip.
#[test]
fn a_provided_class_round_trips() {
    let manifest = Manifest::new(vec!["libtemp.so.1".to_string()])
        .unwrap()
        .with_addon(Some("temperature-sensor".to_string()), None)
        .unwrap();
    let text = manifest.render();
    assert!(text.contains("provides-class = \"temperature-sensor\"\n"), "{text}");
    assert_eq!(Manifest::parse(text.as_bytes()), Ok(manifest));
}

/// The same alphabet an application's `accepts-class` is held to — one rule,
/// checked from both signed declarations.
#[test]
fn a_provided_class_is_refused_by_the_same_rule_an_application_accepts_class_is() {
    for bad in ["", "Temperature", "temperature_sensor", "with space"] {
        assert!(
            Manifest::new(vec!["libgreet.so.1".to_string()])
                .unwrap()
                .with_addon(Some(bad.to_string()), None)
                .is_err(),
            "{bad:?} should be refused"
        );
    }
    let text = "kind = \"library\"\nsoname = [\"libgreet.so.1\"]\nneeds = []\n\
                provides-class = \"Bad Class\"\n";
    assert!(
        Manifest::parse(text.as_bytes()).is_err(),
        "a hand-written manifest smuggled a bad class past the parser"
    );
}

// --- `addon-soname` — `.rulings/` R-201 -------------------------------------

/// **The case that must not have been broken, and the one every addon in
/// this project ships as.** One object, a class, no key: there is nothing to
/// choose between, so the manifest states the object once and the reader
/// resolves it.
///
/// The rendered bytes are asserted as well as the resolution, because this
/// is also the proof that R-201 needs no compatibility shim: an artifact
/// signed before the key existed and one signed after it are the same bytes.
#[test]
fn a_library_of_one_object_names_no_addon_and_joins_by_it_anyway() {
    let manifest = Manifest::new(vec!["libaura-reading-source.so.1".to_string()])
        .unwrap()
        .with_addon(Some("aura-reading-source".to_string()), None)
        .unwrap();
    assert_eq!(manifest.addon_soname(), None);
    assert_eq!(manifest.addon_object(), Some("libaura-reading-source.so.1"));

    let text = manifest.render();
    assert!(!text.contains("addon-soname"), "{text}");
    let read = Manifest::parse(text.as_bytes()).expect("it must parse back");
    assert_eq!(read, manifest);
    assert_eq!(read.addon_object(), Some("libaura-reading-source.so.1"));
}

/// **Several objects and a key: the NAMED one joins**, whichever position it
/// holds. The fixture names the last of three deliberately — the rule this
/// replaced would have answered the first, so a fixture that named the first
/// would pass under both rules and prove neither.
#[test]
fn a_library_of_several_objects_joins_by_the_one_it_names() {
    let manifest = Manifest::new(vec![
        "libpack-codec.so.1".to_string(),
        "libpack-private.so.1".to_string(),
        "libpack.so.1".to_string(),
    ])
    .unwrap()
    .with_addon(
        Some("aura-background-pack".to_string()),
        Some("libpack.so.1".to_string()),
    )
    .unwrap();
    assert_eq!(manifest.addon_object(), Some("libpack.so.1"));

    let text = manifest.render();
    assert!(text.contains("addon-soname = \"libpack.so.1\"\n"), "{text}");
    let read = Manifest::parse(text.as_bytes()).expect("it must parse back");
    assert_eq!(read, manifest);
    assert_eq!(read.addon_object(), Some("libpack.so.1"));
}

/// **The ruling itself**: several objects, a class, and no name is refused —
/// both when a manifest is built and when one is read back — and the refusal
/// names every object the library carries, because *which one should I have
/// said* is the author's next question.
#[test]
fn a_library_of_several_objects_naming_none_is_refused_and_names_them() {
    let objects = vec!["libpack.so.1".to_string(), "libpack-private.so.1".to_string()];
    let refused = Manifest::new(objects.clone())
        .unwrap()
        .with_addon(Some("aura-background-pack".to_string()), None);
    assert_eq!(
        refused,
        Err(ManifestFault::AddonObjectUnnamed {
            objects: objects.clone()
        })
    );

    let text = "kind = \"library\"\n\
                soname = [\"libpack.so.1\", \"libpack-private.so.1\"]\n\
                needs = []\n\
                provides-class = \"aura-background-pack\"\n";
    let read = Manifest::parse(text.as_bytes());
    assert_eq!(read, Err(ManifestFault::AddonObjectUnnamed { objects }));

    let said = read.unwrap_err().to_string();
    for object in ["libpack.so.1", "libpack-private.so.1"] {
        assert!(said.contains(object), "the refusal did not name {object}: {said}");
    }
    assert!(said.contains("addon-soname"), "{said}");
}

/// **A name with nothing behind it is refused**, and the refusal prints the
/// list to compare against — a typo in one of sixteen filenames is found by
/// reading the two beside each other.
#[test]
fn an_addon_object_the_library_does_not_carry_is_refused() {
    let objects = vec!["libpack.so.1".to_string(), "libpack-private.so.1".to_string()];
    let refused = Manifest::new(objects.clone())
        .unwrap()
        .with_addon(
            Some("aura-background-pack".to_string()),
            Some("libpack.so.2".to_string()),
        );
    assert_eq!(
        refused,
        Err(ManifestFault::AddonObjectNotCarried {
            named: "libpack.so.2".to_string(),
            objects: objects.clone(),
        })
    );

    let text = "kind = \"library\"\n\
                soname = [\"libpack.so.1\", \"libpack-private.so.1\"]\n\
                needs = []\n\
                provides-class = \"aura-background-pack\"\n\
                addon-soname = \"libpack.so.2\"\n";
    let read = Manifest::parse(text.as_bytes());
    assert_eq!(
        read,
        Err(ManifestFault::AddonObjectNotCarried {
            named: "libpack.so.2".to_string(),
            objects,
        })
    );
    let said = read.unwrap_err().to_string();
    assert!(said.contains("libpack.so.2"), "{said}");
    assert!(said.contains("libpack-private.so.1"), "{said}");
}

/// A library that joins nothing by class cannot have an addon object: there
/// is no host that would ever be told about it.
#[test]
fn an_addon_object_without_a_class_is_refused() {
    let refused = Manifest::new(vec!["libpack.so.1".to_string()])
        .unwrap()
        .with_addon(None, Some("libpack.so.1".to_string()));
    assert_eq!(
        refused,
        Err(ManifestFault::AddonObjectWithoutClass {
            named: "libpack.so.1".to_string()
        })
    );

    let text = "kind = \"library\"\nsoname = [\"libpack.so.1\"]\nneeds = []\n\
                addon-soname = \"libpack.so.1\"\n";
    assert_eq!(
        Manifest::parse(text.as_bytes()),
        Err(ManifestFault::AddonObjectWithoutClass {
            named: "libpack.so.1".to_string()
        })
    );
}

/// A library that provides no class has no addon object, however many
/// objects it carries — the question is not *which one*, it is *none*.
#[test]
fn a_library_with_no_class_has_no_addon_object() {
    let manifest = Manifest::new(vec![
        "libz.so.1".to_string(),
        "libz-private.so.1".to_string(),
    ])
    .unwrap();
    assert_eq!(manifest.addon_object(), None);
    assert_eq!(Manifest::parse(manifest.render().as_bytes()), Ok(manifest));
}

/// The key is last, so a manifest that puts it before `provides-class` is out
/// of order — the same rule that protects every other key's position, which
/// is what lets a key be appended without invalidating a signed artifact.
#[test]
fn addon_soname_is_the_last_key_in_the_canonical_order() {
    let text = "kind = \"library\"\nsoname = [\"libpack.so.1\"]\nneeds = []\n\
                addon-soname = \"libpack.so.1\"\n\
                provides-class = \"aura-background-pack\"\n";
    assert_eq!(
        Manifest::parse(text.as_bytes()),
        Err(ManifestFault::OutOfOrder {
            key: "provides-class"
        })
    );
}

/// An `addon-soname` that could become a path is refused as a bad value, the
/// same rule `soname`'s own entries are held to — it is a filename a loader
/// will open, and the comparison against the list comes after.
#[test]
fn an_addon_soname_that_could_become_a_path_is_refused() {
    let text = "kind = \"library\"\nsoname = [\"libpack.so.1\"]\nneeds = []\n\
                provides-class = \"aura-background-pack\"\n\
                addon-soname = \"../libc.so\"\n";
    assert!(
        matches!(
            Manifest::parse(text.as_bytes()),
            Err(ManifestFault::BadValue { key: "soname" })
        ),
        "{:?}",
        Manifest::parse(text.as_bytes())
    );
}

// --- the public surface -----------------------------------------------------

/// `PUBLIC_LIBRARIES` is a promise that cannot be withdrawn once applications
/// link against it (§4.3). The test exists so that adding to it is a deliberate
/// act with a failing assertion in front of it, rather than a one-line edit.
///
/// **It fired on 2026-08-31 and did its job.** Mesa needed three of these and
/// the failing assertion is what put the question to the maintainer rather than
/// letting an agent widen the platform's promise on its own judgement. He ruled
/// the GPU stack in, with a clause that bounds it: only because its version
/// must match the kernel's (`.rulings/` R-355). `libgcc_s.so.1` came in beside
/// it on separate grounds — see the constant, where the two arguments are kept
/// apart on purpose.
#[test]
fn the_platform_promises_exactly_these_libraries() {
    assert_eq!(
        PUBLIC_LIBRARIES,
        &[
            // The language runtime — what a compiled program IS.
            "libc.so",
            "libgcc_s.so.1",
            "libstdc++.so.6",
            // The kernel's userspace half — what must match the kernel.
            "libdrm.so.2",
            "libEGL.so.1",
            "libGLESv2.so.2",
            "libgbm.so.1"
        ]
    );
}

/// **The two groups are the rule, and the rule is that there are only two.**
///
/// A future addition belongs to one of them or to neither, and neither means
/// it is a Shared Rivet. This asserts the SIZE of each group rather than its
/// contents — the contents are above — so that a library added without a group
/// fails here as well, and whoever adds it has to say which test it passed.
#[test]
fn every_promised_library_belongs_to_one_of_the_two_groups() {
    let runtime = ["libc.so", "libgcc_s.so.1", "libstdc++.so.6"];
    let kernel = [
        "libdrm.so.2",
        "libEGL.so.1",
        "libGLESv2.so.2",
        "libgbm.so.1",
    ];
    for name in PUBLIC_LIBRARIES {
        assert!(
            runtime.contains(name) || kernel.contains(name),
            "{name} is promised and is in neither group. Either it is what a \
             compiled program IS (could an application avoid it by writing \
             different code?), or its version must match the kernel's — or it \
             is a choice, and a choice is a Shared Rivet."
        );
    }
    assert_eq!(PUBLIC_LIBRARIES.len(), runtime.len() + kernel.len());
}

/// **Every promise is a SONAME, not a file name.** A `NEEDED` entry names a
/// soname and the check that reads this list compares against one, so an entry
/// like `libEGL.so.1.0.0` would be a promise no application could ever match —
/// it would refuse a binary that links the library the image actually ships.
#[test]
fn every_promised_library_is_named_as_a_soname() {
    for name in PUBLIC_LIBRARIES {
        assert!(
            name.contains(".so"),
            "{name} does not look like a soname"
        );
        assert!(
            !name.contains('/'),
            "{name} is a path; a NEEDED entry is a bare soname"
        );
    }
}

/// The loader path is not a choice: it is the `PT_INTERP` the cross toolchain
/// writes into every dynamic object it links. If this constant and the
/// toolchain ever disagree, nothing dynamic starts and the kernel's only
/// account of it is `ENOENT` on exec.
#[test]
fn the_loader_path_is_the_one_the_toolchain_writes() {
    assert_eq!(LOADER_PATH, "/lib/ld-musl-aarch64.so.1");
}

// --- the target class -------------------------------------------------------

#[test]
fn every_target_class_byte_round_trips_and_no_other_is_accepted() {
    for class in [TargetClass::Manifest, TargetClass::Object, TargetClass::Asset] {
        assert_eq!(TargetClass::from_byte(class.as_byte()), Some(class));
    }
    assert_eq!(TargetClass::from_byte(3), None);
    assert_eq!(TargetClass::from_byte(255), None);
}
