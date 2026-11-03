//! Speech to text with whisper.cpp: Vulkan GPU on Windows, Metal on Apple Silicon, CPU otherwise.

use std::path::Path;
use std::sync::Mutex;

use anyhow::{anyhow, Result};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState};

use crate::text::is_hallucination;

pub struct Stt {
    // The state borrows nothing from the context in whisper-rs 0.16, but keep the context alive with it
    _ctx: WhisperContext,
    state: Mutex<WhisperState>,
    vad_model: Option<String>,
    /// Without a GPU, beam search makes every dictation several times slower for a small gain
    greedy: bool,
}

impl Stt {
    /// Loads the model and runs it once so the first dictation isn't slow. `accelerated` is false
    /// on machines with no GPU whisper.cpp can use.
    pub fn load(model: &Path, vad_model: Option<&Path>, accelerated: bool) -> Result<Self> {
        Self::load_on(model, vad_model, true, !accelerated)
    }

    fn load_on(model: &Path, vad_model: Option<&Path>, gpu: bool, greedy: bool) -> Result<Self> {
        let path = model.to_str().ok_or_else(|| anyhow!("model path is not valid UTF-8"))?;
        let mut params = WhisperContextParameters::default();
        params.use_gpu(gpu);
        let ctx = WhisperContext::new_with_params(path, params)?;
        let state = ctx.create_state()?;
        let stt = Self {
            _ctx: ctx,
            state: Mutex::new(state),
            vad_model: vad_model.and_then(|p| p.to_str()).map(String::from),
            greedy,
        };
        stt.run(&vec![0.0; 16_000], Some("en"), "", false)?;
        Ok(stt)
    }

    fn run(&self, audio: &[f32], language: Option<&str>, prompt: &str, vad: bool) -> Result<(String, String)> {
        let strategy = if self.greedy {
            SamplingStrategy::Greedy { best_of: 1 }
        } else {
            SamplingStrategy::BeamSearch { beam_size: 5, patience: -1.0 }
        };
        let mut params = FullParams::new(strategy);
        params.set_language(Some(language.unwrap_or("auto")));
        params.set_initial_prompt(prompt);
        params.set_no_context(true); // don't condition on earlier text
        params.set_suppress_blank(true);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_special(false);
        params.set_print_timestamps(false);
        let threads = std::thread::available_parallelism().map(|n| n.get().min(8)).unwrap_or(4);
        params.set_n_threads(threads as i32);
        if vad {
            if let Some(path) = &self.vad_model {
                // Cuts silence away first, like faster-whisper's vad_filter; stops hallucinations on silence
                params.set_vad_model_path(Some(path));
                params.enable_vad(true);
            }
        }

        let mut state = self.state.lock().unwrap();
        state.full(params, audio)?;
        let text: Vec<String> = state
            .as_iter()
            .filter(|s| s.no_speech_probability() < 0.8)
            .filter_map(|s| s.to_str_lossy().ok().map(|t| t.trim().to_string()))
            .filter(|t| !t.is_empty())
            .collect();
        let lang = whisper_rs::get_lang_str(state.full_lang_id_from_state()).unwrap_or("").to_string();
        Ok((text.join(" "), lang))
    }

    /// Returns (text, detected language code). Empty text when nothing was said.
    pub fn transcribe(&self, audio: &[f32], language: Option<&str>, prompt: &str) -> Result<(String, String)> {
        if audio.len() < 1_600 {
            return Ok((String::new(), String::new()));
        }
        let (text, lang) = self.run(audio, language, prompt, true)?;
        if is_hallucination(&text) {
            return Ok((String::new(), lang));
        }
        Ok((text, lang))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Times every model in PROMPTFLOW_MODELS on a WAV, on the GPU and on the CPU alone:
    /// PROMPTFLOW_BENCH_WAV=speech.wav cargo test --release bench -- --ignored --nocapture
    #[test]
    #[ignore]
    fn bench() {
        let wav = std::env::var("PROMPTFLOW_BENCH_WAV").expect("set PROMPTFLOW_BENCH_WAV");
        let audio = crate::audio::read_wav(Path::new(&wav)).expect("readable WAV");
        let seconds = audio.len() as f64 / 16_000.0;
        let dir = std::path::PathBuf::from(std::env::var("PROMPTFLOW_MODELS").expect("set PROMPTFLOW_MODELS"));
        let only = std::env::var("PROMPTFLOW_BENCH_ONLY").unwrap_or_default();
        for model in crate::catalog::MODELS {
            let path = dir.join(model.file);
            if !path.exists() || (!only.is_empty() && !only.split(',').any(|o| o == model.id)) {
                continue;
            }
            for (gpu, greedy) in [(true, false), (false, false), (false, true)] {
                let stt = Stt::load_on(&path, None, gpu, greedy).unwrap();
                let start = std::time::Instant::now();
                let (text, _) = stt.run(&audio, Some("en"), "", false).unwrap();
                let took = start.elapsed().as_secs_f64();
                let device = match (gpu, greedy) {
                    (true, _) => "gpu",
                    (false, false) => "cpu beam",
                    (false, true) => "cpu greedy",
                };
                let short: String = text.chars().take(90).collect();
                println!("{:32} {device:10} {took:7.2}s  RTF {:6.2}  {short}", model.file, seconds / took);
            }
        }
    }
}
