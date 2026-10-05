# 0005 — Voices: several speech models, a voice library with cloned voices, a voice per kind of line

Date: 2026-10-04. Status: decided by the user (quiz of 2026-10-04); steps 1 and 2 built, the rest in the order below.

## What the user asked for
- Several text-to-speech models, each swappable everywhere: OmniVoice 0.6B, then Chatterbox, Qwen3-TTS 0.6B Base and
  Kokoro-82M (Piper stays). All in Rust (proposal 0006 and later ones per model).
- Voice cloning from samples the owner or admins upload or record, and from any audio they provide. Not from tracked
  people's voices.
- A voice library where cloned voices are kept and from where every voice can be chosen anywhere: per kind of line
  (warnings, strike notices, action notices, greetings, "say now"), per community and per person.

## Design
1. **Voice ids.** `<model>:<name>` for every model; Piper's existing bare ids (`de_DE-thorsten-high`) stay valid and
   mean `piper:<id>`, so settings files and imported data keep working. A cloned voice is `<model>:<library id>`.
2. **What a voice is.** `VoiceInfo` gains `model` and `languages` (every language it speaks: one for a Piper voice,
   many for a multilingual model and its cloned voices). The current `language` stays (the main one).
3. **Several engines.** pb-infer runs each model on its own thread with its own queue (they differ in size, device
   and speed); a speak job goes to the engine that owns the voice; the voice list is the union. Thread counts and
   reloads apply per engine.
4. **Cloning (additive to `TtsEngine` v1).** `clone_voice(sample, sample rate, transcript?) -> ClonedVoice` (the
   model's opaque prompt data: codes, embeddings, reference text) and `add_voice(id, &ClonedVoice)` to register a
   stored one; engines that cannot clone answer `TtsError::NoCloning`.
5. **The voice library.** Events `VoiceAdded {id, name, model, languages, sample (blob), prompt (blob), by}`,
   `VoiceRenamed`, `VoiceRemoved` in the event log; an index table; at start every library voice is registered with
   its engine. Removing a voice that lines use is confirmed and those lines fall back (below).
6. **Choosing.** A new setting `line_voices` (every scope): kind of line → voice id. For a line in language L the bot
   uses the line kind's voice when it speaks L, else `tts_voices[L]`, else the first installed voice for L (today's
   rule). Rendered speech is cached and rendered ahead of time by voice, so nothing else changes.
7. **Web UI.** A Voices page: the installed voices and the library (name, model, languages, play the sample, preview a
   text), "New voice from a sample" (upload or record, pick the model, optional transcript), rename, remove with
   confirmation. The settings show `line_voices` as one picker per kind of line, listing only voices that can be used.

## Order
1. `VoiceInfo.model/languages`, several engines in pb-infer, `line_voices` and its resolution (with tests).
2. The cloning API and the voice library (store, events, index, engine API) with a stand-in cloning engine in tests.
3. The Voices page and the `line_voices` picker (after the web UI's settings restructure).
4. OmniVoice as the first cloning engine (proposal 0006), then the others.

## Step 2 in detail
- **pb-models-api.** `TtsEngine::info() -> TtsInfo {model, cloning}`; default methods `clone_voice(sample, rate,
  transcript) -> ClonedVoice {data: Vec<u8>}`, `add_voice(id, &ClonedVoice) -> VoiceInfo`, `remove_voice(id)`
  (default: `TtsError::NoCloning`); `TtsError::NeedsTranscript` for models that need to know what the sample says.
- **pb-infer.** Clone/add/remove jobs on the model's own queue (cloning at preview priority). The model's thread keeps
  the voices it was given and gives them again after a reload. `speech_models()` lists the models and whether they
  clone.
- **pb-store-api.** `voice.saved` v1 (`VoiceRecord {model, id, name, sample, data, transcript, added_by, by}`; the
  sample as uploaded and the model's data are blobs, role `voice`) and `voice.removed` v1; both audited. `Index::voices()`
  (index schema 4).
- **pb-engine.** `add_voice(file, name, model, transcript)` decodes the file, clones, stores the blobs and the record,
  registers the voice; `rename_voice`, `remove_voice`, `library_voices()`. The id is made from the name once (`anna`,
  `anna-2`) and never changes. At start every library voice is registered with its model (a model that is not
  installed leaves it unusable, and lines fall back).
- **Tests.** The test kit's beep voice learns to clone (its pitch comes from the sample); an engine scenario adds a
  voice, uses it for warnings, restarts, and removes it.
