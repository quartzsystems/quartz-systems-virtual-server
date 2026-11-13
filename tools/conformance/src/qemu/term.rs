// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The serial port as a terminal would show it: a 24x80 grid kept
//! from the bytes the guest sends. The installer and Service Tools
//! draw with cursor addressing; PID 1 prints lines. Both end up here.
//! Colors and other attributes are taken and not kept: a golden file
//! is the characters.

/// Rows of the grid.
pub const ROWS: usize = 24;
/// Columns of the grid.
pub const COLS: usize = 80;

/// The keys a suite types, as a terminal sends them.
pub mod key {
    /// Enter.
    pub const ENTER: &str = "\r";
    /// Field Advance.
    pub const TAB: &str = "\t";
    /// Field Exit: clears the rest of the field and goes to the next.
    pub const FIELD_EXIT: &str = "\x0b";
    /// F1.
    pub const F1: &str = "\x1bOP";
    /// F3.
    pub const F3: &str = "\x1bOR";
    /// F7.
    pub const F7: &str = "\x1b[18~";
    /// F8.
    pub const F8: &str = "\x1b[19~";
    /// F10.
    pub const F10: &str = "\x1b[21~";
    /// F11.
    pub const F11: &str = "\x1b[23~";
    /// F12.
    pub const F12: &str = "\x1b[24~";
}

/// Where the parser is in an escape sequence.
#[derive(Clone, Debug, PartialEq, Eq)]
enum State {
    Text,
    Escape,
    Csi(String),
    /// A two-byte sequence's second byte.
    Skip,
}

/// The grid.
#[derive(Clone, Debug)]
pub struct Grid {
    cells: Vec<[char; COLS]>,
    row: usize,
    col: usize,
    state: State,
}

impl Default for Grid {
    fn default() -> Self {
        Self {
            cells: vec![[' '; COLS]; ROWS],
            row: 0,
            col: 0,
            state: State::Text,
        }
    }
}

impl Grid {
    fn clear(&mut self) {
        self.cells = vec![[' '; COLS]; ROWS];
    }

    fn line_feed(&mut self) {
        if self.row + 1 < ROWS {
            self.row += 1;
        } else {
            self.cells.remove(0);
            self.cells.push([' '; COLS]);
        }
    }

    fn put(&mut self, c: char) {
        if self.col >= COLS {
            self.col = 0;
            self.line_feed();
        }
        self.cells[self.row][self.col] = c;
        self.col += 1;
    }

    fn csi(&mut self, params: &str, last: char) {
        let mut numbers = params
            .trim_start_matches('?')
            .split(';')
            .map(|p| p.parse::<usize>().unwrap_or(0));
        let first = numbers.next().unwrap_or(0);
        match last {
            'H' | 'f' => {
                let col = numbers.next().unwrap_or(0);
                self.row = first.clamp(1, ROWS) - 1;
                self.col = col.clamp(1, COLS) - 1;
            }
            'J' if first == 2 => self.clear(),
            'J' => {
                for c in self.col..COLS {
                    self.cells[self.row][c] = ' ';
                }
                for r in self.row + 1..ROWS {
                    self.cells[r] = [' '; COLS];
                }
            }
            'K' => {
                for c in self.col..COLS {
                    self.cells[self.row][c] = ' ';
                }
            }
            'A' => self.row = self.row.saturating_sub(first.max(1)),
            'B' => self.row = (self.row + first.max(1)).min(ROWS - 1),
            'C' => self.col = (self.col + first.max(1)).min(COLS - 1),
            'D' => self.col = self.col.saturating_sub(first.max(1)),
            // Attributes and modes change no character.
            _ => {}
        }
    }

    /// Take what the guest sent.
    pub fn feed(&mut self, text: &str) {
        for c in text.chars() {
            match std::mem::replace(&mut self.state, State::Text) {
                State::Text => match c {
                    '\x1b' => self.state = State::Escape,
                    // The capture drops carriage returns: a line feed
                    // is a new line.
                    '\n' => {
                        self.col = 0;
                        self.line_feed();
                    }
                    '\r' => self.col = 0,
                    '\x08' => self.col = self.col.saturating_sub(1),
                    c if c.is_control() => {}
                    c => self.put(c),
                },
                State::Escape => match c {
                    '[' => self.state = State::Csi(String::new()),
                    '(' | ')' | '#' => self.state = State::Skip,
                    _ => {}
                },
                State::Csi(mut params) => {
                    if c.is_ascii_digit() || c == ';' || c == '?' {
                        params.push(c);
                        self.state = State::Csi(params);
                    } else {
                        self.csi(&params, c);
                    }
                }
                State::Skip => {}
            }
        }
    }

    /// The 24 rows, each without the blanks at its end.
    pub fn rows(&self) -> Vec<String> {
        self.cells
            .iter()
            .map(|row| row.iter().collect::<String>().trim_end().to_string())
            .collect()
    }

    /// The grid as text: 24 lines.
    pub fn text(&self) -> String {
        let mut text = self.rows().join("\n");
        text.push('\n');
        text
    }

    /// One row, from 1.
    pub fn row(&self, row: usize) -> String {
        self.rows()
            .get(row.wrapping_sub(1))
            .cloned()
            .unwrap_or_default()
    }

    /// The message line: row 24.
    pub fn message(&self) -> String {
        self.row(ROWS).trim().to_string()
    }

    /// Whether any row contains `text`.
    pub fn contains(&self, text: &str) -> bool {
        self.rows().iter().any(|row| row.contains(text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_addressing_puts_text_where_the_display_sent_it() {
        let mut grid = Grid::default();
        // What an ANSI display sends for a title and a field.
        grid.feed("\x1b[?25l\x1b[0m\x1b[2J\x1b[1;33H\x1b[0;97mInstall QS/VS");
        grid.feed("\x1b[4;2H\x1b[0;32mHost name . . :\x1b[4;19H\x1b[0;32;4mvs-01");
        grid.feed("\x1b[0m\x1b[4;19H\x1b[?25h");
        assert_eq!(grid.row(1), format!("{}Install QS/VS", " ".repeat(32)));
        assert_eq!(grid.row(4), " Host name . . :  vs-01");
        assert!(grid.contains("vs-01"));
        // A changed cell only.
        grid.feed("\x1b[4;23H2");
        assert_eq!(grid.row(4), " Host name . . :  vs-02");
        grid.feed("\x1b[24;1HSelection not valid.");
        assert_eq!(grid.message(), "Selection not valid.");
        assert_eq!(grid.text().lines().count(), ROWS);
    }

    #[test]
    fn lines_scroll_as_on_a_terminal() {
        let mut grid = Grid::default();
        for i in 0..30 {
            grid.feed(&format!("REF line {i}\n"));
        }
        assert_eq!(grid.row(1), "REF line 7");
        assert_eq!(grid.row(23), "REF line 29");
        assert_eq!(grid.row(24), "");
        // A screen drawn after lines clears them.
        grid.feed("\x1b[2J\x1b[1;1Hnew");
        assert_eq!(grid.row(1), "new");
        assert_eq!(grid.row(2), "");
        // A sequence split between two reads.
        grid.feed("\x1b[2;");
        grid.feed("3Hsplit");
        assert_eq!(grid.row(2), "  split");
    }
}
