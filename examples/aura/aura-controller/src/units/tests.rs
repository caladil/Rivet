//! The conversion, its rounding, and the absent case.

use super::*;

#[test]
fn imperial_passes_the_reading_through_and_names_fahrenheit() {
    let (value, unit) = to_display(Some(70.0), false);
    assert_eq!(value, Some(70.0));
    assert_eq!(unit, "F");
}

#[test]
fn metric_converts_and_names_celsius() {
    let (value, unit) = to_display(Some(212.0), true);
    assert_eq!(value, Some(100.0));
    assert_eq!(unit, "C");

    let (freezing, _) = to_display(Some(32.0), true);
    assert_eq!(freezing, Some(0.0));
}

/// The renderer prints what it is given, so the rounding has to happen here or
/// a wall panel shows `21.11111111111111`.
#[test]
fn the_converted_reading_is_rounded_to_a_whole_degree() {
    let (value, _) = to_display(Some(70.0), true);
    assert_eq!(value, Some(21.0));
}

/// **Absent is not zero**, and a unit letter beside no reading would be a label
/// on nothing.
#[test]
fn an_absent_reading_converts_to_an_absent_reading_and_no_unit() {
    let (value, unit) = to_display(None, false);
    assert_eq!(value, None);
    assert_eq!(unit, "");

    let (value, unit) = to_display(None, true);
    assert_eq!(value, None);
    assert_eq!(unit, "");
}

/// Zero Fahrenheit is an ordinary January reading in Minnesota, and it must
/// survive the conversion as a reading rather than becoming absent.
#[test]
fn zero_fahrenheit_is_a_reading_and_not_an_absence() {
    let (value, unit) = to_display(Some(0.0), false);
    assert_eq!(value, Some(0.0));
    assert_eq!(unit, "F");

    let (value, _) = to_display(Some(0.0), true);
    assert_eq!(value, Some(-18.0));
}
