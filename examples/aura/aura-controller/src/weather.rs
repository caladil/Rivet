//! **The National Weather Service, and the cache that survives a restart** —
//! `internal/api/weather/nws.go` and `cache.go`, ported.
//!
//! # Three requests, and two of them happen once
//!
//! `/points/<lat>,<lng>` gives a forecast URL and a list of observation
//! stations; the nearest station's `observations/latest` gives the reading.
//! The first two are resolved once per coordinate pair and kept, so a refresh
//! every twenty minutes is one request and not three.
//!
//! # The fallback that stops the panel going blank
//!
//! A station's newest observation regularly carries **no temperature** — KLVN
//! and stations like it publish hourly and drop the field for individual
//! cycles, so the freshest record is often the one useless record in an
//! otherwise healthy series. Rather than report a failure, this reaches into
//! the station's recent list and takes the newest record that has one.
//!
//! It matters most **after a restart**, which discards the last good reading
//! held in memory and leaves a single null standing for the whole station. That
//! is also what the cache below is for.

use serde::{Deserialize, Serialize};

use crate::wire;

/// Fifteen seconds, as in Go.
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// What every weather request calls itself, with or without a contact.
const PRODUCT: &str = "Aura/1.0";

/// **The `User-Agent` a weather request carries**: [`PRODUCT`], and the
/// operator's contact in parentheses when [`crate::config::Config::weather_contact`]
/// holds one.
///
/// The NWS API documentation asks for a `User-Agent` identifying the caller and
/// a way to reach its operator, and says a request without one may be refused.
/// The contact is the operator's to give, so it is configuration and never a
/// constant.
///
/// **A contact a header cannot carry is left out rather than sent.** Only
/// printable ASCII is a valid header value; anything else would fail every
/// request, so a stray control character in a hand-edited file costs the
/// contact and never the weather. The portal does not save such a value
/// ([`contact_is_sendable`]).
pub fn user_agent(contact: &str) -> String {
    let contact = contact.trim();
    if contact.is_empty() || !contact_is_sendable(contact) {
        return PRODUCT.to_string();
    }
    format!("{PRODUCT} ({contact})")
}

/// Whether a contact can travel in a `User-Agent` header: printable ASCII
/// only, which also rules out every control character.
pub fn contact_is_sendable(contact: &str) -> bool {
    contact.bytes().all(|b| (0x20..=0x7e).contains(&b))
}

/// How old a cached reading may be and still be drawn.
///
/// The station publishes hourly and Aura polls every twenty minutes, so a cache
/// inside half an hour is at most one missed cycle — close enough to now that
/// showing it is honest. Past that it is discarded rather than shown, **because
/// the panel gives no indication of a reading's age** and a stale temperature
/// is indistinguishable on screen from a current one.
pub const MAX_CACHE_AGE_SECONDS: i64 = 30 * 60;

/// How many observations back to look for a temperature.
///
/// A station reporting hourly covers most of a working day in eight records,
/// which is well past the point where an older reading stops describing now.
const RECENT_WINDOW: usize = 8;

/// A resolved station and forecast, reused across refreshes.
#[derive(Debug, Default)]
pub struct Client {
    /// What [`Client::resolved`] was resolved for. A change means resolving
    /// again — the maintainer moved, or fixed a typo in the zip code.
    at: Option<(f64, f64)>,
    resolved: Option<Resolved>,
}

/// The three URLs a refresh needs.
#[derive(Debug, Clone)]
struct Resolved {
    forecast: String,
    /// The station's base, for the recent list.
    station: String,
    /// `<station>/observations/latest`.
    latest: String,
}

impl Client {
    /// A client that has resolved nothing yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Current conditions for these coordinates, asked for with `contact` in
    /// the `User-Agent` ([`user_agent`]).
    ///
    /// The gridpoint and the nearest station are resolved on the first call and
    /// whenever the coordinates change; every other call is one request.
    ///
    /// # Errors
    ///
    /// A sentence naming which of the three steps failed. The **statement** is
    /// not one of them: it is best-effort and an empty one is not a failure,
    /// because a temperature with no forecast beside it is still worth drawing.
    pub fn fetch(&mut self, lat: f64, lng: f64, contact: &str) -> Result<wire::Weather, String> {
        let agent = user_agent(contact);
        if self.resolved.is_none() || self.at != Some((lat, lng)) {
            self.resolved = Some(
                resolve(lat, lng, &agent).map_err(|e| format!("nws resolve: {e}"))?,
            );
            self.at = Some((lat, lng));
        }
        let resolved = self
            .resolved
            .as_ref()
            .expect("just resolved, or resolved earlier for these coordinates");

        let mut weather =
            observation(resolved, &agent).map_err(|e| format!("nws observation: {e}"))?;
        weather.statement = statement(&resolved.forecast, &agent);
        Ok(weather)
    }
}

/// `GET` with the headers the NWS wants, and the body as text.
fn get(url: &str, user_agent: &str) -> Result<String, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .user_agent(user_agent)
        .build()
        .into();
    agent
        .get(url)
        .header("Accept", "application/geo+json")
        .call()
        .map_err(|e| format!("{url}: {e}"))?
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("{url}: reading the body: {e}"))
}

/// `/points` and then the station list.
fn resolve(lat: f64, lng: f64, agent: &str) -> Result<Resolved, String> {
    #[derive(Deserialize)]
    struct Points {
        properties: PointsProperties,
    }
    #[derive(Deserialize)]
    struct PointsProperties {
        #[serde(default)]
        forecast: String,
        #[serde(default)]
        #[serde(rename = "observationStations")]
        observation_stations: String,
    }

    // Four decimal places, as in Go. The NWS rejects more and rounds anyway.
    let url = format!("https://api.weather.gov/points/{lat:.4},{lng:.4}");
    let points: Points = serde_json::from_str(&get(&url, agent).map_err(|e| format!("points: {e}"))?)
        .map_err(|e| format!("points decode: {e}"))?;
    if points.properties.forecast.is_empty()
        || points.properties.observation_stations.is_empty()
    {
        return Err("points response missing forecast or observationStations URLs".to_string());
    }

    #[derive(Deserialize)]
    struct Stations {
        #[serde(default)]
        features: Vec<Feature>,
    }
    #[derive(Deserialize)]
    struct Feature {
        id: String,
    }

    let stations: Stations = serde_json::from_str(
        &get(&points.properties.observation_stations, agent)
            .map_err(|e| format!("stations: {e}"))?,
    )
    .map_err(|e| format!("stations decode: {e}"))?;
    // Nearest first, which is the order the NWS returns them in.
    let station = stations
        .features
        .first()
        .ok_or_else(|| format!("no observation stations found near {lat:.4},{lng:.4}"))?
        .id
        .clone();

    Ok(Resolved {
        forecast: points.properties.forecast,
        latest: format!("{station}/observations/latest"),
        station,
    })
}

/// The subset of an observation this reads.
///
/// Named so that the latest reading and the recent list share one shape, and
/// every reading is nullable: the NWS sends `null` for anything the station did
/// not report or that does not currently apply.
///
/// **`default` alone is not enough, and every field says so.** `default` covers
/// a key that is absent; the NWS also sends the key with an explicit `null`,
/// and serde reads that as a value of the wrong type. One station's recent list
/// carrying `"textDescription": null` failed the whole decode, and the panel
/// showed no temperature at all for want of a caption.
#[derive(Debug, Default, Deserialize)]
struct ObservationProperties {
    #[serde(default, deserialize_with = "null_as_default")]
    temperature: Reading,
    #[serde(default, deserialize_with = "null_as_default")]
    dewpoint: Reading,
    #[serde(default, deserialize_with = "null_as_default")]
    #[serde(rename = "windChill")]
    wind_chill: Reading,
    #[serde(default, deserialize_with = "null_as_default")]
    #[serde(rename = "textDescription")]
    text_description: String,
    #[serde(default, deserialize_with = "null_as_default")]
    icon: String,
}

/// One nullable Celsius value.
#[derive(Debug, Default, Deserialize)]
struct Reading {
    #[serde(default)]
    value: Option<f64>,
}

/// Read an explicit `null` as the type's default, exactly as an absent key is.
fn null_as_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

/// The station's latest observation, with the recent-list fallback.
fn observation(resolved: &Resolved, agent: &str) -> Result<wire::Weather, String> {
    #[derive(Deserialize)]
    struct Observation {
        properties: ObservationProperties,
    }

    let mut properties = serde_json::from_str::<Observation>(&get(&resolved.latest, agent)?)
        .map_err(|e| format!("observation decode: {e}"))?
        .properties;

    if properties.temperature.value.is_none() {
        // Routine rather than exceptional — see the module note.
        let recovered = recent(&resolved.station, agent).map_err(|e| {
            format!("temperature unavailable (station not reporting): {e}")
        })?;
        if properties.text_description.is_empty() {
            properties.text_description = recovered.text_description.clone();
        }
        if properties.icon.is_empty() {
            properties.icon = recovered.icon.clone();
        }
        properties.temperature = recovered.temperature;
        properties.dewpoint = recovered.dewpoint;
        properties.wind_chill = recovered.wind_chill;
    }

    let mut weather = wire::Weather {
        temp_f: properties.temperature.value.map(to_fahrenheit),
        condition: properties.text_description,
        icon: icon_slug(&properties.icon),
        ..wire::Weather::default()
    };
    // **Wind chill wins when the NWS reports one.** It computes one only below
    // about 50°F with enough wind for it to mean anything, so this picks the
    // winter reading in winter and the summer one otherwise — without Aura
    // deciding what season it is.
    if let Some(chill) = properties.wind_chill.value.map(to_fahrenheit) {
        weather.secondary_f = Some(chill);
        weather.secondary_kind = "windchill".to_string();
    } else if let Some(dewpoint) = properties.dewpoint.value.map(to_fahrenheit) {
        weather.secondary_f = Some(dewpoint);
        weather.secondary_kind = "dewpoint".to_string();
    }
    Ok(weather)
}

/// The newest recent observation that actually carries a temperature.
///
/// The NWS returns this list newest-first, so the first usable record is also
/// the freshest. Nothing here reaches further back than [`RECENT_WINDOW`]: an
/// observation old enough to be wrong is worse than no observation, because the
/// panel gives no sign of a number's age.
fn recent(station: &str, agent: &str) -> Result<ObservationProperties, String> {
    #[derive(Deserialize)]
    struct List {
        #[serde(default)]
        features: Vec<Feature>,
    }
    #[derive(Deserialize)]
    struct Feature {
        properties: ObservationProperties,
    }

    let url = format!("{station}/observations?limit={RECENT_WINDOW}");
    let list: List = serde_json::from_str(&get(&url, agent)?)
        .map_err(|e| format!("recent observations decode: {e}"))?;
    newest_usable(list.features.into_iter().map(|f| f.properties))
}

/// The first record worth recovering, out of a newest-first sequence.
///
/// **Split out of [`recent`] so that it is testable**, and because the rule it
/// carries is the one worth asserting: prefer the newest record that carries a
/// **condition as well as** a temperature. A station's older observations often
/// hold a temperature with an empty icon and description, and recovering one of
/// those puts a reading on screen with no condition beside it — which used to
/// leave an icon-sized hole in the panel and still leaves the reading
/// uncaptioned.
///
/// # Errors
///
/// A sentence, when nothing in the window carries a temperature at all.
fn newest_usable(
    properties: impl Iterator<Item = ObservationProperties>,
) -> Result<ObservationProperties, String> {
    let mut temperature_only: Option<ObservationProperties> = None;
    for candidate in properties {
        if candidate.temperature.value.is_none() {
            continue;
        }
        if !candidate.icon.is_empty() {
            return Ok(candidate);
        }
        if temperature_only.is_none() {
            temperature_only = Some(candidate);
        }
    }
    temperature_only.ok_or_else(|| {
        format!("no temperature in the last {RECENT_WINDOW} observations")
    })
}

/// The `shortForecast` for the current period, or empty.
///
/// **Best-effort by design**: every failure returns an empty string, because a
/// temperature with no forecast beside it is still worth drawing and the
/// caller treats this as non-fatal.
fn statement(forecast_url: &str, agent: &str) -> String {
    #[derive(Deserialize)]
    struct Forecast {
        properties: ForecastProperties,
    }
    #[derive(Deserialize)]
    struct ForecastProperties {
        #[serde(default)]
        periods: Vec<Period>,
    }
    #[derive(Deserialize)]
    struct Period {
        #[serde(default)]
        #[serde(rename = "shortForecast")]
        short_forecast: String,
    }

    let Ok(body) = get(forecast_url, agent) else {
        return String::new();
    };
    serde_json::from_str::<Forecast>(&body)
        .ok()
        .and_then(|forecast| forecast.properties.periods.into_iter().next())
        .map(|period| period.short_forecast)
        .unwrap_or_default()
}

/// Celsius to Fahrenheit, rounded.
fn to_fahrenheit(celsius: f64) -> f64 {
    (celsius * 9.0 / 5.0 + 32.0).round()
}

/// The NWS icon identifier, out of a full icon URL.
///
/// ```text
/// .../icons/land/day/ovc?size=medium              → ovc
/// .../icons/land/night/rain_showers,30?size=medium → rain_showers
/// ```
fn icon_slug(raw: &str) -> String {
    if raw.is_empty() {
        return String::new();
    }
    let without_query = raw.split('?').next().unwrap_or(raw);
    let Some(last) = without_query.rsplit('/').next() else {
        return String::new();
    };
    // The probability suffix — `,30` in `rain_showers,30`.
    last.split(',').next().unwrap_or(last).to_string()
}

// ---------------------------------------------------------------------------
// The cache
// ---------------------------------------------------------------------------

/// What gets written: the reading, and when it was taken.
///
/// **Unix seconds and not RFC 3339**, which is the one place this differs from
/// the Go cache file. Nothing else in this program parses a date, and adding a
/// date-time crate to read back a file this program wrote itself would be a
/// dependency bought for one field. A cache written by the Go controller
/// therefore fails to parse here — which is already a case
/// [`load_cache`] handles, by fetching, exactly as it did before a cache
/// existed.
#[derive(Debug, Serialize, Deserialize)]
struct CacheFile {
    saved_at_unix: i64,
    data: CachedWeather,
}

/// The readings worth keeping. **A subset**: the statement is not cached,
/// because a forecast is about the future and one from an hour ago is wrong in
/// a way a temperature from an hour ago is not.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct CachedWeather {
    temp_f: Option<f64>,
    secondary_f: Option<f64>,
    #[serde(default)]
    secondary_kind: String,
    #[serde(default)]
    condition: String,
    #[serde(default)]
    icon: String,
}

/// The cache's location, beside the config it belongs to.
///
/// # Errors
///
/// As [`crate::config::data_dir`].
pub fn cache_path() -> Result<std::path::PathBuf, rivet_sdk::NotHanded> {
    Ok(crate::config::data_dir()?.join("weather-cache.json"))
}

/// Record the last good reading so a restart has something to draw.
///
/// # Errors
///
/// A sentence — but **failure is not worth failing a poll over**: the cache is
/// an optimisation, and a controller that cannot write it works exactly as it
/// did before there was one. The caller logs it and carries on.
pub fn save_cache(weather: &wire::Weather, now_unix: i64) -> Result<(), String> {
    let path = cache_path().map_err(|e| e.to_string())?;
    let file = CacheFile {
        saved_at_unix: now_unix,
        data: CachedWeather {
            temp_f: weather.temp_f,
            secondary_f: weather.secondary_f,
            secondary_kind: weather.secondary_kind.clone(),
            condition: weather.condition.clone(),
            icon: weather.icon.clone(),
        },
    };
    let text = serde_json::to_string(&file).map_err(|e| format!("weather: encode cache: {e}"))?;
    // Written and renamed so a crash mid-write cannot leave a truncated file
    // that parses as a reading.
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, text.as_bytes())
        .map_err(|e| format!("weather: write cache: {e}"))?;
    std::fs::rename(&temporary, &path).map_err(|e| {
        let _ = std::fs::remove_file(&temporary);
        format!("weather: replace cache: {e}")
    })
}

/// The cached reading and its age, when one exists and is young enough.
///
/// **Everything else comes back as `None`** — no file, unreadable,
/// unparseable, holding no temperature, or too old. The caller then draws
/// nothing and fetches, which is exactly what it did before a cache existed.
///
/// # Errors
///
/// A sentence, only for a file that exists and cannot be parsed. That is worth
/// saying out loud because it means something wrote nonsense into the data
/// directory; every other miss is silent because every other miss is ordinary.
pub fn load_cache(now_unix: i64) -> Result<Option<(wire::Weather, i64)>, String> {
    let Ok(path) = cache_path() else {
        return Ok(None);
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(None);
    };
    let file: CacheFile = serde_json::from_str(&text)
        .map_err(|e| format!("weather: cache unreadable: {e}"))?;

    if file.data.temp_f.is_none() {
        // A cache holding no temperature is a cache of a failure, and restoring
        // it would put the broken-cloud icon on screen for no reason.
        return Ok(None);
    }
    let age = now_unix - file.saved_at_unix;
    if age < 0 {
        // **The clock moved backwards.** A Pi with no RTC does this on every
        // boot until NTP lands. Unusable rather than infinitely fresh.
        return Ok(None);
    }
    if age > MAX_CACHE_AGE_SECONDS {
        return Ok(None);
    }
    Ok(Some((
        wire::Weather {
            temp_f: file.data.temp_f,
            secondary_f: file.data.secondary_f,
            secondary_kind: file.data.secondary_kind,
            condition: file.data.condition,
            icon: file.data.icon,
            // Not cached; see [`CachedWeather`].
            statement: String::new(),
            unit: String::new(),
        },
        age,
    )))
}

#[cfg(test)]
mod tests;
