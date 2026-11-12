// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Small things every panel uses, over the screen kit's builders.

use console::{Attr, Screen};
use scrkit::ui;

/// Rows a list or a text may fill between the heading and the keys.
pub const BODY_ROWS: usize = 15;

/// A screen Enter works on. The console reports Enter only from a
/// screen with an input field, so one that has none gets a field of one
/// cell that shows nothing, above the function keys.
#[must_use]
pub fn enterable(mut s: Screen) -> Screen {
    if s.fields.is_empty() {
        s.field(ui::SELECTION_ROW + 1, ui::BODY_COL, 1, true);
    }
    s
}

/// Text lines as the builders take them.
#[must_use]
pub fn lines(text: &[&str]) -> Vec<String> {
    text.iter().map(ToString::to_string).collect()
}

/// A text display Enter works on.
#[must_use]
pub fn page(title: &str, system: &str, text: &[String], keys: &str) -> Screen {
    enterable(ui::text(title, system, text, None, keys))
}

/// Put `text` on `row` from the body column.
pub fn put(s: &mut Screen, row: usize, text: &str) {
    s.put(row, ui::BODY_COL, text, Attr::GREEN);
}

/// The number typed in a selection field, when it is one.
#[must_use]
pub fn number(field: Option<&String>) -> Option<u32> {
    field.and_then(|f| f.trim().parse().ok())
}

/// A size as an operator reads it: `931.5 GB`, in powers of ten as
/// disks are sold.
#[must_use]
pub fn size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];
    let mut unit = 0;
    let mut tenths = bytes.saturating_mul(10);
    while tenths >= 10_000 && unit + 1 < UNITS.len() {
        tenths /= 1000;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{}.{} {}", tenths / 10, tenths % 10, UNITS[unit])
    }
}

/// A label with dot leaders to a colon at column `width`, the dots in
/// the same columns whatever the label's length: `Host name  . . . :`.
#[must_use]
pub fn dotted(label: &str, width: usize) -> String {
    let mut out = format!("{label} ");
    while out.len() < width {
        out.push(if (width - out.len()).is_multiple_of(2) {
            '.'
        } else {
            ' '
        });
    }
    out.push(':');
    out
}

/// `text` cut to `width` characters, or padded to it.
#[must_use]
pub fn fit(text: &str, width: usize) -> String {
    let cut: String = text.chars().take(width).collect();
    format!("{cut:<width$}")
}

/// One page of `rows`, `top` being the first shown: the rows, and the
/// words for the foot of the list.
#[must_use]
pub fn window(rows: &[String], top: usize, height: usize) -> (Vec<String>, String) {
    let top = top.min(rows.len().saturating_sub(1));
    let shown: Vec<String> = rows.iter().skip(top).take(height).cloned().collect();
    let foot = if rows.is_empty() {
        "(None)".to_string()
    } else if top + shown.len() < rows.len() {
        "More...".to_string()
    } else {
        "Bottom".to_string()
    };
    (shown, foot)
}

/// Where a list is after F7 (back) or F8 (on) by one page.
#[must_use]
pub fn scroll(top: usize, key: u8, len: usize, height: usize) -> usize {
    match key {
        7 => top.saturating_sub(height),
        8 if top + height < len => top + height,
        _ => top,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_screen_without_fields_gets_one_that_shows_nothing() {
        let s = page("Title", "SYS", &lines(&["one"]), "Enter=Continue");
        assert_eq!(s.fields.len(), 1);
        assert!(s.fields[0].nondisplay);
        let form = ui::form("T", "SYS", &[ui::FormLine::new("A", 3, "")], "");
        assert_eq!(enterable(form).fields.len(), 1);
    }

    #[test]
    fn sizes_read_as_disks_are_sold() {
        assert_eq!(size(512), "512 B");
        assert_eq!(size(1_073_741_824), "1.0 GB");
        assert_eq!(size(931_500_000_000), "931.5 GB");
        assert_eq!(size(4_000_787_030_016), "4.0 TB");
        assert_eq!(size(250_000_000), "250.0 MB");
    }

    #[test]
    fn a_list_pages() {
        let rows: Vec<String> = (0..25).map(|i| i.to_string()).collect();
        let (shown, foot) = window(&rows, 0, 10);
        assert_eq!((shown.len(), foot.as_str()), (10, "More..."));
        let top = scroll(0, 8, rows.len(), 10);
        assert_eq!(top, 10);
        let top = scroll(top, 8, rows.len(), 10);
        let (shown, foot) = window(&rows, top, 10);
        assert_eq!((shown.len(), foot.as_str()), (5, "Bottom"));
        assert_eq!(scroll(top, 8, rows.len(), 10), 20);
        assert_eq!(scroll(top, 7, rows.len(), 10), 10);
        assert_eq!(window(&[], 0, 10).1, "(None)");
        assert_eq!(dotted("Protection", 23), "Protection . . . . . . :");
        assert_eq!(dotted("Host name", 23), "Host name  . . . . . . :");
        assert_eq!(dotted("A label that is too long", 23), "A label that is too long :");
        assert_eq!(fit("abcdef", 3), "abc");
        assert_eq!(fit("ab", 4), "ab  ");
        assert_eq!(number(Some(&" 7 ".to_string())), Some(7));
        assert_eq!(number(Some(&"x".to_string())), None);
    }
}
