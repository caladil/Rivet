//! **`std::mt19937`, because that is the generator the screensavers were tuned
//! against.**
//!
//! **Moved here verbatim from `examples/aura/aura-renderer/src/rng.rs`**,
//! `docs/PLAN-Addon-Feature-Set-2026-09-19.md` T6: constant for constant, the
//! same rule the screensavers themselves move under (see `background.rs`'s
//! module doc comment). Nothing below differs from the renderer's copy by a
//! single bit — not the seeding, not the tempering, not the distributions —
//! because `examples/aura/aura-renderer/evidence/T6-background-baseline/`'s
//! hashes were captured against exactly this generator and a difference here
//! would be indistinguishable, from the hash alone, from a difference in the
//! screensavers themselves.
//!
//! `ss_matrix.hpp` and `ss_boxes.hpp` both hold a `std::mt19937` and draw from
//! `std::uniform_real_distribution` and `std::uniform_int_distribution`. The
//! maintainer's constraint on this port is that randomness matches **in
//! distribution and in rate**; it cannot match in sequence, because both C++
//! screensavers seed from `std::random_device` and so does this, in
//! production (`Mt19937::from_the_clock`).
//!
//! # Why not a dependency
//!
//! `ADR-009` keeps this project's dependency graph small on purpose, and
//! `rivet-sdk`'s note about `ring` is the standing argument. Mersenne Twister is
//! forty lines and its constants are published; taking `rand` for it would add a
//! tree to an example.
//!
//! # Where this is NOT the C++
//!
//! `std::uniform_int_distribution` and `std::uniform_real_distribution` are
//! **unspecified** in the standard — libstdc++, libc++ and MSVC each map the
//! generator's output to a range their own way, so "the same distribution" is
//! the most that can be claimed by anybody, including two C++ compilers. The
//! mappings below are the obvious ones, written out rather than hidden:
//!
//! * a real in `[a, b)` from the top 24 bits, which is a `f32` mantissa exactly
//!   and so is uniform over the representable values rather than clumping;
//! * an integer in `[a, b]` by rejection, which is unbiased where a modulo is
//!   not. The bias a modulo introduces is small at these ranges and it is still
//!   not worth having, because it is invisible and permanent.

/// The state of a Mersenne Twister.
pub struct Mt19937 {
    state: [u32; N],
    index: usize,
}

const N: usize = 624;
const M: usize = 397;
const MATRIX_A: u32 = 0x9908_b0df;
const UPPER_MASK: u32 = 0x8000_0000;
const LOWER_MASK: u32 = 0x7fff_ffff;

impl Mt19937 {
    /// Seeded the way `std::mt19937`'s own constructor seeds.
    #[must_use]
    pub fn new(seed: u32) -> Self {
        let mut state = [0u32; N];
        state[0] = seed;
        for i in 1..N {
            state[i] = 1_812_433_253u32
                .wrapping_mul(state[i - 1] ^ (state[i - 1] >> 30))
                .wrapping_add(i as u32);
        }
        Self { state, index: N }
    }

    /// Seeded from the clock, which is what `std::random_device` stands in for
    /// here.
    ///
    /// **A Rivet has no `getrandom` in this application's reach and does not
    /// need one**: nothing here is a secret, and a screensaver seeded from the
    /// same number every boot would still look like rain — it would just be the
    /// same rain, which is only noticeable if two boards are side by side.
    #[must_use]
    pub fn from_the_clock() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos() ^ (d.as_secs() as u32));
        Self::new(nanos | 1)
    }

    /// The next 32 bits.
    pub fn next_u32(&mut self) -> u32 {
        if self.index >= N {
            self.twist();
        }
        let mut y = self.state[self.index];
        self.index += 1;
        y ^= y >> 11;
        y ^= (y << 7) & 0x9d2c_5680;
        y ^= (y << 15) & 0xefc6_0000;
        y ^= y >> 18;
        y
    }

    fn twist(&mut self) {
        for i in 0..N {
            let y = (self.state[i] & UPPER_MASK) | (self.state[(i + 1) % N] & LOWER_MASK);
            let mut next = self.state[(i + M) % N] ^ (y >> 1);
            if y & 1 != 0 {
                next ^= MATRIX_A;
            }
            self.state[i] = next;
        }
        self.index = 0;
    }

    /// `std::uniform_real_distribution<float>(low, high)`.
    pub fn real(&mut self, low: f32, high: f32) -> f32 {
        // 24 bits is an `f32` mantissa, so every value this produces is exactly
        // representable and none is produced twice as often as its neighbour.
        let unit = (self.next_u32() >> 8) as f32 / 16_777_216.0;
        low + (high - low) * unit
    }

    /// `std::uniform_int_distribution<int>(low, high)`, inclusive of both.
    ///
    /// Rejection rather than modulo. `low > high` is not a range and returns
    /// `low` rather than looping for ever — the C++ has undefined behaviour
    /// there and this has an answer.
    pub fn int(&mut self, low: i32, high: i32) -> i32 {
        if low >= high {
            return low;
        }
        let span = (high as i64 - low as i64 + 1) as u64;
        // The largest multiple of `span` that fits in 32 bits. Draws at or above
        // it are thrown away, which is what removes the bias.
        let limit = (u64::from(u32::MAX) + 1) / span * span;
        loop {
            let draw = u64::from(self.next_u32());
            if draw < limit {
                return (low as i64 + (draw % span) as i64) as i32;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;

    /// **The published test vector**, and it is the reason this file is worth
    /// having rather than a plausible-looking shift register.
    ///
    /// `std::mt19937` default-constructed — seed 5489 — produces `3499211612`
    /// first, and C++'s own `[rand.predef]` requires its **ten-thousandth**
    /// consecutive invocation to be `4123659995`. The second number is the one
    /// that actually proves the twist: an implementation can get the first draw
    /// right and still have the recurrence wrong, because the first 624 outputs
    /// come straight out of the seeding loop.
    #[test]
    fn it_is_the_mersenne_twister_the_standard_specifies() {
        let mut rng = Mt19937::new(5489);
        assert_eq!(rng.next_u32(), 3_499_211_612);
        for _ in 1..9_999 {
            rng.next_u32();
        }
        assert_eq!(rng.next_u32(), 4_123_659_995);
    }

    /// Digital Rain's speeds are `uniform_real_distribution(4, 14)` and its
    /// delays `(0, 6)`. A generator that clipped or clumped would make every
    /// column fall at nearly one speed, which is the single most visible thing
    /// about the effect.
    #[test]
    fn reals_cover_their_range_evenly() {
        let mut rng = Mt19937::new(1);
        let mut buckets = [0u32; 10];
        for _ in 0..100_000 {
            let v = rng.real(4.0, 14.0);
            assert!((4.0..14.0).contains(&v), "{v} escaped the range");
            buckets[((v - 4.0) as usize).min(9)] += 1;
        }
        for (i, count) in buckets.iter().enumerate() {
            assert!(
                (8_000..12_000).contains(count),
                "bucket {i} got {count} of 100000, which is not uniform"
            );
        }
    }

    /// Both ends of an integer range must actually occur. Boxes' ages are
    /// `(1, 15)` and its palette index `(0, 7)`; an implementation that never
    /// returned the last entry would silently drop a colour.
    #[test]
    fn integers_include_both_ends_of_the_range() {
        let mut rng = Mt19937::new(7);
        let mut low = false;
        let mut high = false;
        for _ in 0..10_000 {
            let v = rng.int(1, 15);
            assert!((1..=15).contains(&v), "{v} escaped the range");
            low |= v == 1;
            high |= v == 15;
        }
        assert!(low && high, "an end of the range never came up");
        // A degenerate range answers rather than hanging.
        assert_eq!(rng.int(3, 3), 3);
    }
}
