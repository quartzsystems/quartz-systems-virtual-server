// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The next input: one that was kept, changed in a few places. The
//! changes are the usual ones: a bit, a byte, a number at a boundary,
//! a run copied, cut or filled, a piece of another input.

/// A small deterministic generator: the same seed gives the same run.
pub struct Rng(u64);

impl Rng {
    /// A generator from `seed`.
    pub fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    /// The next number.
    pub fn draw(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A number below `bound`; 0 when `bound` is 0.
    pub fn below(&mut self, bound: usize) -> usize {
        if bound == 0 {
            return 0;
        }
        #[allow(clippy::cast_possible_truncation)] // reduced below a usize
        {
            (self.draw() % bound as u64) as usize
        }
    }
}

/// Numbers at the edges parsers get wrong.
const EDGES: [u64; 12] = [
    0,
    1,
    0x7F,
    0x80,
    0xFF,
    0x7FFF,
    0x8000,
    0xFFFF,
    0x7FFF_FFFF,
    0x8000_0000,
    0xFFFF_FFFF,
    u64::MAX,
];

/// The most bytes an input grows to.
pub const MAX_LEN: usize = 1 << 20;

fn edge(input: &mut [u8], rng: &mut Rng) {
    let width = [1, 2, 4, 8][rng.below(4)];
    if input.len() < width {
        return;
    }
    let at = rng.below(input.len() - width + 1);
    let value = EDGES[rng.below(EDGES.len())];
    let bytes = if rng.below(2) == 0 {
        value.to_le_bytes()
    } else {
        value.to_be_bytes()
    };
    let from = if rng.below(2) == 0 { 0 } else { 8 - width };
    input[at..at + width].copy_from_slice(&bytes[from..from + width]);
}

/// Change `input` once.
fn once(input: &mut Vec<u8>, other: &[u8], rng: &mut Rng) {
    let len = input.len();
    match rng.below(9) {
        0 if len > 0 => {
            let at = rng.below(len);
            input[at] ^= 1 << rng.below(8);
        }
        #[allow(clippy::cast_possible_truncation)] // one byte of it
        1 if len > 0 => {
            let at = rng.below(len);
            input[at] = rng.draw() as u8;
        }
        2 => edge(input, rng),
        #[allow(clippy::cast_possible_truncation)]
        3 if len > 0 => {
            // Add or subtract a little.
            let at = rng.below(len);
            let delta = rng.below(16) as u8 + 1;
            input[at] = if rng.below(2) == 0 {
                input[at].wrapping_add(delta)
            } else {
                input[at].wrapping_sub(delta)
            };
        }
        4 if len > 1 => {
            // Copy a run over another place.
            let n = 1 + rng.below(len.min(64));
            let (from, to) = (rng.below(len - n + 1), rng.below(len - n + 1));
            input.copy_within(from..from + n, to);
        }
        5 if len > 1 => {
            // Cut a run out.
            let n = 1 + rng.below(len.min(64) - 1);
            let at = rng.below(len - n + 1);
            input.drain(at..at + n);
        }
        #[allow(clippy::cast_possible_truncation)]
        6 if len < MAX_LEN => {
            // Put a run in.
            let n = 1 + rng.below(32);
            let at = rng.below(len + 1);
            let fill = rng.draw() as u8;
            input.splice(at..at, std::iter::repeat_n(fill, n));
        }
        #[allow(clippy::cast_possible_truncation)]
        7 if len > 0 => {
            // Fill a run.
            let n = 1 + rng.below(len.min(64));
            let at = rng.below(len - n + 1);
            let fill = [0x00, 0xFF, rng.draw() as u8][rng.below(3)];
            input[at..at + n].fill(fill);
        }
        8 if !other.is_empty() && len > 0 => {
            // A piece of another input, in the same place.
            let n = 1 + rng.below(other.len().min(len).min(256));
            let at = rng.below(other.len().min(len) - n + 1);
            input[at..at + n].copy_from_slice(&other[at..at + n]);
        }
        _ => {}
    }
}

/// A new input from `input`, with `other` to take pieces from.
pub fn mutate(input: &[u8], other: &[u8], rng: &mut Rng) -> Vec<u8> {
    let mut out = input.to_vec();
    for _ in 0..=rng.below(4) {
        once(&mut out, other, rng);
    }
    out.truncate(MAX_LEN);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_seed_gives_the_same_inputs() {
        let seed: Vec<u8> = (0..200u8).collect();
        let run = |n: u64| {
            let mut rng = Rng::new(n);
            let mut input = seed.clone();
            for _ in 0..500 {
                input = mutate(&input, &seed, &mut rng);
            }
            input
        };
        assert_eq!(run(7), run(7));
        assert_ne!(run(7), run(8));
    }

    #[test]
    fn no_input_makes_the_mutator_fail() {
        let mut rng = Rng::new(3);
        let mut input = Vec::new();
        for _ in 0..20_000 {
            input = mutate(&input, &[1, 2, 3], &mut rng);
            assert!(input.len() <= MAX_LEN);
        }
        let mut one = vec![9];
        for _ in 0..20_000 {
            one = mutate(&one, &[], &mut rng);
        }
        assert_eq!(rng.below(0), 0);
    }
}
