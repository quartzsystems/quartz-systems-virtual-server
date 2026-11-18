// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The corpus kept between runs: `<dir>/<target>/<hash>`, one input a
//! file, read before a run and written after it. What is written is
//! bounded, so the corpus kept in the repository (`tools/fuzz/corpus`)
//! stays small: the shortest inputs that found something new, at most
//! [`FILES`] of them and [`BYTES`] in all for a target, none longer than
//! [`LONGEST`].

use std::fs;
use std::io;
use std::path::Path;

/// Inputs kept for a target at most.
pub const FILES: usize = 64;
/// Bytes kept for a target at most.
pub const BYTES: usize = 128 << 10;
/// The longest input kept.
pub const LONGEST: usize = 16 << 10;

/// A name from the input's own bytes: the same input, the same file.
pub fn name(input: &[u8]) -> String {
    let mut sum: u64 = 0xCBF2_9CE4_8422_2325;
    for byte in input {
        sum = (sum ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01B3);
    }
    format!("{sum:016x}")
}

/// The inputs kept for `target` under `dir`; none when there are none.
pub fn load(dir: &Path, target: &str) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(dir.join(target)) else {
        return out;
    };
    let mut paths: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        if let Ok(bytes) = fs::read(&path)
            && bytes.len() <= LONGEST
        {
            out.push(bytes);
        }
    }
    out
}

/// The inputs to keep of `inputs`: the shortest first, within the bounds.
pub fn choose(inputs: &[Vec<u8>]) -> Vec<Vec<u8>> {
    let mut sorted: Vec<&Vec<u8>> = inputs.iter().filter(|i| i.len() <= LONGEST).collect();
    sorted.sort_by(|a, b| a.len().cmp(&b.len()).then_with(|| a.cmp(b)));
    sorted.dedup();
    let mut out = Vec::new();
    let mut bytes = 0;
    for input in sorted {
        if out.len() == FILES || bytes + input.len() > BYTES {
            break;
        }
        bytes += input.len();
        out.push(input.clone());
    }
    out
}

/// Keep [`choose`]'s inputs as `target`'s under `dir`, in place of
/// what was there.
pub fn save(dir: &Path, target: &str, inputs: &[Vec<u8>]) -> io::Result<usize> {
    let at = dir.join(target);
    fs::create_dir_all(&at)?;
    let chosen = choose(inputs);
    let names: Vec<String> = chosen.iter().map(|i| name(i)).collect();
    for entry in fs::read_dir(&at)?.flatten() {
        let file = entry.file_name().to_string_lossy().into_owned();
        if !names.contains(&file) {
            fs::remove_file(entry.path())?;
        }
    }
    for (input, file) in chosen.iter().zip(&names) {
        fs::write(at.join(file), input)?;
    }
    Ok(chosen.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shortest_are_kept_within_the_bounds() {
        let mut inputs: Vec<Vec<u8>> = (0..200u8).map(|n| vec![n; usize::from(n) * 10]).collect();
        inputs.push(vec![1; LONGEST + 1]);
        inputs.push(vec![0; 0]);
        let kept = choose(&inputs);
        assert!(kept.len() <= FILES);
        assert!(kept.iter().map(Vec::len).sum::<usize>() <= BYTES);
        assert!(kept.windows(2).all(|w| w[0].len() <= w[1].len()));
        assert!(kept.iter().all(|k| k.len() <= LONGEST));
    }

    #[test]
    fn what_is_saved_is_loaded_and_the_rest_removed() {
        let dir = std::env::temp_dir().join(format!("fuzz-corpus-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        save(&dir, "t", &[b"abc".to_vec(), b"de".to_vec()]).unwrap();
        fs::write(dir.join("t").join("stale"), b"x").unwrap();
        save(&dir, "t", &[b"de".to_vec()]).unwrap();
        assert_eq!(load(&dir, "t"), vec![b"de".to_vec()]);
        assert!(load(&dir, "none").is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }
}
