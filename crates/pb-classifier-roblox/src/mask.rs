//! The attention mask, computed exactly as `inference.py` does for one clip (`run_inference` always passes a mask of
//! ones). Kept as plain vectors: it is tiny and its rules are the subtle part of matching the reference.

/// Mask after the front end: `mask[:, : -(hop - 1) : hop]` of a mask of ones of `samples` length.
pub fn after_frontend(samples: usize, hop: usize) -> Vec<bool> {
    let end = samples.saturating_sub(hop - 1);
    (0..end).step_by(hop).map(|_| true).collect()
}

/// `attn_mask_conv` (kernel 3, stride 2, padding 1) applied to the mask with the model's fixed weights, then `!= 0`.
pub fn after_stride_conv(mask: &[bool], weight: [f32; 3], bias: f32) -> Vec<bool> {
    let n = mask.len();
    let out = (n + 2 - 3) / 2 + 1;
    let at = |j: isize| -> f32 {
        if j < 0 || j as usize >= n {
            0.0
        } else if mask[j as usize] {
            1.0
        } else {
            0.0
        }
    };
    (0..out)
        .map(|i| {
            let base = 2 * i as isize - 1;
            let v = weight[0] * at(base) + weight[1] * at(base + 1) + weight[2] * at(base + 2) + bias;
            v != 0.0
        })
        .collect()
}

/// `mask[:, ::step][:, :len]`.
pub fn every(mask: &[bool], step: usize, len: usize) -> Vec<bool> {
    mask.iter().step_by(step).take(len).copied().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontend_mask_has_one_entry_per_kept_frame() {
        for samples in [201, 4800, 32_000, 32_160, 480_000] {
            assert_eq!(after_frontend(samples, 160).len(), samples / 160, "{samples}");
        }
    }

    #[test]
    fn odd_frame_counts_mask_the_last_token() {
        let w = [0.0, 0.0, 1.0];
        assert_eq!(after_stride_conv(&[true; 4], w, 0.0), vec![true, true]);
        assert_eq!(after_stride_conv(&[true; 5], w, 0.0), vec![true, true, false]);
    }
}
