//! **A zip code to a latitude and longitude**, once, and then never again.
//!
//! `geocodeZip` and `resolveGeoCache` in `cmd/aura-controller/main.go`.
//! `api.zippopotam.us` is free, needs no key and knows US zip codes, which is
//! the whole of what Aura asks of it.
//!
//! # The result is written into the config, and that is why this is cached
//!
//! The NWS wants coordinates and a person knows their zip code. Resolving on
//! every start would be a request to somebody else's free service on every
//! reboot for an answer that has not changed since the maintainer moved house.
//! So the answer is kept in `geo_cache`, keyed by the zip it came from — a
//! cache whose key no longer matches the configured zip is stale by
//! construction, which is the only invalidation rule this needs.

use serde::Deserialize;

/// Fifteen seconds, as in Go.
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// What `api.zippopotam.us/us/<zip>` answers with.
#[derive(Debug, Deserialize)]
struct Response {
    #[serde(default)]
    places: Vec<Place>,
}

/// The coordinates arrive as **strings**, which is the service's choice and not
/// a mistake here.
#[derive(Debug, Deserialize)]
struct Place {
    latitude: String,
    longitude: String,
}

/// Resolve a US zip code to coordinates.
///
/// # Errors
///
/// A sentence naming which step failed — the request, the status, the decode,
/// an empty result, or a coordinate that is not a number. A failure is not
/// fatal to the caller: it logs and keeps whatever cache it already had, so a
/// weather panel that was working goes on working through an outage at a free
/// geocoding service.
pub fn zip(code: &str) -> Result<(f64, f64), String> {
    let url = format!("https://api.zippopotam.us/us/{code}");
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .build()
        .into();

    let response: Response = agent
        .get(&url)
        .call()
        .map_err(|e| format!("geocode request: {e}"))?
        .body_mut()
        .read_json()
        .map_err(|e| format!("geocode decode: {e}"))?;

    let place = response
        .places
        .first()
        .ok_or_else(|| format!("geocode: no results for zip {code}"))?;
    let latitude = place
        .latitude
        .parse::<f64>()
        .map_err(|e| format!("geocode parse lat: {e}"))?;
    let longitude = place
        .longitude
        .parse::<f64>()
        .map_err(|e| format!("geocode parse lng: {e}"))?;
    Ok((latitude, longitude))
}
