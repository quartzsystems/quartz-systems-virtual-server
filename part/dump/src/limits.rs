// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The dumps kept of one partition: at most a number of them and a
//! size together. A new dump makes room for itself by the oldest going
//! first.

/// One dump as kept: its name, when it was taken, its size.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Kept {
    /// Its file's name.
    pub name: String,
    /// When it was taken, in seconds since the epoch.
    pub time: u64,
    /// The bytes it takes.
    pub bytes: u64,
}

/// The dumps to remove, oldest first, so that `kept` and one more of
/// `adding` bytes are within `most` dumps and `most_bytes`.
pub fn to_remove(kept: &[Kept], adding: u64, most: usize, most_bytes: u64) -> Vec<String> {
    let mut by_age: Vec<&Kept> = kept.iter().collect();
    by_age.sort_by(|a, b| (a.time, &a.name).cmp(&(b.time, &b.name)));
    let mut count = by_age.len() + 1;
    let mut bytes = by_age.iter().map(|k| k.bytes).sum::<u64>() + adding;
    let mut out = Vec::new();
    for k in by_age {
        if count <= most && bytes <= most_bytes {
            break;
        }
        out.push(k.name.clone());
        count -= 1;
        bytes -= k.bytes;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(name: &str, time: u64, bytes: u64) -> Kept {
        Kept {
            name: name.into(),
            time,
            bytes,
        }
    }

    #[test]
    fn the_oldest_go_first_for_the_number_and_for_the_size() {
        let kept = [k("c", 30, 10), k("a", 10, 10), k("b", 20, 10)];
        assert!(to_remove(&kept, 10, 4, 100).is_empty());
        assert_eq!(to_remove(&kept, 10, 3, 100), ["a"]);
        assert_eq!(to_remove(&kept, 10, 1, 100), ["a", "b", "c"]);
        assert_eq!(to_remove(&kept, 25, 10, 50), ["a"]);
        assert_eq!(to_remove(&kept, 35, 10, 50), ["a", "b"]);
        // A dump larger than the limit alone removes every other.
        assert_eq!(to_remove(&kept, 500, 10, 50), ["a", "b", "c"]);
        assert!(to_remove(&[], 1, 1, 1).is_empty());
    }
}
