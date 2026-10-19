// SPDX-License-Identifier: GPL-3.0-or-later
//! `datetime.strftime` of a naive value, as Python on glibc prints it in the C locale: the
//! directives of C, glibc's flags and width, and `%f` for the microseconds. `%z` and `%Z` are
//! empty, as for a `datetime` with no zone. A directive nothing knows stays as it was written.

use super::{DateTime, days_from_civil};

const DAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// A flag after `%`: how a number is padded, or how a name is cased.
#[derive(Default, Clone, Copy)]
struct Flags {
    /// `-`: no padding; `_`: spaces; `0`: zeros.
    pad: Option<char>,
    /// `^` or `#`: upper case (lower case for `%p` with `#`).
    upper: bool,
    swap: bool,
}

/// What a directive prints before flags and width apply.
enum Piece {
    /// A number, its default width and its default fill.
    Number(i64, usize, char),
    Text(String),
}

/// `datetime.strftime(format)`.
pub(super) fn format(at: &DateTime, format: &str) -> String {
    let chars: Vec<char> = format.chars().collect();
    let mut out = String::with_capacity(format.len() * 2);
    let mut next = 0;
    while let Some(&c) = chars.get(next) {
        next += 1;
        if c != '%' {
            out.push(c);
            continue;
        }
        let start = next - 1;
        let mut flags = Flags::default();
        while let Some(&flag) = chars.get(next)
            && matches!(flag, '-' | '_' | '0' | '^' | '#')
        {
            match flag {
                '^' => flags.upper = true,
                '#' => flags.swap = true,
                other => flags.pad = Some(other),
            }
            next += 1;
        }
        let mut width: Option<usize> = None;
        while let Some(digit) = chars.get(next).and_then(|c| c.to_digit(10)) {
            width = Some(
                width
                    .unwrap_or(0)
                    .saturating_mul(10)
                    .saturating_add(digit as usize),
            );
            next += 1;
        }
        if matches!(chars.get(next), Some('E' | 'O')) {
            next += 1;
        }
        let Some(&directive) = chars.get(next) else {
            out.extend(&chars[start..]);
            break;
        };
        next += 1;
        match piece(at, directive) {
            Some(piece) => out.push_str(&place(piece, flags, width, directive)),
            None => out.extend(&chars[start..next]),
        }
    }
    out
}

/// A directive's value, or `None` when nothing in C or in Python has the directive.
fn piece(at: &DateTime, directive: char) -> Option<Piece> {
    let hour12 = match at.hour % 12 {
        0 => 12,
        hour => i64::from(hour),
    };
    let (iso_year, iso_week) = iso_week(at);
    let yday = at.days() - days_from_civil(at.year, 1, 1);
    let sunday = i64::from(at.weekday() + 1) % 7;
    let monday = i64::from(at.weekday());
    let name = |names: &[&str], index: u32| names[index as usize].to_string();
    Some(match directive {
        'Y' => Piece::Number(at.year, 1, '0'),
        'C' => Piece::Number(at.year.div_euclid(100), 2, '0'),
        'y' => Piece::Number(at.year.rem_euclid(100), 2, '0'),
        'G' => Piece::Number(iso_year, 1, '0'),
        'g' => Piece::Number(iso_year.rem_euclid(100), 2, '0'),
        'm' => Piece::Number(i64::from(at.month), 2, '0'),
        'd' => Piece::Number(i64::from(at.day), 2, '0'),
        'e' => Piece::Number(i64::from(at.day), 2, ' '),
        'H' => Piece::Number(i64::from(at.hour), 2, '0'),
        'k' => Piece::Number(i64::from(at.hour), 2, ' '),
        'I' => Piece::Number(hour12, 2, '0'),
        'l' => Piece::Number(hour12, 2, ' '),
        'M' => Piece::Number(i64::from(at.minute), 2, '0'),
        'S' => Piece::Number(i64::from(at.second), 2, '0'),
        'f' => Piece::Number(i64::from(at.microsecond), 6, '0'),
        'j' => Piece::Number(yday + 1, 3, '0'),
        'u' => Piece::Number(monday + 1, 1, '0'),
        'w' => Piece::Number(sunday, 1, '0'),
        'U' => Piece::Number((yday + 7 - sunday) / 7, 2, '0'),
        'W' => Piece::Number((yday + 7 - monday) / 7, 2, '0'),
        'V' => Piece::Number(iso_week, 2, '0'),
        's' => Piece::Number(at.local_seconds(), 1, '0'),
        'a' => Piece::Text(name(&DAYS, (at.weekday() + 1) % 7)[..3].to_string()),
        'A' => Piece::Text(name(&DAYS, (at.weekday() + 1) % 7)),
        'b' | 'h' => Piece::Text(name(&MONTHS, at.month - 1)[..3].to_string()),
        'B' => Piece::Text(name(&MONTHS, at.month - 1)),
        'p' => Piece::Text(if at.hour < 12 { "AM" } else { "PM" }.into()),
        'P' => Piece::Text(if at.hour < 12 { "am" } else { "pm" }.into()),
        'z' | 'Z' => Piece::Text(String::new()),
        'n' => Piece::Text("\n".into()),
        't' => Piece::Text("\t".into()),
        '%' => Piece::Text("%".into()),
        'c' => Piece::Text(format(at, "%a %b %e %H:%M:%S %Y")),
        'D' | 'x' => Piece::Text(format(at, "%m/%d/%y")),
        'F' => Piece::Text(format(at, "%Y-%m-%d")),
        'r' => Piece::Text(format(at, "%I:%M:%S %p")),
        'R' => Piece::Text(format(at, "%H:%M")),
        'T' | 'X' => Piece::Text(format(at, "%H:%M:%S")),
        _ => return None,
    })
}

/// A piece with the flags and width of its directive applied.
fn place(piece: Piece, flags: Flags, width: Option<usize>, directive: char) -> String {
    let (text, fill, default_width) = match piece {
        Piece::Number(value, default_width, fill) => {
            (value.to_string(), fill, width.unwrap_or(default_width))
        }
        Piece::Text(text) => {
            let text = if flags.upper || (flags.swap && directive != 'p') {
                text.to_uppercase()
            } else if flags.swap {
                text.to_lowercase()
            } else {
                text
            };
            (text, ' ', width.unwrap_or(0))
        }
    };
    let (fill, width) = match flags.pad {
        Some('-') => (fill, 0),
        Some('_') => (' ', default_width),
        Some(_) => ('0', default_width),
        None => (fill, default_width),
    };
    let missing = width.saturating_sub(text.chars().count());
    let mut out: String = std::iter::repeat_n(fill, missing).collect();
    out.push_str(&text);
    out
}

/// The ISO 8601 year and week of a date.
fn iso_week(at: &DateTime) -> (i64, i64) {
    let yday = at.days() - days_from_civil(at.year, 1, 1);
    let week = (yday - i64::from(at.weekday()) + 10) / 7;
    if week < 1 {
        (at.year - 1, weeks_in(at.year - 1))
    } else if week > weeks_in(at.year) {
        (at.year + 1, 1)
    } else {
        (at.year, week)
    }
}

/// How many ISO weeks a year has: 53 when it starts or ends on a Thursday.
fn weeks_in(year: i64) -> i64 {
    let weekday =
        |y: i64| (y + y.div_euclid(4) - y.div_euclid(100) + y.div_euclid(400)).rem_euclid(7);
    if weekday(year) == 4 || weekday(year - 1) == 3 {
        53
    } else {
        52
    }
}
