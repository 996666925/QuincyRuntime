//! CSS value parsing helpers shared by layout, hit testing and painting.
//!
//! Keeping these conversions together prevents each rendering path from
//! interpreting lengths, colors, and shorthand box values differently.

use std::collections::BTreeMap;

use skia_safe::Color;
use taffy::prelude::{Dimension, LengthPercentageAuto};

pub(crate) fn declarations(value: &str) -> BTreeMap<String, String> {
    value
        .split(';')
        .filter_map(|part| part.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
        .collect()
}

pub(crate) fn pixels(value: &str) -> Option<f32> {
    let value = value.trim().to_ascii_lowercase();
    let (number, multiplier) = if let Some(value) = value.strip_suffix("px") {
        (value, 1.0)
    } else if let Some(value) = value.strip_suffix("rem") {
        (value, 16.0)
    } else if let Some(value) = value.strip_suffix("em") {
        (value, 16.0)
    } else if let Some(value) = value.strip_suffix("pt") {
        (value, 96.0 / 72.0)
    } else {
        (value.as_str(), 1.0)
    };
    number.parse::<f32>().ok().map(|value| value * multiplier)
}

pub(crate) fn box_values(value: &str) -> Option<[f32; 4]> {
    let values = value
        .split_whitespace()
        .map(pixels)
        .collect::<Option<Vec<_>>>()?;
    match values.as_slice() {
        [all] => Some([*all; 4]),
        [vertical, horizontal] => Some([*vertical, *horizontal, *vertical, *horizontal]),
        [top, horizontal, bottom] => Some([*top, *horizontal, *bottom, *horizontal]),
        [top, right, bottom, left] => Some([*top, *right, *bottom, *left]),
        _ => None,
    }
}

pub(crate) fn dimension(value: &str) -> Option<Dimension> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("auto") {
        return Some(Dimension::auto());
    }
    if let Some(percent) = value.strip_suffix('%') {
        return percent
            .parse::<f32>()
            .ok()
            .map(|v| Dimension::percent(v / 100.0));
    }
    pixels(value).map(Dimension::length)
}

pub(crate) fn dimension_auto(value: &str) -> Option<LengthPercentageAuto> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("auto") {
        return Some(LengthPercentageAuto::auto());
    }
    if let Some(percent) = value.strip_suffix('%') {
        return percent
            .parse::<f32>()
            .ok()
            .map(|v| LengthPercentageAuto::percent(v / 100.0));
    }
    pixels(value).map(LengthPercentageAuto::length)
}

pub(crate) fn color(value: &str) -> Option<Color> {
    let value = value.trim().to_ascii_lowercase();
    match value.as_str() {
        "transparent" => Some(Color::TRANSPARENT),
        "white" => Some(Color::WHITE),
        "black" => Some(Color::BLACK),
        "red" => Some(Color::from_rgb(255, 0, 0)),
        "green" => Some(Color::from_rgb(0, 128, 0)),
        "blue" => Some(Color::from_rgb(0, 0, 255)),
        "gray" | "grey" => Some(Color::from_rgb(128, 128, 128)),
        "yellow" => Some(Color::from_rgb(255, 255, 0)),
        "orange" => Some(Color::from_rgb(255, 165, 0)),
        "purple" => Some(Color::from_rgb(128, 0, 128)),
        "pink" => Some(Color::from_rgb(255, 192, 203)),
        "brown" => Some(Color::from_rgb(165, 42, 42)),
        "navy" => Some(Color::from_rgb(0, 0, 128)),
        "teal" => Some(Color::from_rgb(0, 128, 128)),
        "silver" => Some(Color::from_rgb(192, 192, 192)),
        "lime" => Some(Color::from_rgb(0, 255, 0)),
        other => rgb_color(other).or_else(|| hex_color(other)),
    }
}

fn rgb_color(value: &str) -> Option<Color> {
    let (has_alpha, body) = if let Some(body) = value
        .strip_prefix("rgba(")
        .and_then(|v| v.strip_suffix(')'))
    {
        (true, body)
    } else {
        let body = value
            .strip_prefix("rgb(")
            .and_then(|v| v.strip_suffix(')'))?;
        (false, body)
    };
    let mut channels = body.split(',').map(str::trim);
    let red = channels.next()?.parse::<u8>().ok()?;
    let green = channels.next()?.parse::<u8>().ok()?;
    let blue = channels.next()?.parse::<u8>().ok()?;
    if has_alpha {
        let alpha = channels.next()?.parse::<f32>().ok()?.clamp(0.0, 1.0);
        Some(Color::from_argb(
            (alpha * 255.0).round() as u8,
            red,
            green,
            blue,
        ))
    } else {
        Some(Color::from_rgb(red, green, blue))
    }
}

fn hex_color(value: &str) -> Option<Color> {
    let value = value.strip_prefix('#')?;
    let value = if value.len() == 3 {
        return Some(Color::from_rgb(
            u8::from_str_radix(&value[0..1].repeat(2), 16).ok()?,
            u8::from_str_radix(&value[1..2].repeat(2), 16).ok()?,
            u8::from_str_radix(&value[2..3].repeat(2), 16).ok()?,
        ));
    } else if value.len() == 6 || value.len() == 8 {
        value
    } else {
        return None;
    };
    if value.len() == 8 {
        return Some(Color::from_argb(
            u8::from_str_radix(&value[6..8], 16).ok()?,
            u8::from_str_radix(&value[0..2], 16).ok()?,
            u8::from_str_radix(&value[2..4], 16).ok()?,
            u8::from_str_radix(&value[4..6], 16).ok()?,
        ));
    }
    Some(Color::from_rgb(
        u8::from_str_radix(&value[0..2], 16).ok()?,
        u8::from_str_radix(&value[2..4], 16).ok()?,
        u8::from_str_radix(&value[4..6], 16).ok()?,
    ))
}

pub(crate) fn border(value: &str) -> (f32, Color) {
    let mut width = 1.0;
    let mut color = Color::from_rgb(80, 80, 80);
    for token in value.split_whitespace() {
        if let Some(parsed) = pixels(token) {
            width = parsed.max(0.0);
        } else if let Some(parsed) = self::color(token) {
            color = parsed;
        }
    }
    (width, color)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_declarations_case_insensitively() {
        let parsed = declarations(" COLOR: white; width : 12px ");
        assert_eq!(parsed.get("color").map(String::as_str), Some("white"));
        assert_eq!(pixels(parsed.get("width").unwrap()), Some(12.0));
    }

    #[test]
    fn expands_css_box_shorthand() {
        assert_eq!(box_values("1px 2px 3px 4px"), Some([1.0, 2.0, 3.0, 4.0]));
        assert_eq!(box_values("4px 8px"), Some([4.0, 8.0, 4.0, 8.0]));
    }

    #[test]
    fn parses_named_and_hex_colors_and_border_width() {
        assert_eq!(color("#0f0"), Some(Color::from_rgb(0, 255, 0)));
        assert_eq!(color("BLUE"), Some(Color::from_rgb(0, 0, 255)));
        assert_eq!(border("3px solid #123456").0, 3.0);
    }

    #[test]
    fn parses_modern_lengths_and_rgba_colors() {
        assert_eq!(pixels("1rem"), Some(16.0));
        assert_eq!(pixels("12pt"), Some(16.0));
        assert_eq!(
            color("rgba(10, 20, 30, 0.5)"),
            Some(Color::from_argb(128, 10, 20, 30))
        );
        assert_eq!(color("#10203080"), Some(Color::from_argb(128, 16, 32, 48)));
    }
}
