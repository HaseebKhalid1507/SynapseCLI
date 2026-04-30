#[cfg(all(feature = "voice-stt-whisper", feature = "voice-mic"))]
fn main() -> synaps_cli::Result<()> {
    let mut args = std::env::args_os();
    let program = args.next().unwrap_or_else(|| "voice-mic-demo".into());
    let Some(model_path) = args.next() else {
        eprintln!("usage: {} <whisper-model-path> [seconds] [language|auto]", std::path::Path::new(&program).display());
        std::process::exit(2);
    };
    let seconds = args
        .next()
        .and_then(|value| value.to_string_lossy().parse::<u64>().ok())
        .unwrap_or(8);
    let language = args
        .next()
        .map(|value| value.to_string_lossy().to_string())
        .filter(|value| !value.trim().is_empty() && !value.eq_ignore_ascii_case("auto"));

    synaps_cli::run_whisper_mic_demo(std::path::PathBuf::from(model_path), language, seconds)
}

#[cfg(not(all(feature = "voice-stt-whisper", feature = "voice-mic")))]
fn main() {
    eprintln!("voice-mic-demo requires --features voice-stt-whisper,voice-mic");
    std::process::exit(2);
}
