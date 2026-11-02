//! The speech model catalog: what can be downloaded, what fits this machine, and the downloads themselves.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::hardware::Hardware;

#[derive(Serialize, Clone, Copy, Debug)]
pub struct Model {
    pub id: &'static str,
    pub name: &'static str,
    pub file: &'static str,
    pub url: &'static str,
    pub size: u64,
    pub sha256: &'static str,
    pub english_only: bool,
    /// Word error rate for Turkish on FLEURS, from Handy's published benchmarks
    pub turkish_error: Option<f32>,
    pub description: &'static str,
}

macro_rules! model {
    ($id:literal, $name:literal, $file:literal, $size:literal, $sha:literal, $en:literal, $tr:expr, $desc:literal) => {
        Model {
            id: $id,
            name: $name,
            file: $file,
            url: concat!("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/", $file),
            size: $size,
            sha256: $sha,
            english_only: $en,
            turkish_error: $tr,
            description: $desc,
        }
    };
}

pub const MODELS: &[Model] = &[
    model!("turbo", "Whisper Large v3 Turbo", "ggml-large-v3-turbo-q5_0.bin", 574041195,
        "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2", false, Some(6.97),
        "Very accurate in 99 languages and fast on a GPU. The best choice for most people."),
    model!("large-v3", "Whisper Large v3", "ggml-large-v3-q5_0.bin", 1081140203,
        "d75795ecff3f83b5faa89d1900604ad8c780abd5739fae406de19f23ecd98ad1", false, Some(6.51),
        "The most accurate, a little better than Turbo, but about twice as slow."),
    model!("turbo-q8", "Whisper Large v3 Turbo (higher precision)", "ggml-large-v3-turbo-q8_0.bin", 874188075,
        "317eb69c11673c9de1e1f0d459b253999804ec71ac4c23c17ecf5fbe24e259a1", false, Some(6.97),
        "Turbo with less compression. Hardly any difference in practice; larger download."),
    model!("medium", "Whisper Medium", "ggml-medium-q5_0.bin", 539212467,
        "19fea4b380c3a618ec4723c3eef2eb785ffba0d0538cf43f8f235e7b3b34220f", false, Some(9.35),
        "Good in many languages, slower than Turbo on most machines."),
    model!("small", "Whisper Small", "ggml-small-q5_1.bin", 190085487,
        "ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb", false, Some(15.95),
        "Fast even without a GPU. Fine for English, makes more mistakes in other languages."),
    model!("small-en", "Whisper Small (English)", "ggml-small.en-q5_1.bin", 190098681,
        "bfdff4894dcb76bbf647d56263ea2a96645423f1669176f4844a1bf8e478ad30", true, None,
        "English only. Fast without a GPU and accurate for English."),
    model!("base-en", "Whisper Base (English)", "ggml-base.en-q5_1.bin", 59721011,
        "4baf70dd0d7c4247ba2b81fafd9c01005ac77c2f9ef064e00dcf195d0e2fdd2f", true, None,
        "English only and tiny. For old or very slow computers."),
];

/// Silero VAD for whisper.cpp: cuts silence away before transcribing. Downloaded with the first model.
pub const VAD: Model = Model {
    id: "vad",
    name: "Silero VAD",
    file: "ggml-silero-v5.1.2.bin",
    url: "https://huggingface.co/ggml-org/whisper-vad/resolve/main/ggml-silero-v5.1.2.bin",
    size: 885098,
    sha256: "29940d98d42b91fbd05ce489f3ecf7c72f0a42f027e4875919a28fb4c04ea2cf",
    english_only: false,
    turkish_error: None,
    description: "Voice activity detection",
};

pub fn find(id: &str) -> Option<Model> {
    MODELS.iter().copied().find(|m| m.id == id || m.file == id)
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Recommendation {
    pub recommended: &'static str,
    pub faster: Option<&'static str>,
    pub more_accurate: Option<&'static str>,
    /// One sentence on why, shown above the list
    pub reason: String,
    /// Ollama model for the AI cleanup, or None if this machine is better off in Raw mode
    pub ai_model: Option<&'static str>,
}

/// Picks models for this machine and language (None = auto-detect / several languages).
pub fn recommend(hw: &Hardware, language: Option<&str>) -> Recommendation {
    let english = language == Some("en");
    let ai_model = if (hw.fast_gpu && !hw.apple_silicon && hw.vram_gb >= 12.0) || (hw.apple_silicon && hw.ram_gb >= 24.0) {
        Some("gemma3:12b")
    } else if (hw.fast_gpu && !hw.apple_silicon && hw.vram_gb >= 6.0) || hw.ram_gb >= 16.0 {
        Some("gemma3:4b")
    } else {
        None
    };
    if hw.fast_gpu {
        Recommendation {
            recommended: "turbo",
            faster: Some(if english { "small-en" } else { "small" }),
            more_accurate: (hw.vram_gb >= 4.0 || hw.apple_silicon).then_some("large-v3"),
            reason: format!("Your {} runs large models in well under a second.", hw.gpu),
            ai_model,
        }
    } else if english {
        Recommendation {
            recommended: "small-en",
            faster: Some("base-en"),
            more_accurate: Some("turbo"),
            reason: "No fast GPU found, so a small English model keeps dictation quick.".into(),
            ai_model,
        }
    } else {
        // Small makes too many mistakes outside English; Turbo is worth a few seconds' wait
        Recommendation {
            recommended: "turbo",
            faster: Some("small"),
            more_accurate: None,
            reason: "No fast GPU found: Turbo takes a few seconds per sentence but stays accurate in every language."
                .into(),
            ai_model,
        }
    }
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

    fn hw(gpu: bool, vram: f32, ram: f32, apple: bool) -> Hardware {
        Hardware { gpu: "Test GPU".into(), vram_gb: vram, ram_gb: ram, fast_gpu: gpu, apple_silicon: apple }
    }

    #[test]
    fn every_model_points_at_its_file() {
        for m in MODELS {
            assert!(m.url.starts_with("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/") && m.url.ends_with(m.file), "{}", m.id);
            assert_eq!(m.sha256.len(), 64);
        }
    }

    #[test]
    fn recommendations() {
        let rtx = recommend(&hw(true, 16.0, 32.0, false), Some("tr"));
        assert_eq!((rtx.recommended, rtx.more_accurate, rtx.ai_model), ("turbo", Some("large-v3"), Some("gemma3:12b")));
        let laptop = recommend(&hw(false, 0.5, 8.0, false), Some("en"));
        assert_eq!((laptop.recommended, laptop.ai_model), ("small-en", None));
        let laptop_tr = recommend(&hw(false, 0.5, 16.0, false), Some("tr"));
        assert_eq!((laptop_tr.recommended, laptop_tr.ai_model), ("turbo", Some("gemma3:4b")));
        let m1 = recommend(&hw(true, 16.0, 16.0, true), None);
        assert_eq!((m1.recommended, m1.ai_model), ("turbo", Some("gemma3:4b")));
        for r in [rtx, laptop, laptop_tr, m1] {
            for id in [Some(r.recommended), r.faster, r.more_accurate].into_iter().flatten() {
                assert!(find(id).is_some(), "{id}");
            }
        }
    }
}
