//! **Unit conversion happens once, on the way out** — `toDisplayUnit` in
//! `cmd/aura-controller/main.go`.
//!
//! The schema says it plainly:
//!
//! > Readings are held in Fahrenheit everywhere upstream and converted on the
//! > way out, so `temp_f`, `secondary_f` and `indoor.temp_f` are always in the
//! > unit named by `unit` **despite the field name**.
//!
//! That is the whole design and it is worth stating why: a program that
//! converted at the point of collection would hold two units at once — the NWS
//! answers in Celsius, TempMon answers in Celsius, and both are turned into
//! Fahrenheit by their own clients — and the cache, the log lines and the
//! thresholds would each have to say which. Converting in one place, at the
//! last moment, means everything between the client and here is in one unit and
//! no comment has to say so.

/// A reading and the letter that names its unit.
///
/// `None` in gives `None` out **and an empty unit**: a unit letter beside no
/// reading would be a label on nothing, and the schema omits both.
pub fn to_display(reading: Option<f64>, metric: bool) -> (Option<f64>, String) {
    let Some(fahrenheit) = reading else {
        return (None, String::new());
    };
    if !metric {
        return (Some(fahrenheit), "F".to_string());
    }
    (
        Some(((fahrenheit - 32.0) * 5.0 / 9.0).round()),
        "C".to_string(),
    )
}

#[cfg(test)]
mod tests;
