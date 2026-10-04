//! Invariants of the segmenter and the ring for arbitrary probability sequences and settings.

#![allow(clippy::unwrap_used, clippy::expect_used)] // a panic is how a test fails

use pb_segment::{Event, FlushReason, PcmRing, SegCfg, Segmenter, ms_to_samples};
use proptest::prelude::*;

fn cfg_strategy() -> impl Strategy<Value = SegCfg> {
    (
        0.1f64..0.95,
        0.05f64..0.95,
        96u32..2000,
        32u32..1000,
        0u32..1000,
        0u32..1000,
        200u32..3000,
        2.0f64..29.9,
        320u32..5000,
        32u32..1000,
    )
        .prop_map(|(start, end, minv, gap, pre, tail, endsil, maxc, win, run)| {
            SegCfg {
                start_thr: start,
                end_thr: end,
                min_voiced_ms: minv,
                abort_gap_ms: gap,
                pre_roll_ms: pre,
                tail_ms: tail,
                end_silence_ms: endsil,
                max_clip_s: maxc,
                soft_window_ms: win,
                soft_min_run_ms: run,
            }
            .normalized()
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    #[test]
    fn sentences_are_ordered_bounded_and_unique(cfg in cfg_strategy(), probs in prop::collection::vec(0.0f64..1.0, 0..2000)) {
        let mut next = 0u64;
        let max = ms_to_samples(cfg.max_clip_s * 1000.0);
        let min_voiced = i64::from(cfg.min_voiced_ms);
        let mut seg = Segmenter::new(cfg, || { next += 1; next }, 0);
        let mut events = Vec::new();
        // Sentences cut in the same frame they qualified in: their speech was already longer than the maximum while
        // it was still too little to count (as in the old bot); such a cut ends right there.
        let mut cut_at_open = std::collections::HashSet::new();
        for (k, p) in probs.iter().enumerate() {
            let batch = seg.push(k as i64, *p);
            for ev in &batch {
                if let Event::Cut(c) = ev
                    && batch.iter().any(|e| matches!(e, Event::Open(o) if o.id == c.id))
                {
                    cut_at_open.insert(c.id);
                }
            }
            events.extend(batch);
        }
        events.extend(seg.flush(probs.len() as i64 * 512, FlushReason::End));
        let mut last_end = 0i64;
        let mut seen = std::collections::HashSet::new();
        for ev in &events {
            match ev {
                Event::Cut(c) => {
                    prop_assert!(c.s0 >= last_end, "no overlap: {} < {}", c.s0, last_end);
                    prop_assert!(c.s0 < c.s1);
                    prop_assert!(c.v0 >= c.s0 && c.v1 <= c.s1 && c.v0 <= c.v1, "voiced core inside: {c:?}");
                    prop_assert!(
                        cut_at_open.contains(&c.id) || c.s1 - c.s0 <= max + 512,
                        "at most the maximum length (+1 frame): {c:?}"
                    );
                    prop_assert!(c.voiced_ms >= min_voiced);
                    prop_assert!(seen.insert(c.id), "ids are unique");
                    last_end = c.s1;
                }
                Event::Drop(d) => {
                    prop_assert!(d.s0 >= last_end);
                    prop_assert!(seen.insert(d.id), "ids are unique");
                    last_end = last_end.max(d.s1);
                }
                _ => {}
            }
        }
    }

    #[test]
    fn ring_returns_what_was_appended(chunks in prop::collection::vec(prop::collection::vec(any::<i16>(), 0..700), 0..20), forget in 0u64..5000, a in 0u64..15000, b in 0u64..15000) {
        let mut ring = PcmRing::new(0);
        let mut all: Vec<i16> = Vec::new();
        for c in &chunks {
            ring.append(c);
            all.extend(c);
        }
        ring.forget_before(forget);
        let (s0, s1) = (a.min(b), a.max(b));
        let lo = s0.max(forget.min(all.len() as u64));
        let hi = s1.min(all.len() as u64);
        let want: Vec<i16> = if hi > lo { all[lo as usize..hi as usize].to_vec() } else { Vec::new() };
        prop_assert_eq!(ring.slice(s0, s1), want);
        prop_assert_eq!(ring.end(), all.len() as u64);
    }
}
