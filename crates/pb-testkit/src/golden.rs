//! Reading golden test data (safetensors files written by `tools/golden/oracle.py`, a script now in the history at 18db450): little-endian tensors.

/// Little-endian `f32` values.
pub fn f32s(bytes: &[u8]) -> Vec<f32> {
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect()
}

/// Little-endian `i16` values.
pub fn i16s(bytes: &[u8]) -> Vec<i16> {
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| i16::from_le_bytes(*b))
        .collect()
}

/// Little-endian `i64` values.
pub fn i64s(bytes: &[u8]) -> Vec<i64> {
    bytes
        .as_chunks::<8>()
        .0
        .iter()
        .map(|b| i64::from_le_bytes(*b))
        .collect()
}

/// 16-bit PCM as floats in [-1, 1) (int16 / 32768, as the old bot fed its models).
pub fn pcm_f32(bytes: &[u8]) -> Vec<f32> {
    i16s(bytes).into_iter().map(|s| f32::from(s) / 32768.0).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_little_endian() {
        assert_eq!(f32s(&1.5f32.to_le_bytes()), vec![1.5]);
        assert_eq!(i64s(&(-7i64).to_le_bytes()), vec![-7]);
        assert_eq!(pcm_f32(&(-32768i16).to_le_bytes()), vec![-1.0]);
    }
}
