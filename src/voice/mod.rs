pub mod types;
pub mod audio;
pub mod vad;
pub mod transcript;
pub mod commands;

#[cfg(feature = "voice-stt-whisper")]
pub mod stt_whisper;

#[cfg(feature = "voice-stt-whisper")]
pub use stt_whisper::WhisperSttProvider;
#[cfg(all(feature = "voice-stt-whisper", feature = "voice-mic"))]
pub use stt_whisper::run_whisper_mic_demo;

pub use audio::{convert_interleaved_to_whisper_pcm, interleaved_to_mono_f32, resample_linear_mono, AudioFormat};
pub use transcript::{sanitize_voice_transcript, DEFAULT_MAX_VOICE_TRANSCRIPT_CHARS};
pub use commands::{map_spoken_phrase, VoiceCommandAction, VoiceCommandConfig};
pub use vad::{VadConfig, VoiceActivityDetector};
pub use types::{
    SpeechToTextProvider, TextToSpeechProvider, VoiceEvent, VoiceEventReceiver,
    VoiceEventSender, VoiceProviderState, VoiceRuntime, VoiceRuntimeHandle,
};
