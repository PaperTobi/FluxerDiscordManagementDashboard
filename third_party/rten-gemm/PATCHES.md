# Local patch to rten-gemm 0.27.0

rten-gemm 0.27.0 from crates.io (MIT OR Apache-2.0, Robert Knight; the matrix multiplication of rten), used through
`[patch.crates-io]` in the workspace `Cargo.toml`. One change, kept small so that it can be proposed upstream; the
results stay bit for bit the same, only the packing of the convolution input is faster.

Why: measured 2026-10-04 on a Ryzen 5 7600X (Zen 4, AVX-512), 2 threads. In the Piper voices (pb-tts-piper) 81 % of
the time was `Conv`, and about 40 % of that was not the matrix multiplication but packing the convolution input:
`Im2Col::pack_block` gathers every element through computed offsets and scalar loads, even where a panel row is
`NR` consecutive input elements (every unit-stride convolution away from the padding, which is nearly all of a
HiFi-GAN decoder's 1-D convolutions). For example the decoder's last stage, `Conv` 32 → 32 channels, kernel 7, over
39 936 samples, took about 10 ms, with the patch about 5 ms; the output convolution (32 → 1 channel) went from about
7 ms to 1–2 ms. Whole sentence (model only, the golden sentence s0, noise scales 0, medians of three interleaved
rounds): en_US-lessac-medium 147 → 95 ms, de_DE-thorsten-medium 157 → 112 ms, en_US-lessac-high 975 → 605 ms. The
voices' audio is unchanged: the golden test against onnxruntime gives the same deviations as before (at most
1.81e-4).

Upstream lists exactly this as an open item: robertknight/rten#1444, "Optimize im2col packing for `Conv` operator …
for `stride=1` convolutions vector loads can be used".

1. `src/im2col.rs`, `Im2Col::pack_block` (f32 path used by every f32 kernel): per column panel, check once whether
   its columns are adjacent patches of one image row (same Y offset, X offsets increasing by 1, which also means the
   innermost image stride is 1). For such a panel, each packed row whose Y offset and first and last X offsets lie
   outside the padding region is copied from the image as one slice (`Im2Col::contiguous_row`, bounds-checked);
   every other row takes the unchanged gather. The values written are the same as the gather's.
2. `src/tests.rs`: `test_gemm_im2col_f32_unit_stride_conv` checks 1-D, dilated 1-D and 2-D unit-stride convolutions
   (panels inside the image, touching the padding and spanning two output rows) and an image whose innermost
   dimension is not contiguous, against a direct convolution, for every available f32 kernel. It fails when the copy
   path writes wrong values. (Tests are not part of the crates.io package's build here; run with the rten repository.)

Checked in the rten repository at tag `rten-gemm-v0.27.0` with the patch applied: `cargo test -p rten-gemm` (32
passed, 5 benchmarks ignored), `cargo test -p rten --lib -- ops::conv` (32 passed), `cargo fmt --check`, no new
clippy warnings.

Upstream: a candidate pull request for robertknight/rten (closes the first item of #1444). rten's AI policy
(AI_POLICY.md) does not accept contributions made by autonomous agents: a person has to review it, open the pull
request and answer for it in their own words.

Drop this copy once rten-gemm releases an equivalent change.
