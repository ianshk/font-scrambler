//! Reversible byte scrambles applied to a WOFF2 buffer.
//!
//! THE SINGLE SOURCE OF TRUTH for the wire format. `web/unscramble.js` is a
//! hand-mirrored copy of exactly these rules — if you change one, change the
//! other and regenerate the sample. The three invariants both sides rely on:
//!
//! 1. A WOFF2 file starts with the 4-byte signature `wOF2`. By default those
//!    4 bytes are left untouched (`preserve_magic`) so the scrambled artefact
//!    still *reports* as "Web Open Font Format 2" to `file`/font tools even
//!    though the payload is unreadable. `region` is everything after (or,
//!    with preserve_magic off, the whole file).
//! 2. All scrambling operates on `region` only, indexed from 0 inside the
//!    region (never by absolute file offset), so the two sides stay aligned
//!    regardless of the magic prefix.
//! 3. Every scheme is its own inverse except `swap`, whose inverse is the
//!    opposite rotation. `xor`, `rev` are involutions.

use anyhow::Result;

/// The 4-byte WOFF2 signature kept in the clear by default.
pub const MAGIC_LEN: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Scheme {
    /// XOR each region byte with an xorshift32 keystream seeded by `key`.
    Xor,
    /// Reverse the region byte order.
    Rev,
    /// Rotate the region left by `len/2` bytes (split and swap halves).
    Swap,
}

impl Scheme {
    pub fn as_str(self) -> &'static str {
        match self {
            Scheme::Xor => "xor",
            Scheme::Rev => "rev",
            Scheme::Swap => "swap",
        }
    }
}

/// Nonzero seed so the xorshift state never collapses to the all-zero fixed
/// point; key 0 is remapped to a constant.
fn seed_of(key: u32) -> u32 {
    if key == 0 { 0x9e37_79b9 } else { key }
}

/// Advance the xorshift32 state and return its low byte. Kept byte-for-byte
/// identical to the JS `keystream()` helper (`>>> 0` mirrors u32 wrapping).
fn keystream_next(state: &mut u32) -> u8 {
    let mut s = *state;
    s ^= s << 13;
    s ^= s >> 17;
    s ^= s << 5;
    *state = s;
    (s & 0xff) as u8
}

fn region_start(preserve_magic: bool) -> usize {
    if preserve_magic { MAGIC_LEN } else { 0 }
}

/// Rotate `buf[start..]` left by `k` (element `i` moves to `i-k`).
fn rotate_left(buf: &mut [u8], start: usize, k: usize) {
    let n = buf.len() - start;
    if n == 0 {
        return;
    }
    let k = k % n;
    buf[start..].rotate_left(k);
}

/// Rotate `buf[start..]` right by `k` — the exact inverse of `rotate_left`.
fn rotate_right(buf: &mut [u8], start: usize, k: usize) {
    let n = buf.len() - start;
    if n == 0 {
        return;
    }
    let k = k % n;
    buf[start..].rotate_right(k);
}

pub fn scramble(buf: &mut [u8], scheme: Scheme, key: u32, preserve_magic: bool) {
    let start = region_start(preserve_magic);
    match scheme {
        Scheme::Xor => {
            let mut state = seed_of(key);
            for b in &mut buf[start..] {
                *b ^= keystream_next(&mut state);
            }
        }
        Scheme::Rev => buf[start..].reverse(),
        // Forward rotation; descramble undoes it with the opposite direction.
        Scheme::Swap => {
            let half = (buf.len() - start) / 2;
            rotate_left(buf, start, half);
        }
    }
}

pub fn descramble(buf: &mut [u8], scheme: Scheme, key: u32, preserve_magic: bool) {
    let start = region_start(preserve_magic);
    match scheme {
        // XOR and reverse are involutions: the same op undoes itself.
        Scheme::Xor => scramble(buf, scheme, key, preserve_magic),
        Scheme::Rev => buf[start..].reverse(),
        Scheme::Swap => {
            let half = (buf.len() - start) / 2;
            rotate_right(buf, start, half);
        }
    }
}

/// Convert a TTF/OTF buffer to WOFF2 (mirrors `converter::to_woff2` in the
/// worker: same compression level / brotli-text-flag defaults).
pub fn to_woff2(sfnt: &[u8]) -> Result<Vec<u8>> {
    woofwoof::compress(sfnt, "", 8, true)
        .ok_or_else(|| anyhow::anyhow!("WOFF2 compression failed"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(scheme: Scheme, key: u32, preserve: bool) {
        let original: Vec<u8> = (0..=255u8).cycle().take(1000).collect();
        let mut buf = original.clone();
        scramble(&mut buf, scheme, key, preserve);
        if preserve {
            // Magic is never touched, so the scrambled file keeps its header.
            assert_eq!(&buf[..MAGIC_LEN], &original[..MAGIC_LEN]);
        }
        descramble(&mut buf, scheme, key, preserve);
        assert_eq!(buf, original, "{scheme:?} preserve={preserve} must round-trip");
    }

    #[test]
    fn all_schemes_round_trip_both_magic_modes() {
        for scheme in [Scheme::Xor, Scheme::Rev, Scheme::Swap] {
            roundtrip(scheme, 0x1234_5678, true);
            roundtrip(scheme, 0x1234_5678, false);
            roundtrip(scheme, 0, true); // key 0 remaps to the nonzero seed
        }
    }

    #[test]
    fn keystream_is_deterministic_and_seed_dependent() {
        let a: Vec<u8> = {
            let mut s = seed_of(42);
            (0..8).map(|_| keystream_next(&mut s)).collect()
        };
        let b: Vec<u8> = {
            let mut s = seed_of(42);
            (0..8).map(|_| keystream_next(&mut s)).collect()
        };
        let c: Vec<u8> = {
            let mut s = seed_of(43);
            (0..8).map(|_| keystream_next(&mut s)).collect()
        };
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn xor_with_wrong_key_does_not_recover() {
        let mut buf = b"hello world, this is font payload bytes".to_vec();
        scramble(&mut buf, Scheme::Xor, 1, false);
        descramble(&mut buf, Scheme::Xor, 2, false);
        assert!(&buf[..] != b"hello world, this is font payload bytes".as_slice());
    }
}
