//! The speech model catalog: what can be downloaded, which models suit this machine and these languages,
//! and the downloads themselves.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::hardware::{GpuKind, Hardware};

#[derive(Serialize, Clone, Copy, Debug)]
pub struct Model {
    pub id: &'static str,
    pub name: &'static str,
    pub file: &'static str,
    pub url: &'static str,
    pub size: u64,
    pub sha256: &'static str,
    /// Which entry of benchmarks.json describes it
    pub benchmark: &'static str,
    pub english_only: bool,
}

macro_rules! model {
    ($id:literal, $name:literal, $file:literal, $size:literal, $sha:literal, $bench:literal, $en:literal) => {
        Model {
            id: $id,
            name: $name,
            file: $file,
            url: concat!("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/", $file),
            size: $size,
            sha256: $sha,
            benchmark: $bench,
            english_only: $en,
        }
    };
}

pub const MODELS: &[Model] = &[
    model!("turbo", "Whisper Large v3 Turbo", "ggml-large-v3-turbo-q5_0.bin", 574041195,
        "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2", "turbo", false),
    model!("large-v3", "Whisper Large v3", "ggml-large-v3-q5_0.bin", 1081140203,
        "d75795ecff3f83b5faa89d1900604ad8c780abd5739fae406de19f23ecd98ad1", "large-v3", false),
    model!("medium", "Whisper Medium", "ggml-medium-q5_0.bin", 539212467,
        "19fea4b380c3a618ec4723c3eef2eb785ffba0d0538cf43f8f235e7b3b34220f", "medium", false),
    model!("small", "Whisper Small", "ggml-small-q5_1.bin", 190085487,
        "ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb", "small", false),
    // English-only Small: at least as accurate as Small in English (benchmarked as Small, a cautious estimate)
    model!("small-en", "Whisper Small (English)", "ggml-small.en-q5_1.bin", 190098681,
        "bfdff4894dcb76bbf647d56263ea2a96645423f1669176f4844a1bf8e478ad30", "small", true),
];

/// Silero VAD for whisper.cpp: cuts silence away before transcribing. Downloaded with the first model.
pub const VAD: Model = Model {
    id: "vad",
    name: "Silero VAD",
    file: "ggml-silero-v5.1.2.bin",
    url: "https://huggingface.co/ggml-org/whisper-vad/resolve/main/ggml-silero-v5.1.2.bin",
    size: 885098,
    sha256: "29940d98d42b91fbd05ce489f3ecf7c72f0a42f027e4875919a28fb4c04ea2cf",
    benchmark: "",
    english_only: false,
};

pub fn find(id: &str) -> Option<Model> {
    MODELS.iter().copied().find(|m| m.id == id || m.file == id)
}

// ---------------- benchmarks ----------------

#[derive(Deserialize)]
struct Speed {
    /// Real-time factor with Vulkan on a Ryzen 7 4750U's integrated GPU
    accelerated: f32,
    /// Real-time factor on the same machine's 8-core CPU
    cpu: f32,
    /// Real-time factor with PromptFlow's own build on an RTX 4060 Ti (discrete GPU reference)
    #[serde(default)]
    gpu: Option<f32>,
}

#[derive(Deserialize)]
struct Benchmark {
    /// FLEURS word error rate in percent, by language code
    error: HashMap<String, f32>,
    speed: Speed,
}

#[derive(Deserialize)]
struct Benchmarks {
    models: HashMap<String, Benchmark>,
}

fn benchmarks() -> &'static Benchmarks {
    static DATA: OnceLock<Benchmarks> = OnceLock::new();
    DATA.get_or_init(|| serde_json::from_str(include_str!("benchmarks.json")).expect("benchmarks.json"))
}

/// Codes of every language the Whisper models were measured on, sorted.
pub fn languages() -> Vec<String> {
    let mut codes: Vec<String> = benchmarks().models["turbo"].error.keys().cloned().collect();
    codes.sort();
    codes
}

/// Seconds a typical 10-second dictation takes to transcribe on this machine.
pub fn estimated_wait(model: &Model, hw: &Hardware) -> f32 {
    let speed = &benchmarks().models[model.benchmark].speed;
    // Scale the reference machine's numbers to this one. The discrete-GPU factor is calibrated on an
    // RTX 4060 Ti (Turbo: ~0.35 s for 5 s of speech); Apple Silicon is a conservative guess.
    let cpu = speed.cpu * (hw.cpu_cores as f32 / 8.0).clamp(0.3, 2.0) * if hw.cpu_fast_math { 1.0 } else { 0.4 };
    // Our own measurement on a discrete GPU, or the integrated-GPU number scaled up
    let gpu = speed.gpu.unwrap_or(speed.accelerated * 4.5);
    let rtf = match hw.gpu_kind {
        GpuKind::Discrete if hw.vram_gb >= 8.0 => gpu,
        GpuKind::Discrete => gpu * 0.6,
        GpuKind::Apple if hw.ram_gb >= 32.0 => gpu * 0.8, // usually a Pro / Max chip
        GpuKind::Apple => gpu * 0.5,
        GpuKind::Integrated => speed.accelerated.max(cpu),
        GpuKind::None => cpu,
    };
    10.0 / rtf.max(0.01)
}

/// Average word error rate over `languages`, or None if the model doesn't handle one of them.
pub fn error_rate(model: &Model, languages: &[String]) -> Option<f32> {
    if languages.is_empty() || (model.english_only && languages.iter().any(|l| l != "en")) {
        return None;
    }
    let errors = &benchmarks().models[model.benchmark].error;
    let values: Option<Vec<f32>> = languages.iter().map(|l| errors.get(l).copied()).collect();
    values.map(|v| v.iter().sum::<f32>() / v.len() as f32)
}

/// Whether the model fits in this machine's memory with room to spare.
pub fn fits(model: &Model, hw: &Hardware) -> bool {
    let gb = model.size as f32 / 1_073_741_824.0;
    match hw.gpu_kind {
        GpuKind::Discrete => gb * 1.5 < hw.vram_gb,
        _ => gb * 2.5 < hw.ram_gb,
    }
}

#[derive(Serialize, Debug, Clone)]
pub struct Estimate {
    pub id: &'static str,
    /// Seconds for a 10-second dictation on this machine
    pub wait: f32,
    /// Average error rate in the chosen languages; None if a language isn't supported
    pub error: Option<f32>,
    pub fits: bool,
}

#[derive(Serialize, Debug)]
pub struct Recommendation {
    pub recommended: Option<&'static str>,
    pub faster: Option<&'static str>,
    pub more_accurate: Option<&'static str>,
    pub estimates: Vec<Estimate>,
    /// Ollama model for the AI cleanup, or None if this machine is better off in Raw mode
    pub ai_model: Option<&'static str>,
}

/// A dictation that takes longer than this feels slow
const COMFORTABLE_WAIT: f32 = 1.5;

/// Lower is better: mistakes plus waiting. A second of waiting weighs like 1.5 points of error rate,
/// and every second beyond a comfortable wait three times more.
fn score(e: &Estimate) -> f32 {
    e.error.unwrap_or(f32::MAX) + 1.5 * e.wait + 3.0 * (e.wait - COMFORTABLE_WAIT).max(0.0)
}

fn error_of(e: &Estimate) -> f32 {
    e.error.unwrap_or(f32::MAX)
}

/// Picks models for this machine and these languages, from measured accuracy and speed.
pub fn recommend(hw: &Hardware, languages: &[String]) -> Recommendation {
    let estimates: Vec<Estimate> = MODELS
        .iter()
        .map(|m| Estimate { id: m.id, wait: estimated_wait(m, hw), error: error_rate(m, languages), fits: fits(m, hw) })
        .collect();
    let usable: Vec<&Estimate> = estimates.iter().filter(|e| e.fits && e.error.is_some()).collect();

    // Best balance among the models that answer within a few seconds; the fastest one if none does
    let recommended = usable
        .iter()
        .filter(|e| e.wait <= 4.0)
        .min_by(|a, b| score(a).total_cmp(&score(b)))
        .or_else(|| usable.iter().min_by(|a, b| a.wait.total_cmp(&b.wait)))
        .copied();
    let (faster, more_accurate) = match recommended {
        Some(rec) => {
            // Noticeably quicker, without making many more mistakes
            let faster = usable
                .iter()
                .filter(|e| e.wait < rec.wait * 0.6 && error_of(e) < error_of(rec) + 8.0)
                .min_by(|a, b| score(a).total_cmp(&score(b)));
            // Fewer mistakes, if the wait stays bearable
            let more_accurate = usable
                .iter()
                .filter(|e| error_of(e) < error_of(rec) - 0.3 && e.wait <= 8.0)
                .min_by(|a, b| error_of(a).total_cmp(&error_of(b)));
            (faster.map(|e| e.id), more_accurate.map(|e| e.id))
        }
        None => (None, None),
    };

    let ai_model = match hw.gpu_kind {
        GpuKind::Discrete if hw.vram_gb >= 12.0 => Some("gemma3:12b"),
        GpuKind::Apple if hw.ram_gb >= 24.0 => Some("gemma3:12b"),
        GpuKind::Discrete if hw.vram_gb >= 6.0 => Some("gemma3:4b"),
        _ if hw.ram_gb >= 16.0 => Some("gemma3:4b"),
        _ => None,
    };
    Recommendation { recommended: recommended.map(|e| e.id), faster, more_accurate, estimates, ai_model }
}

/// Downloads a model into `dir`, resuming a partial download, and checks its SHA256.
/// `progress(done, total)` is called about ten times a second.
pub fn download(model: &Model, dir: &Path, cancel: &AtomicBool, progress: &dyn Fn(u64, u64)) -> Result<()> {
    fs::create_dir_all(dir)?;
    let target = dir.join(model.file);
    if target.exists() {
        return Ok(());
    }
    let part = dir.join(format!("{}.part", model.file));
    let mut have = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    if have > model.size {
        fs::remove_file(&part)?;
        have = 0;
    }

    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(15)))
        .build()
        .into();
    let mut request = agent.get(model.url);
    if have > 0 {
        request = request.header("Range", format!("bytes={have}-"));
    }
    let response = request.call()?;
    let mut file = if response.status() == 206 {
        OpenOptions::new().append(true).open(&part)?
    } else {
        have = 0; // the server ignored the range: start over
        File::create(&part)?
    };

    let mut reader = response.into_body().into_reader();
    let mut buf = vec![0u8; 256 * 1024];
    let mut last = Instant::now();
    loop {
        if cancel.load(Ordering::Relaxed) {
            bail!("cancelled");
        }
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        have += n as u64;
        if last.elapsed() >= Duration::from_millis(100) {
            progress(have, model.size);
            last = Instant::now();
        }
    }
    file.flush()?;
    drop(file);
    progress(have, model.size);

    let hash = sha256_file(&part)?;
    if hash != model.sha256 {
        fs::remove_file(&part)?;
        return Err(anyhow!("the download was corrupted (checksum mismatch), please try again"));
    }
    fs::rename(&part, &target)?;
    Ok(())
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hw(kind: GpuKind, vram: f32, ram: f32, cores: usize) -> Hardware {
        Hardware {
            gpu: "Test GPU".into(),
            gpu_kind: kind,
            vram_gb: vram,
            ram_gb: ram,
            cpu: "Test CPU".into(),
            cpu_cores: cores,
            cpu_fast_math: true,
        }
    }

    fn langs(l: &[&str]) -> Vec<String> {
        l.iter().map(|s| s.to_string()).collect()
    }

    fn estimate<'a>(r: &'a Recommendation, id: &str) -> &'a Estimate {
        r.estimates.iter().find(|e| e.id == id).unwrap()
    }

    /// cargo test --release print_table -- --ignored --nocapture
    #[test]
    #[ignore]
    fn print_table() {
        let machines = [
            ("RTX 4060 Ti 16 GB", hw(GpuKind::Discrete, 16.0, 32.0, 8)),
            ("GTX 1650 4 GB", hw(GpuKind::Discrete, 4.0, 16.0, 4)),
            ("MacBook Air M2 16 GB", hw(GpuKind::Apple, 16.0, 16.0, 8)),
            ("Laptop, Intel iGPU, 8 cores", hw(GpuKind::Integrated, 0.5, 16.0, 8)),
            ("Old laptop, 4 cores, no GPU", hw(GpuKind::None, 0.0, 8.0, 4)),
        ];
        for (name, m) in &machines {
            for l in ["en", "pt", "tr", "ja"] {
                let r = recommend(m, &langs(&[l]));
                let w = |id: Option<&str>| id.map(|id| format!("{id} ({:.1}s)", estimate(&r, id).wait)).unwrap_or("-".into());
                println!("{name:30} {l}: rec={:22} faster={:22} accurate={}", w(r.recommended), w(r.faster), w(r.more_accurate));
            }
        }
    }

    #[test]
    fn every_model_has_benchmarks() {
        for m in MODELS {
            assert!(m.url.ends_with(m.file) && m.sha256.len() == 64, "{}", m.id);
            assert!(benchmarks().models.contains_key(m.benchmark), "{}", m.id);
        }
    }

    #[test]
    fn gaming_pc_gets_turbo_for_common_languages() {
        let pc = hw(GpuKind::Discrete, 16.0, 32.0, 8);
        for l in ["en", "pt", "es", "de", "ja", "tr"] {
            let r = recommend(&pc, &langs(&[l]));
            assert_eq!(r.recommended, Some("turbo"), "{l}");
        }
        // Large v3 is offered only where it's measurably more accurate than Turbo
        assert_eq!(recommend(&pc, &langs(&["tr"])).more_accurate, Some("large-v3"));
        assert_eq!(recommend(&pc, &langs(&["ja"])).more_accurate, None);
        assert!(estimated_wait(&MODELS[0], &pc) < 1.0);
    }

    #[test]
    fn laptop_without_gpu_stays_quick() {
        let laptop = hw(GpuKind::None, 0.0, 8.0, 4);
        let r = recommend(&laptop, &langs(&["en"]));
        assert!(estimate(&r, r.recommended.unwrap()).wait <= 4.0);
        assert_ne!(r.recommended, Some("large-v3")); // far too slow on 4 cores
    }

    #[test]
    fn english_only_models_only_for_english() {
        let pc = hw(GpuKind::None, 0.0, 16.0, 8);
        let pt = recommend(&pc, &langs(&["pt"]));
        assert!(estimate(&pt, "small-en").error.is_none());
        assert_ne!(pt.recommended, Some("small-en"));
        let two = recommend(&pc, &langs(&["en", "tr"]));
        assert!(estimate(&two, "small-en").error.is_none());
    }

    #[test]
    fn big_models_need_memory() {
        let tiny = hw(GpuKind::None, 0.0, 2.0, 2);
        assert!(!fits(&MODELS[1], &tiny)); // Large v3 on 2 GB of RAM
    }
}
