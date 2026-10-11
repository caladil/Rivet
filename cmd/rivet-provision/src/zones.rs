//! **The zone names this tool will accept**, and where they are checked.
//!
//! # Why a name is checked at all, when the grammar already passed
//!
//! `rivet_core::timezone::Timezone` refuses a path, a POSIX `TZ` string and
//! anything that is not shaped like an identifier — and it says plainly that it
//! *cannot* check whether the zone exists, because a document is written for
//! machines that have not been built yet.
//!
//! That leaves exactly one gap, and it is the expensive one:
//! `America/Chigago` is a well-formed name for a file no image carries. It
//! parses. It is projected. Nothing fails. musl maps no tzfile, falls back to
//! `__utc` **silently**, and the machine keeps UTC with a document on it naming
//! a zone (`rivet_core::timezone`, and `ADR-021` §9). A typo here is therefore
//! not a typo — it is a wrong clock nobody looks at, in every timestamp
//! including the ones somebody would use to diagnose it.
//!
//! **So the name is checked against a list of zones that exist**, at the one
//! moment a person is looking at what they typed.
//!
//! # Two authorities, and the real one wins whenever it is present
//!
//! `ADR-011` §3 is the reason this module is shaped the way it is rather than
//! being a constant. The authority on which zones exist is **the database the
//! image ships**, at `rivet_core::etc::ZONEINFO_DIR`
//! (`/system/share/zoneinfo`), put there by `rivet-image --zoneinfo` out of the
//! build container's `/usr/share/zoneinfo`.
//!
//! * **If a database is readable, that is what is checked** — [`database`]
//!   finds one, and the file is read and held against `TZif` exactly as musl
//!   would (`rivet_core::timezone::is_tzfile`), so what this tool accepts is
//!   what that machine will actually map. This is the case inside the build
//!   container, which is where cards are built.
//! * **Otherwise [`KNOWN`] answers**, which is a *copy* and is labelled as one.
//!   A Windows workstation has no zone database at all, and the alternative to
//!   a copy there is no check — which is the defect.
//!
//! [`source`] says which of the two answered, so a person is never left
//! guessing what their name was held against.
//!
//! # What a stale copy costs, in each direction
//!
//! **A zone missing from [`KNOWN`] is refused, visibly**, with the near
//! misses named and `RIVET_ZONEINFO` offered as the way past it. A zone
//! wrongly *in* it would be accepted and produce a UTC clock. The list is
//! therefore generated from a real database rather than typed, and the
//! generation is recorded above the list itself.

use std::path::PathBuf;

/// The environment variable that names a zone database to check against.
///
/// **The way past a stale [`KNOWN`].** The tz database gains zones; this build
/// does not. A person whose zone is real and whose copy of this tool predates
/// it points this at any `zoneinfo` directory — a container's, a Linux box's,
/// an unpacked image's — and the check is made against files rather than
/// against a list.
pub const OVERRIDE: &str = "RIVET_ZONEINFO";

/// The zone database roots this tool will read, in the order it tries them.
///
/// The board's path is first because it is the one that is *authoritative for a
/// Rivet*: a workstation that happens to be a Rivet, or a mounted image, should
/// be checked against what that card will carry rather than against the host
/// distribution's database, which may be a different vintage.
const ROOTS: &[&str] = &[rivet_core::etc::ZONEINFO_DIR, "/usr/share/zoneinfo"];

/// The zones a person is offered by number, before typing anything.
///
/// **Short on purpose.** [`KNOWN`] has six hundred names and a menu that
/// printed them would be a menu nobody reads. These are the ones a person
/// provisioning a board in this project is most likely to want, and every other
/// zone is still typed in full — the numbered list is a convenience, never the
/// set of legal answers.
///
/// `UTC` is first and it is not filler: it is the honest answer for a machine
/// whose operator does not want a local zone, and choosing it is a *recorded*
/// answer rather than the silence that produces the same clock by accident.
pub const OFFERED: &[&str] = &[
    "UTC",
    "America/Chicago",
    "America/New_York",
    "America/Denver",
    "America/Los_Angeles",
    "America/Phoenix",
    "America/Anchorage",
    "Pacific/Honolulu",
    "Europe/London",
    "Europe/Berlin",
    "Australia/Sydney",
];

/// A readable zone database, if this machine has one.
///
/// [`OVERRIDE`] wins over both [`ROOTS`], because a person who named a
/// directory has said something more specific than a default.
#[must_use]
pub fn database() -> Option<PathBuf> {
    if let Some(named) = std::env::var_os(OVERRIDE) {
        let path = PathBuf::from(named);
        if path.is_dir() {
            return Some(path);
        }
    }
    ROOTS
        .iter()
        .map(PathBuf::from)
        .find(|p| p.is_dir())
}

/// What the names are being held against, in words for a screen.
#[must_use]
pub fn source() -> String {
    database().map_or_else(
        || format!("the {} zone names this build carries", KNOWN.len()),
        |dir| format!("the zone database at {}", dir.display()),
    )
}

/// **Does a zone by this name exist?**
///
/// Against a real database this reads the file and applies musl's own test, so
/// a name that passes here is a name that machine can actually map. Against
/// [`KNOWN`] it is a membership test on a sorted list.
///
/// The name is assumed to have passed `Timezone::new` already — [`check`] is
/// the order-enforcing caller — so it cannot escape the database root.
#[must_use]
pub fn known(name: &str) -> bool {
    if let Some(dir) = database() {
        let Some(zone) = rivet_core::timezone::Timezone::new(name) else {
            return false;
        };
        let path = zone.tzfile_under(&dir.to_string_lossy());
        return match std::fs::read(&path) {
            Ok(bytes) => rivet_core::timezone::is_tzfile(&bytes),
            Err(_) => false,
        };
    }
    KNOWN.binary_search(&name).is_ok()
}

/// Zone names that look like what somebody typed, for a refusal to name.
///
/// **Two different mistakes, so two different tests.** A person types either a
/// fragment they remember — `chicago` — or a name they got slightly wrong —
/// `America/Chigago`. A substring match finds the first and is useless on the
/// second, where the wrong letter is in the middle of the word; an edit
/// distance finds the second and would drag in half the database for the
/// first. Both run, substring matches first, and the whole thing is capped at
/// five: a refusal that prints forty suggestions is a refusal nobody reads.
///
/// **The last component is what is compared**, because that is the part a
/// person gets wrong. `America/Chigago` and `America/Chicago` differ by one
/// letter in the city and not at all in the continent.
#[must_use]
pub fn like(text: &str) -> Vec<&'static str> {
    let typed = text.trim().to_ascii_lowercase();
    let Some(needle) = typed.rsplit('/').next() else {
        return Vec::new();
    };
    // Two characters would match hundreds of zones and suggest nothing.
    if needle.len() < 3 {
        return Vec::new();
    }
    // A distance of one on a short word is a different word; two on a long one
    // is still recognisably the same. Scaled rather than fixed for that reason.
    let tolerance = if needle.len() >= 6 { 2 } else { 1 };

    let mut out: Vec<&'static str> = Vec::new();
    for pass in 0..2 {
        for zone in KNOWN {
            if out.len() == 5 {
                return out;
            }
            if out.contains(zone) {
                continue;
            }
            let lower = zone.to_ascii_lowercase();
            let Some(tail) = lower.rsplit('/').next() else { continue };
            let close = if pass == 0 {
                lower.contains(needle)
            } else {
                within(tail, needle, tolerance)
            };
            if close {
                out.push(zone);
            }
        }
    }
    out
}

/// Whether `a` and `b` are within `limit` single-character edits of each other.
///
/// A bounded Levenshtein over one row. It is here rather than pulled in from a
/// crate for `ADR-009` §5's reason, which binds this binary harder than most:
/// it mints and holds `root.key`, so a dependency added to spell-check a city
/// name is a dependency with the opportunity to read the fleet's whole trust.
fn within(a: &str, b: &str, limit: usize) -> bool {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    if a.len().abs_diff(b.len()) > limit {
        return false;
    }
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ac) in a.iter().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        let mut best = row[0];
        for (j, bc) in b.iter().enumerate() {
            let cost = usize::from(ac != bc);
            let next = (row[j + 1] + 1).min(row[j] + 1).min(diagonal + cost);
            diagonal = row[j + 1];
            row[j + 1] = next;
            best = best.min(next);
        }
        // Every remaining edit can only add to the best figure in this row, so
        // a row whose cheapest cell already exceeds the limit cannot recover.
        if best > limit {
            return false;
        }
    }
    row.last().is_some_and(|d| *d <= limit)
}

/// **The check the menu runs on a typed zone.**
///
/// Three refusals in order, each with its own reason, because they are three
/// different mistakes:
///
/// 1. **Nothing typed.** Silence is what produces the UTC clock this key
///    exists to prevent, so an empty answer is not accepted as *"UTC then"* —
///    `UTC` is typed, and thereby recorded.
/// 2. **Not a zone identifier**, which is `Timezone::RULE`'s job — a path, a
///    POSIX `TZ` string, a trailing space.
/// 3. **Not a zone that exists**, which nothing else in this workspace can
///    say. This is the one that matters.
///
/// # Errors
///
/// A short reason, for the menu's error line. The long form belongs in the
/// field's `explain` text and in what the edit prompt prints.
pub fn check(text: &str) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("required — an unanswered zone is a machine that keeps UTC silently".to_owned());
    }
    if rivet_core::timezone::Timezone::new(text).is_none() {
        return Err("an IANA name like America/Chicago, not a path or a TZ string".to_owned());
    }
    if !known(text) {
        let near = like(text);
        return Err(if near.is_empty() {
            format!("not a zone in {}", source())
        } else {
            format!("not a zone; did you mean {}", near.join(", "))
        });
    }
    Ok(())
}

/// **Every zone the card-build container's database carries**, sorted.
///
/// # Where this came from, so it can be made again
///
/// Generated 2026-09-07 from `rivet-card-build:6` — the container
/// `tools/container/build-card.sh` builds cards in, and therefore the database
/// `rivet-image --zoneinfo` ships onto one:
///
/// ```text
/// docker run --rm rivet-card-build:6 sh -c "cd /usr/share/zoneinfo && \
///   find . -type f ! -path './posix/*' ! -path './right/*' | \
///   while read -r f; do \
///     if [ \"$(head -c4 \"$f\")\" = TZif ]; then echo \"${f#./}\"; fi; \
///   done | LC_ALL=C sort"
/// ```
///
/// **Every file was read and checked for `TZif`**, not trusted by its name —
/// the same rule `cmd/rivet-image`'s `zone_database` applies, and for the same
/// reason: `zone.tab`, `iso3166.tab`, `tzdata.zi` and `leapseconds` sit beside
/// the zones and none of them is one. `posix/` and `right/` are excluded
/// because the image does not ship them.
///
/// **Links are here as well as canonical names** — `US/Central`,
/// `Asia/Calcutta`, `Etc/GMT+5`. They are real files on the card, so refusing
/// one would be refusing a name that works.
///
/// **Sorted, and [`known`] binary-searches it.** The order is `LC_ALL=C`, which
/// is byte order, which is what `binary_search` uses.
pub const KNOWN: &[&str] = &[
    "Africa/Abidjan", "Africa/Accra", "Africa/Addis_Ababa", "Africa/Algiers",
    "Africa/Asmara", "Africa/Asmera", "Africa/Bamako", "Africa/Bangui",
    "Africa/Banjul", "Africa/Bissau", "Africa/Blantyre", "Africa/Brazzaville",
    "Africa/Bujumbura", "Africa/Cairo", "Africa/Casablanca", "Africa/Ceuta",
    "Africa/Conakry", "Africa/Dakar", "Africa/Dar_es_Salaam", "Africa/Djibouti",
    "Africa/Douala", "Africa/El_Aaiun", "Africa/Freetown", "Africa/Gaborone",
    "Africa/Harare", "Africa/Johannesburg", "Africa/Juba", "Africa/Kampala",
    "Africa/Khartoum", "Africa/Kigali", "Africa/Kinshasa", "Africa/Lagos",
    "Africa/Libreville", "Africa/Lome", "Africa/Luanda", "Africa/Lubumbashi",
    "Africa/Lusaka", "Africa/Malabo", "Africa/Maputo", "Africa/Maseru",
    "Africa/Mbabane", "Africa/Mogadishu", "Africa/Monrovia", "Africa/Nairobi",
    "Africa/Ndjamena", "Africa/Niamey", "Africa/Nouakchott", "Africa/Ouagadougou",
    "Africa/Porto-Novo", "Africa/Sao_Tome", "Africa/Timbuktu", "Africa/Tripoli",
    "Africa/Tunis", "Africa/Windhoek", "America/Adak", "America/Anchorage",
    "America/Anguilla", "America/Antigua", "America/Araguaina", "America/Argentina/Buenos_Aires",
    "America/Argentina/Catamarca", "America/Argentina/ComodRivadavia", "America/Argentina/Cordoba", "America/Argentina/Jujuy",
    "America/Argentina/La_Rioja", "America/Argentina/Mendoza", "America/Argentina/Rio_Gallegos", "America/Argentina/Salta",
    "America/Argentina/San_Juan", "America/Argentina/San_Luis", "America/Argentina/Tucuman", "America/Argentina/Ushuaia",
    "America/Aruba", "America/Asuncion", "America/Atikokan", "America/Atka",
    "America/Bahia", "America/Bahia_Banderas", "America/Barbados", "America/Belem",
    "America/Belize", "America/Blanc-Sablon", "America/Boa_Vista", "America/Bogota",
    "America/Boise", "America/Buenos_Aires", "America/Cambridge_Bay", "America/Campo_Grande",
    "America/Cancun", "America/Caracas", "America/Catamarca", "America/Cayenne",
    "America/Cayman", "America/Chicago", "America/Chihuahua", "America/Ciudad_Juarez",
    "America/Coral_Harbour", "America/Cordoba", "America/Costa_Rica", "America/Coyhaique",
    "America/Creston", "America/Cuiaba", "America/Curacao", "America/Danmarkshavn",
    "America/Dawson", "America/Dawson_Creek", "America/Denver", "America/Detroit",
    "America/Dominica", "America/Edmonton", "America/Eirunepe", "America/El_Salvador",
    "America/Ensenada", "America/Fort_Nelson", "America/Fort_Wayne", "America/Fortaleza",
    "America/Glace_Bay", "America/Godthab", "America/Goose_Bay", "America/Grand_Turk",
    "America/Grenada", "America/Guadeloupe", "America/Guatemala", "America/Guayaquil",
    "America/Guyana", "America/Halifax", "America/Havana", "America/Hermosillo",
    "America/Indiana/Indianapolis", "America/Indiana/Knox", "America/Indiana/Marengo", "America/Indiana/Petersburg",
    "America/Indiana/Tell_City", "America/Indiana/Vevay", "America/Indiana/Vincennes", "America/Indiana/Winamac",
    "America/Indianapolis", "America/Inuvik", "America/Iqaluit", "America/Jamaica",
    "America/Jujuy", "America/Juneau", "America/Kentucky/Louisville", "America/Kentucky/Monticello",
    "America/Knox_IN", "America/Kralendijk", "America/La_Paz", "America/Lima",
    "America/Los_Angeles", "America/Louisville", "America/Lower_Princes", "America/Maceio",
    "America/Managua", "America/Manaus", "America/Marigot", "America/Martinique",
    "America/Matamoros", "America/Mazatlan", "America/Mendoza", "America/Menominee",
    "America/Merida", "America/Metlakatla", "America/Mexico_City", "America/Miquelon",
    "America/Moncton", "America/Monterrey", "America/Montevideo", "America/Montreal",
    "America/Montserrat", "America/Nassau", "America/New_York", "America/Nipigon",
    "America/Nome", "America/Noronha", "America/North_Dakota/Beulah", "America/North_Dakota/Center",
    "America/North_Dakota/New_Salem", "America/Nuuk", "America/Ojinaga", "America/Panama",
    "America/Pangnirtung", "America/Paramaribo", "America/Phoenix", "America/Port-au-Prince",
    "America/Port_of_Spain", "America/Porto_Acre", "America/Porto_Velho", "America/Puerto_Rico",
    "America/Punta_Arenas", "America/Rainy_River", "America/Rankin_Inlet", "America/Recife",
    "America/Regina", "America/Resolute", "America/Rio_Branco", "America/Rosario",
    "America/Santa_Isabel", "America/Santarem", "America/Santiago", "America/Santo_Domingo",
    "America/Sao_Paulo", "America/Scoresbysund", "America/Shiprock", "America/Sitka",
    "America/St_Barthelemy", "America/St_Johns", "America/St_Kitts", "America/St_Lucia",
    "America/St_Thomas", "America/St_Vincent", "America/Swift_Current", "America/Tegucigalpa",
    "America/Thule", "America/Thunder_Bay", "America/Tijuana", "America/Toronto",
    "America/Tortola", "America/Vancouver", "America/Virgin", "America/Whitehorse",
    "America/Winnipeg", "America/Yakutat", "America/Yellowknife", "Antarctica/Casey",
    "Antarctica/Davis", "Antarctica/DumontDUrville", "Antarctica/Macquarie", "Antarctica/Mawson",
    "Antarctica/McMurdo", "Antarctica/Palmer", "Antarctica/Rothera", "Antarctica/South_Pole",
    "Antarctica/Syowa", "Antarctica/Troll", "Antarctica/Vostok", "Arctic/Longyearbyen",
    "Asia/Aden", "Asia/Almaty", "Asia/Amman", "Asia/Anadyr",
    "Asia/Aqtau", "Asia/Aqtobe", "Asia/Ashgabat", "Asia/Ashkhabad",
    "Asia/Atyrau", "Asia/Baghdad", "Asia/Bahrain", "Asia/Baku",
    "Asia/Bangkok", "Asia/Barnaul", "Asia/Beirut", "Asia/Bishkek",
    "Asia/Brunei", "Asia/Calcutta", "Asia/Chita", "Asia/Choibalsan",
    "Asia/Chongqing", "Asia/Chungking", "Asia/Colombo", "Asia/Dacca",
    "Asia/Damascus", "Asia/Dhaka", "Asia/Dili", "Asia/Dubai",
    "Asia/Dushanbe", "Asia/Famagusta", "Asia/Gaza", "Asia/Harbin",
    "Asia/Hebron", "Asia/Ho_Chi_Minh", "Asia/Hong_Kong", "Asia/Hovd",
    "Asia/Irkutsk", "Asia/Istanbul", "Asia/Jakarta", "Asia/Jayapura",
    "Asia/Jerusalem", "Asia/Kabul", "Asia/Kamchatka", "Asia/Karachi",
    "Asia/Kashgar", "Asia/Kathmandu", "Asia/Katmandu", "Asia/Khandyga",
    "Asia/Kolkata", "Asia/Krasnoyarsk", "Asia/Kuala_Lumpur", "Asia/Kuching",
    "Asia/Kuwait", "Asia/Macao", "Asia/Macau", "Asia/Magadan",
    "Asia/Makassar", "Asia/Manila", "Asia/Muscat", "Asia/Nicosia",
    "Asia/Novokuznetsk", "Asia/Novosibirsk", "Asia/Omsk", "Asia/Oral",
    "Asia/Phnom_Penh", "Asia/Pontianak", "Asia/Pyongyang", "Asia/Qatar",
    "Asia/Qostanay", "Asia/Qyzylorda", "Asia/Rangoon", "Asia/Riyadh",
    "Asia/Saigon", "Asia/Sakhalin", "Asia/Samarkand", "Asia/Seoul",
    "Asia/Shanghai", "Asia/Singapore", "Asia/Srednekolymsk", "Asia/Taipei",
    "Asia/Tashkent", "Asia/Tbilisi", "Asia/Tehran", "Asia/Tel_Aviv",
    "Asia/Thimbu", "Asia/Thimphu", "Asia/Tokyo", "Asia/Tomsk",
    "Asia/Ujung_Pandang", "Asia/Ulaanbaatar", "Asia/Ulan_Bator", "Asia/Urumqi",
    "Asia/Ust-Nera", "Asia/Vientiane", "Asia/Vladivostok", "Asia/Yakutsk",
    "Asia/Yangon", "Asia/Yekaterinburg", "Asia/Yerevan", "Atlantic/Azores",
    "Atlantic/Bermuda", "Atlantic/Canary", "Atlantic/Cape_Verde", "Atlantic/Faeroe",
    "Atlantic/Faroe", "Atlantic/Jan_Mayen", "Atlantic/Madeira", "Atlantic/Reykjavik",
    "Atlantic/South_Georgia", "Atlantic/St_Helena", "Atlantic/Stanley", "Australia/ACT",
    "Australia/Adelaide", "Australia/Brisbane", "Australia/Broken_Hill", "Australia/Canberra",
    "Australia/Currie", "Australia/Darwin", "Australia/Eucla", "Australia/Hobart",
    "Australia/LHI", "Australia/Lindeman", "Australia/Lord_Howe", "Australia/Melbourne",
    "Australia/NSW", "Australia/North", "Australia/Perth", "Australia/Queensland",
    "Australia/South", "Australia/Sydney", "Australia/Tasmania", "Australia/Victoria",
    "Australia/West", "Australia/Yancowinna", "Brazil/Acre", "Brazil/DeNoronha",
    "Brazil/East", "Brazil/West", "CET", "CST6CDT",
    "Canada/Atlantic", "Canada/Central", "Canada/Eastern", "Canada/Mountain",
    "Canada/Newfoundland", "Canada/Pacific", "Canada/Saskatchewan", "Canada/Yukon",
    "Chile/Continental", "Chile/EasterIsland", "Cuba", "EET",
    "EST", "EST5EDT", "Egypt", "Eire",
    "Etc/GMT", "Etc/GMT+0", "Etc/GMT+1", "Etc/GMT+10",
    "Etc/GMT+11", "Etc/GMT+12", "Etc/GMT+2", "Etc/GMT+3",
    "Etc/GMT+4", "Etc/GMT+5", "Etc/GMT+6", "Etc/GMT+7",
    "Etc/GMT+8", "Etc/GMT+9", "Etc/GMT-0", "Etc/GMT-1",
    "Etc/GMT-10", "Etc/GMT-11", "Etc/GMT-12", "Etc/GMT-13",
    "Etc/GMT-14", "Etc/GMT-2", "Etc/GMT-3", "Etc/GMT-4",
    "Etc/GMT-5", "Etc/GMT-6", "Etc/GMT-7", "Etc/GMT-8",
    "Etc/GMT-9", "Etc/GMT0", "Etc/Greenwich", "Etc/UCT",
    "Etc/UTC", "Etc/Universal", "Etc/Zulu", "Europe/Amsterdam",
    "Europe/Andorra", "Europe/Astrakhan", "Europe/Athens", "Europe/Belfast",
    "Europe/Belgrade", "Europe/Berlin", "Europe/Bratislava", "Europe/Brussels",
    "Europe/Bucharest", "Europe/Budapest", "Europe/Busingen", "Europe/Chisinau",
    "Europe/Copenhagen", "Europe/Dublin", "Europe/Gibraltar", "Europe/Guernsey",
    "Europe/Helsinki", "Europe/Isle_of_Man", "Europe/Istanbul", "Europe/Jersey",
    "Europe/Kaliningrad", "Europe/Kiev", "Europe/Kirov", "Europe/Kyiv",
    "Europe/Lisbon", "Europe/Ljubljana", "Europe/London", "Europe/Luxembourg",
    "Europe/Madrid", "Europe/Malta", "Europe/Mariehamn", "Europe/Minsk",
    "Europe/Monaco", "Europe/Moscow", "Europe/Nicosia", "Europe/Oslo",
    "Europe/Paris", "Europe/Podgorica", "Europe/Prague", "Europe/Riga",
    "Europe/Rome", "Europe/Samara", "Europe/San_Marino", "Europe/Sarajevo",
    "Europe/Saratov", "Europe/Simferopol", "Europe/Skopje", "Europe/Sofia",
    "Europe/Stockholm", "Europe/Tallinn", "Europe/Tirane", "Europe/Tiraspol",
    "Europe/Ulyanovsk", "Europe/Uzhgorod", "Europe/Vaduz", "Europe/Vatican",
    "Europe/Vienna", "Europe/Vilnius", "Europe/Volgograd", "Europe/Warsaw",
    "Europe/Zagreb", "Europe/Zaporozhye", "Europe/Zurich", "Factory",
    "GB", "GB-Eire", "GMT", "GMT+0",
    "GMT-0", "GMT0", "Greenwich", "HST",
    "Hongkong", "Iceland", "Indian/Antananarivo", "Indian/Chagos",
    "Indian/Christmas", "Indian/Cocos", "Indian/Comoro", "Indian/Kerguelen",
    "Indian/Mahe", "Indian/Maldives", "Indian/Mauritius", "Indian/Mayotte",
    "Indian/Reunion", "Iran", "Israel", "Jamaica",
    "Japan", "Kwajalein", "Libya", "MET",
    "MST", "MST7MDT", "Mexico/BajaNorte", "Mexico/BajaSur",
    "Mexico/General", "NZ", "NZ-CHAT", "Navajo",
    "PRC", "PST8PDT", "Pacific/Apia", "Pacific/Auckland",
    "Pacific/Bougainville", "Pacific/Chatham", "Pacific/Chuuk", "Pacific/Easter",
    "Pacific/Efate", "Pacific/Enderbury", "Pacific/Fakaofo", "Pacific/Fiji",
    "Pacific/Funafuti", "Pacific/Galapagos", "Pacific/Gambier", "Pacific/Guadalcanal",
    "Pacific/Guam", "Pacific/Honolulu", "Pacific/Johnston", "Pacific/Kanton",
    "Pacific/Kiritimati", "Pacific/Kosrae", "Pacific/Kwajalein", "Pacific/Majuro",
    "Pacific/Marquesas", "Pacific/Midway", "Pacific/Nauru", "Pacific/Niue",
    "Pacific/Norfolk", "Pacific/Noumea", "Pacific/Pago_Pago", "Pacific/Palau",
    "Pacific/Pitcairn", "Pacific/Pohnpei", "Pacific/Ponape", "Pacific/Port_Moresby",
    "Pacific/Rarotonga", "Pacific/Saipan", "Pacific/Samoa", "Pacific/Tahiti",
    "Pacific/Tarawa", "Pacific/Tongatapu", "Pacific/Truk", "Pacific/Wake",
    "Pacific/Wallis", "Pacific/Yap", "Poland", "Portugal",
    "ROC", "ROK", "Singapore", "Turkey",
    "UCT", "US/Alaska", "US/Aleutian", "US/Arizona",
    "US/Central", "US/East-Indiana", "US/Eastern", "US/Hawaii",
    "US/Indiana-Starke", "US/Michigan", "US/Mountain", "US/Pacific",
    "US/Samoa", "UTC", "Universal", "W-SU",
    "WET", "Zulu", "posixrules",
];

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// **The list is sorted, or [`known`] silently answers `false` for zones
    /// that are in it.** A binary search over an unsorted slice does not fail
    /// loudly; it fails by saying no, which here means refusing a real zone.
    #[test]
    fn the_list_is_in_the_order_the_search_assumes() {
        let mut sorted = KNOWN.to_vec();
        sorted.sort_unstable();
        assert_eq!(KNOWN, sorted.as_slice(), "KNOWN is not in byte order");
        let mut unique = sorted.clone();
        unique.dedup();
        assert_eq!(unique.len(), KNOWN.len(), "KNOWN carries a duplicate");
    }

    /// **Every name in the list is one the schema would accept.** A name this
    /// tool offers and `rivet_config` refuses would be the two-authorities bug
    /// in the cheapest place to catch it.
    #[test]
    fn every_known_name_is_a_name_the_schema_takes() {
        for name in KNOWN {
            assert!(
                rivet_core::timezone::Timezone::new(name).is_some(),
                "{name} is in KNOWN and is not a Timezone"
            );
        }
    }

    /// The short list a person is offered must be names the long list has, or
    /// pressing a number produces a refusal.
    #[test]
    fn every_offered_zone_is_a_zone_this_build_knows() {
        for name in OFFERED {
            assert!(
                KNOWN.binary_search(name).is_ok(),
                "{name} is offered by number and is not in KNOWN"
            );
        }
        assert_eq!(OFFERED.first(), Some(&"UTC"), "UTC must be offerable, and named");
    }

    /// **The database this tree ships is present**, spot-checked at both ends:
    /// the zone the maintainer keeps and the one musl falls back to.
    #[test]
    fn the_zones_this_project_actually_uses_are_there() {
        for name in ["UTC", "America/Chicago", "Etc/UTC", "US/Central"] {
            assert!(KNOWN.binary_search(&name).is_ok(), "{name} is missing");
        }
        // The database is ~600 files; a list that had lost most of it would
        // still pass every test above.
        assert!(KNOWN.len() > 500, "KNOWN holds only {}", KNOWN.len());
    }

    /// **The three refusals, and each one's reason.** This is the whole point
    /// of the module: a well-formed name for a zone the image has no file for
    /// is accepted by every other check in this workspace.
    #[test]
    fn a_well_formed_name_for_a_zone_that_does_not_exist_is_refused() {
        // Only meaningful against the compiled list; a machine with a real
        // database is checked against that instead and the assertion still
        // holds, because no database carries `America/Chigago` either.
        let why = check("America/Chigago").expect_err("a typo must be refused");
        assert!(why.contains("America/Chicago"), "{why}");

        assert!(check("").is_err(), "silence must be refused");
        assert!(check("../../etc/passwd").is_err(), "a path must be refused");
        assert!(
            check("CST6CDT,M3.2.0/2,M11.1.0/2").is_err(),
            "a POSIX TZ string must be refused"
        );
        assert!(check("America/Chicago").is_ok());
        assert!(check("UTC").is_ok());
    }

    /// A refusal that names near misses is the difference between *"no"* and
    /// *"you meant this"*.
    #[test]
    fn a_fragment_finds_the_zone_somebody_half_remembered() {
        assert!(like("chicago").contains(&"America/Chicago"));
        assert!(like("Chigago").contains(&"America/Chicago"));
        // Two characters would match hundreds of zones and suggest nothing.
        assert!(like("ch").is_empty());
    }

    /// What answered is reported, because a person told *"not a zone"* is
    /// owed the name of whatever said so.
    #[test]
    fn the_source_of_the_answer_is_nameable() {
        let said = source();
        assert!(said.contains("zone"), "{said}");
    }
}
