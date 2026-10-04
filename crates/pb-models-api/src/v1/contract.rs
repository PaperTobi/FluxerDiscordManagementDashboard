//! Contract tests: every implementation of these interfaces must pass them (call from its own tests).

use std::num::NonZeroUsize;

use super::{Classifier, FRAME, ModelError, SpeakOpts, TtsEngine, TtsError, VadModel};

/// Checks the [`Classifier`] contract on the given speech clip (mono 16 kHz, within the model's limits).
pub fn classifier<C: Classifier>(model: &mut C, speech: &[f32]) {
    let info = model.info().clone();
    assert!(
        info.min_samples >= 1 && info.max_samples > info.min_samples,
        "sane limits: {info:?}"
    );

    let a = model.classify(speech).expect("scores a normal clip");
    for p in a.labels.iter().chain(a.languages.iter()) {
        assert!(
            p.is_finite() && (0.0..=1.0).contains(p),
            "probabilities stay in [0, 1]: {a:?}"
        );
    }
    let total: f32 = a.languages.iter().sum();
    assert!(
        (total - 1.0).abs() < 1e-3,
        "language probabilities sum to 1, got {total}"
    );

    let b = model.classify(speech).expect("scores it again");
    assert_eq!(a, b, "the same clip gives the same scores");

    let too_short = vec![0.0; info.min_samples - 1];
    assert!(
        matches!(model.classify(&too_short), Err(ModelError::TooShort { .. })),
        "too short is an error"
    );
    let too_long = vec![0.0; info.max_samples + 1];
    assert!(
        matches!(model.classify(&too_long), Err(ModelError::TooLong { .. })),
        "too long is an error, never cut"
    );

    model.set_threads(NonZeroUsize::MIN).expect("thread count can change");
    let c = model.classify(speech).expect("scores with one thread");
    for (x, y) in a.labels.iter().zip(c.labels.iter()) {
        assert!(
            (x - y).abs() < 1e-4,
            "thread count does not change results beyond rounding: {x} vs {y}"
        );
    }
}

/// Checks the [`VadModel`] contract: independent streams in one batch give the same results as alone.
pub fn vad<V: VadModel>(model: &mut V, speech: &[f32]) {
    let frames: Vec<[f32; FRAME]> = speech.as_chunks::<FRAME>().0.to_vec();
    assert!(
        frames.len() >= 10,
        "give the contract test at least 10 frames of speech"
    );
    let silence = [0.0f32; FRAME];

    let mut alone = model.new_state();
    let solo: Vec<f32> = frames
        .iter()
        .map(|f| model.step(std::slice::from_ref(f), &mut [&mut alone])[0])
        .collect();
    for p in &solo {
        assert!(p.is_finite() && (0.0..=1.0).contains(p), "probability in [0, 1]: {p}");
    }

    let (mut s1, mut s2) = (model.new_state(), model.new_state());
    for (i, f) in frames.iter().enumerate() {
        let out = model.step(&[*f, silence], &mut [&mut s1, &mut s2]);
        assert_eq!(out.len(), 2);
        assert!(
            (out[0] - solo[i]).abs() < 1e-5,
            "batching does not change a stream's result (frame {i})"
        );
    }
    assert!(model.step(&[], &mut []).is_empty(), "an empty batch is fine");
}

/// Checks the [`TtsEngine`] contract with one of its voices and a sentence in that voice's language.
pub fn tts<T: TtsEngine>(engine: &mut T, voice: &str, sentence: &str) {
    let info = engine
        .voices()
        .into_iter()
        .find(|v| v.id == voice)
        .expect("the voice is listed");
    let speech = engine
        .synthesize(voice, sentence, &SpeakOpts::default())
        .expect("speaks");
    assert_eq!(speech.sample_rate, info.sample_rate);
    let seconds = speech.samples.len() as f32 / speech.sample_rate as f32;
    assert!(seconds > 0.3, "some speech comes out ({seconds} s)");
    assert!(
        speech.samples.iter().all(|s| s.is_finite() && (-1.0..=1.0).contains(s)),
        "samples in [-1, 1]"
    );
    let fast = engine
        .synthesize(
            voice,
            sentence,
            &SpeakOpts {
                rate: 2.0,
                ..SpeakOpts::default()
            },
        )
        .expect("speaks fast");
    assert!(fast.samples.len() < speech.samples.len(), "a higher rate is shorter");
    assert!(matches!(
        engine.synthesize("no-such-voice", sentence, &SpeakOpts::default()),
        Err(TtsError::NoVoice(_))
    ));
    assert!(matches!(
        engine.synthesize(voice, "   ", &SpeakOpts::default()),
        Err(TtsError::Empty)
    ));
}
