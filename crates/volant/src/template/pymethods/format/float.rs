// SPDX-License-Identifier: GPL-3.0-or-later
//! Python's text for a float: `repr`, and the `e`, `f`, `g` and `%` presentation types.

/// What `repr(f)` prints: the shortest digits that read back, in positional form for exponents
/// from -4 to 15 and in scientific form outside that.
pub(crate) fn repr(f: f64) -> String {
    if !f.is_finite() {
        return special(f, false);
    }
    let sci = format!("{:e}", f.abs());
    let (digits, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let sign = if f.is_sign_negative() { "-" } else { "" };
    if (-4..16).contains(&exp) {
        let mut text = format!("{}", f.abs());
        if !text.contains('.') {
            text.push_str(".0");
        }
        format!("{sign}{text}")
    } else {
        format!("{sign}{digits}e{}", exponent(exp))
    }
}

/// `inf`, `-inf` and `nan`, in capitals when asked.
pub(crate) fn special(f: f64, upper: bool) -> String {
    let text = if f.is_nan() {
        "nan"
    } else if f < 0.0 {
        "-inf"
    } else {
        "inf"
    };
    if upper {
        text.to_uppercase()
    } else {
        text.to_string()
    }
}

/// An exponent as C writes it: a sign and at least two digits.
fn exponent(exp: i32) -> String {
    format!("{}{:02}", if exp < 0 { '-' } else { '+' }, exp.abs())
}

/// Digits after the point past which a float's exact decimal expansion is all zeros: a double
/// has at most 767 significant digits in scientific form and 1074 places after the point.
const EXACT_SCI: usize = 800;
const EXACT_FIXED: usize = 1100;

/// `{f:.precision$e}` for any precision. Rust's formatter refuses one above 65535, and the
/// digits past `EXACT_SCI` are zeros, so those are written here.
fn sci(f: f64, precision: usize) -> String {
    let text = format!("{f:.prec$e}", prec = precision.min(EXACT_SCI));
    if precision <= EXACT_SCI {
        return text;
    }
    let (mantissa, exp) = text.split_once('e').unwrap_or((&text, "0"));
    format!("{mantissa}{}e{exp}", "0".repeat(precision - EXACT_SCI))
}

/// `%e` with `precision` digits after the point. `f` is not negative.
pub(crate) fn exp_style(f: f64, precision: usize, alt: bool, upper: bool) -> String {
    let sci = sci(f, precision);
    let (mantissa, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let mut mantissa = mantissa.to_string();
    if alt && precision == 0 {
        mantissa.push('.');
    }
    let text = format!("{mantissa}e{}", exponent(exp.parse().unwrap_or(0)));
    if upper { text.to_uppercase() } else { text }
}

/// `%f` with `precision` digits after the point. `f` is not negative.
pub(crate) fn fixed(f: f64, precision: usize, alt: bool) -> String {
    let mut text = format!("{f:.prec$}", prec = precision.min(EXACT_FIXED));
    if precision > EXACT_FIXED {
        text.push_str(&"0".repeat(precision - EXACT_FIXED));
    }
    if alt && precision == 0 {
        text.push('.');
    }
    text
}

/// `%g`: `precision` significant digits (at least one), positional or scientific as the
/// exponent says, trailing zeros dropped unless `alt`. `f` is not negative.
pub(crate) fn general(f: f64, precision: usize, alt: bool, upper: bool) -> String {
    let precision = precision.max(1);
    let sci = sci(f, precision - 1);
    let exp: i32 = sci
        .split_once('e')
        .and_then(|(_, e)| e.parse().ok())
        .unwrap_or(0);
    let mut text = if exp < -4 || exp >= i32::try_from(precision).unwrap_or(i32::MAX) {
        exp_style(f, precision - 1, alt, upper)
    } else {
        let decimals =
            usize::try_from(i32::try_from(precision).unwrap_or(0) - 1 - exp).unwrap_or(0);
        fixed(f, decimals, alt)
    };
    if !alt {
        text = trim_zeros(&text);
    }
    text
}

/// `1.500` is `1.5` and `2.000e+05` is `2e+05`: the zeros after the point, and the point when
/// nothing is left behind it.
fn trim_zeros(text: &str) -> String {
    let (body, tail) = text
        .find(['e', 'E'])
        .map_or((text, ""), |at| (&text[..at], &text[at..]));
    if !body.contains('.') {
        return text.to_string();
    }
    format!("{}{tail}", body.trim_end_matches('0').trim_end_matches('.'))
}

/// The presentation of a float with no type letter: `repr`, or `g` with the precision given
/// and at least one digit after the point.
pub(crate) fn plain(f: f64, precision: Option<usize>) -> String {
    match precision {
        None => repr(f.abs()),
        Some(p) => {
            let text = general(f.abs(), p, false, false);
            if text.contains(['.', 'e', 'n', 'i']) {
                text
            } else {
                format!("{text}.0")
            }
        }
    }
}
