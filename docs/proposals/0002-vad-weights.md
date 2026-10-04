# 0002 — Where the Silero VAD weights come from

Date: 2026-10-03. Status: decided (internal detail).

## Finding
silero-vad publishes, at tag v6.2.3, both `silero_vad.onnx` (sha256 1a153a22…, the file the old Python bot ran, from
the silero-vad 6.2.3 wheel) and `silero_vad_16k.safetensors` (sha256 c59271c2…, added for re-implementations, with
`tinygrad_model.py`). They do not hold the same weights: only the STFT basis is identical; every convolution and LSTM
weight differs, and the safetensors network gives different probabilities (e.g. 0.018 vs 0.0017 on the first frame of
the test speech). The architecture is the same: `tinygrad_model.py` with the ONNX file's 16 kHz weights reproduces
the ONNX outputs to 1.7e-6.

## Decision
`crates/pb-vad-silero` follows `tinygrad_model.py` and reads its weights directly from the official
`silero_vad.onnx` (16 kHz branch, chosen by the graph's own `sr` constant) with Burn's pure-Rust ONNX protobuf types
(`onnx-ir`). One official file, pinned by hash, no conversion. Golden tests: probabilities within 1.2e-6 of the ONNX
model on speech, noise and dialogue; 0.17 ms per frame for one stream, 0.034 ms per stream-frame with 30 streams.

Update 2026-10-04: the network is now a hand-written forward pass, and the ONNX file is read with rten-onnx. Same
file and weights, same golden tests (within 4.4e-6); 0.02 ms per frame for one stream.

## How to undo
Another VAD implementation can replace it behind `pb_models_api::VadModel` (same contract tests).
