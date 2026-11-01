//! Deterministic synthetic dictionary data and hand-rolled timing stats.
//!
//! The generator is seeded and branch-free on external state, so every run
//! benchmarks the exact same workload: pinyin keys are built from the
//! engine-core syllable table and hanzi words from index-derived CJK
//! characters ("序号汉字"), with a xorshift64* PRNG for shape (lengths,
//! frequencies). No trait objects, no external crates.

use engine_core::SYLLABLES;

/// xorshift64* step: deterministic, fast, good enough for shape data.
pub fn next_u64(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    *state = x;
    x.wrapping_mul(0x2545_F491_4F6C_DD1D)
}

/// First code point of the CJK Unified Ideographs block.
const CJK_START: u32 = 0x4E00;

/// Size of the hanzi pool drawn from the CJK block.
const CJK_SPAN: u32 = 20_000;

/// Generate `count` TSV lines (`pinyin<TAB>hanzi<TAB>freq`), deterministically.
pub fn synth_tsv(count: usize, seed: u64) -> String {
    let mut state = seed | 1; // xorshift must not start at zero
    let mut out = String::with_capacity(count * 28);
    for index in 0..count {
        let syllable_count = 1 + (next_u64(&mut state) % 4) as usize;
        for syllable in 0..syllable_count {
            if syllable > 0 {
                out.push('\'');
            }
            let pick = next_u64(&mut state) as usize % SYLLABLES.len();
            out.push_str(SYLLABLES[pick]);
        }
        out.push('\t');
        let hanzi_count = 1 + (next_u64(&mut state) % 3) as usize;
        for hanzi in 0..hanzi_count {
            // mix the entry index in so words differ even for equal shapes
            let code = CJK_START
                + ((next_u64(&mut state) as u32).wrapping_add(index as u32 * 7 + hanzi as u32))
                    % CJK_SPAN;
            out.push(char::from_u32(code).unwrap_or('\u{4e00}'));
        }
        out.push('\t');
        out.push_str(&(1 + next_u64(&mut state) % 100_000).to_string());
        out.push('\n');
    }
    out
}

/// Arithmetic mean (0.0 for an empty sample set).
pub fn mean(samples: &[f64]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    samples.iter().sum::<f64>() / samples.len() as f64
}

/// Median (50th percentile) of a copy of `samples`.
pub fn median(samples: &[f64]) -> f64 {
    percentile(samples, 50.0)
}

/// Nearest-rank percentile `p` in 0..=100 of a sorted copy of `samples`.
pub fn percentile(samples: &[f64], p: f64) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).expect("samples are finite"));
    let rank = ((p / 100.0) * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[rank.min(sorted.len() - 1)]
}

/// Format a Duration-like value in microseconds with fixed precision.
pub fn micros(seconds: f64) -> String {
    format!("{:.3}", seconds * 1e6)
}

/// Format a Duration-like value in milliseconds with fixed precision.
pub fn millis(seconds: f64) -> String {
    format!("{:.1}", seconds * 1e3)
}
