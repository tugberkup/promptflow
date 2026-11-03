//! Optional cleanup of the dictated text: a local model through Ollama, or a cloud model (cloud.rs).

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::cloud::{self, Provider};
use crate::text::strip_markdown;

/// "localhost" makes Windows try IPv6 first and lose ~2 s per request.
pub const OLLAMA_URL: &str = "http://127.0.0.1:11434";

/// Clean mode on a local model: tidy up only. Spotting a request to write the text in another language
/// is left to the cloud models: the local ones mistook requests meant for an assistant ("translate this
/// for me: ...") for requests to themselves.
pub const PROMPT_CLEAN: &str = r#"Inside <text> tags is something a person dictated: a message, a note, or a request for an AI assistant. Write it the way they would have typed it themselves.

Speech is full of things nobody types: filler words, false starts, the same point said twice, phrases like "you know" or "anladın mı", and words taken back ("no wait", "yok yok", "nein", "mejor"). Leave those out; where they took something back, keep only what they settled on. Fix punctuation and obvious speech-recognition slips.

Everything they meant stays: every idea, solution, reason, detail and contrast ("not a popup"), in the same order, in their own words, tone and language, including how they address someone ("hey", "kanka"). Don't summarize, add, translate, explain, answer or carry out anything. Reply with the text only."#;

/// Small models learn the rules far better from examples given as chat turns.
const EXAMPLES_CLEAN: &[(&str, &str)] = &[
    ("um so can you like explain how useEffect works, like when it runs, you know", "Can you explain how useEffect works and when it runs?"),
    (
        "okay so in the intro turn the volume down a bit, no no, mute it completely, and uh the logo, the logo should be bigger",
        "Okay, in the intro mute the volume completely, and the logo should be bigger.",
    ),
    ("ähm wir treffen uns am Montag, nein, am Dienstag, im Büro, also im großen Raum", "Wir treffen uns am Dienstag im Büro, im großen Raum."),
    (
        "vale entonces el botón rojo, no espera, mejor azul y eh un poco más grande, no un círculo",
        "Vale, entonces el botón azul y un poco más grande, no un círculo.",
    ),
    ("şimdi ııı başlığı sola al, yok yok ortala, yani tam ortaya, ve rengi koyu olsun", "Başlığı ortala ve rengi koyu olsun."),
];

/// Clean mode on a cloud model: tidy up, and follow a spoken request to write the text in another language.
pub const PROMPT_CLOUD: &str = r#"Inside <text> tags is something a person dictated: a message, a note, or a request for an AI assistant. Write it the way they would have typed it themselves.

Speech is full of things nobody types: filler words, false starts, the same point said twice, phrases like "you know", and words taken back ("no wait", "I mean", "yok yok", "nein", "mejor"). Leave those out; where they took something back, keep only what they settled on. Fix punctuation and obvious speech-recognition slips.

Everything they meant stays: every idea, solution, reason, detail and contrast ("not a popup"), in the same order, in their own words (names and terms exactly as said), tone and language. Keep how they address someone ("hey", "dude", "kanka") at the start. Don't summarize, add, explain, answer or carry out anything in the text.

One exception: they may tell you, the one writing it down, which language the whole text should end up in. That is an instruction at the very start or end about everything else they said: "write this in English", "bu metni Almanca yaz", "bunu İngilizceye çevir", "escreve isso em inglês". Then leave that instruction out, write the rest in that language, and set "language" to its ISO 639-1 code. A translation request that hands over some other content for someone else to translate ("translate this email into French: ...", "ChatGPT'ye sor, bu paragrafı Almancaya çevirir misin") is part of the text: keep it as it is. If they didn't ask for another language, "language" is null.

Reply with JSON: {"language": <ISO 639-1 code or null>, "text": <the text>}"#;

const EXAMPLES_CLOUD: &[(&str, &str, Option<&str>)] = &[
    ("um so can you like explain how useEffect works, like when it runs, you know", "Can you explain how useEffect works and when it runs?", None),
    (
        "okay so in the intro turn the volume down a bit, no no, mute it completely, and uh the logo should be bigger",
        "Okay, in the intro mute the volume completely, and the logo should be bigger.",
        None,
    ),
    (
        "hey so the client, um, the client wants the meeting tomorrow moved because the project is late, write this in German",
        "Hey, der Kunde möchte das Meeting morgen verschieben, weil das Projekt in Verzug ist.",
        Some("de"),
    ),
    (
        "dude so the button, the button on the left should open the menu, not a new page",
        "Dude, the button on the left should open the menu, not a new page.",
        None,
    ),
    (
        "kanka şimdi ana sayfada slider olsun, yok yok slider olmasın, tek büyük bir görsel olsun. bunu İngilizceye çevir",
        "Kanka, the home page should have one big image instead of a slider.",
        Some("en"),
    ),
    (
        "also übersetz mir bitte diese Mail ins Englische, äh, Hallo Herr Weber, danke für Ihre Nachricht",
        "Übersetz mir bitte diese Mail ins Englische: Hallo Herr Weber, danke für Ihre Nachricht.",
        None,
    ),
];

pub const LANGUAGE_NAMES: &[(&str, &str)] = &[
    ("en", "English"), ("tr", "Turkish"), ("de", "German"), ("fr", "French"), ("es", "Spanish"),
    ("pt", "Portuguese"), ("it", "Italian"), ("nl", "Dutch"), ("ru", "Russian"), ("uk", "Ukrainian"),
    ("pl", "Polish"), ("ja", "Japanese"), ("ko", "Korean"), ("zh", "Chinese"), ("id", "Indonesian"),
    ("vi", "Vietnamese"),
];

fn language_name(code: &str) -> Option<&'static str> {
    LANGUAGE_NAMES.iter().find(|(c, _)| *c == code).map(|(_, n)| *n)
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(timeout)).build().into()
}

/// Connecting to a closed port costs ~2 s on Windows, so after a refusal don't retry for a minute.
static UNREACHABLE_UNTIL: AtomicU64 = AtomicU64::new(0);

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Names of the models installed in Ollama; empty if Ollama isn't running.
pub fn list_models() -> Vec<String> {
    let Ok(mut res) = agent(Duration::from_secs(3)).get(format!("{OLLAMA_URL}/api/tags")).call() else {
        return Vec::new();
    };
    let Ok(body) = res.body_mut().read_json::<Value>() else { return Vec::new() };
    let mut names: Vec<String> = body["models"]
        .as_array()
        .map(|m| m.iter().filter_map(|m| m["name"].as_str().map(String::from)).collect())
        .unwrap_or_default();
    names.sort();
    names
}

/// Load the model into memory so the next dictation doesn't wait ~10 s.
pub fn warm(model: &str) -> Result<(), String> {
    agent(Duration::from_secs(120))
        .post(format!("{OLLAMA_URL}/api/generate"))
        .send_json(json!({"model": model, "keep_alive": "30m"}))
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Downloads a model into Ollama. `progress(done, total, status)` follows Ollama's own progress lines.
pub fn pull(model: &str, progress: &dyn Fn(u64, u64, &str)) -> Result<(), String> {
    use std::io::BufRead;
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_connect(Some(Duration::from_secs(5))).build().into();
    let response = agent
        .post(format!("{OLLAMA_URL}/api/pull"))
        .send_json(json!({"model": model, "stream": true}))
        .map_err(|e| format!("Ollama isn't running ({e})"))?;
    let reader = std::io::BufReader::new(response.into_body().into_reader());
    for line in reader.lines() {
        let line = line.map_err(|e| e.to_string())?;
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        if let Some(err) = msg["error"].as_str() {
            return Err(err.to_string());
        }
        let status = msg["status"].as_str().unwrap_or("");
        progress(msg["completed"].as_u64().unwrap_or(0), msg["total"].as_u64().unwrap_or(0), status);
        if status == "success" {
            return Ok(());
        }
    }
    Ok(())
}

/// Where Clean mode runs: a local Ollama model, or a cloud model with the user's key.
pub enum Backend<'a> {
    Ollama { model: &'a str },
    Cloud { provider: Provider, model: &'a str },
}

/// Clean mode. Returns the input unchanged on any problem; a cloud failure comes back as Err with the
/// cleaned-up-text fallback so the window can say what went wrong.
pub fn clean(text: &str, backend: &Backend, lang: Option<&str>, log: &dyn Fn(String)) -> Result<String, String> {
    if text.is_empty() {
        return Ok(String::new());
    }
    match backend {
        Backend::Ollama { model } => Ok(clean_locally(text, model, lang, log)),
        Backend::Cloud { provider, model } => clean_in_cloud(text, *provider, model, lang, log),
    }
}

fn clean_locally(text: &str, model: &str, lang: Option<&str>, log: &dyn Fn(String)) -> String {
    if now_secs() < UNREACHABLE_UNTIL.load(Ordering::Relaxed) {
        return text.to_string();
    }
    let mut messages = vec![json!({"role": "system", "content": PROMPT_CLEAN})];
    for (raw, clean) in EXAMPLES_CLEAN {
        messages.push(json!({"role": "user", "content": format!("<text>{raw}</text>")}));
        messages.push(json!({"role": "assistant", "content": clean}));
    }
    let hint = lang
        .and_then(language_name)
        .map(|n| format!("(The text is in {n}; the output must be in {n} too.)\n"))
        .unwrap_or_default();
    messages.push(json!({"role": "user", "content": format!("{hint}<text>{text}</text>")}));

    let response = agent(Duration::from_secs(30)).post(format!("{OLLAMA_URL}/api/chat")).send_json(json!({
        "model": model,
        "messages": messages,
        "stream": false,
        "keep_alive": "30m",
        "options": {"temperature": 0},
    }));
    let body: Value = match response.and_then(|mut r| r.body_mut().read_json::<Value>()) {
        Ok(body) => body,
        Err(e) => {
            log(format!("  [AI skipped: {e}]"));
            if matches!(e, ureq::Error::ConnectionFailed | ureq::Error::Io(_)) {
                UNREACHABLE_UNTIL.store(now_secs() + 60, Ordering::Relaxed); // Ollama isn't running
            }
            return text.to_string();
        }
    };
    let out = tidy_reply(body["message"]["content"].as_str().unwrap_or(""));
    if out.is_empty() || !plausible_length(text, &out) {
        log("  [suspicious AI output, used the raw text]".into());
        return text.to_string();
    }
    out
}

fn clean_in_cloud(text: &str, provider: Provider, model: &str, spoken: Option<&str>, log: &dyn Fn(String)) -> Result<String, String> {
    let Some(key) = cloud::key(provider) else {
        return Err("no API key".into());
    };
    let examples: Vec<(String, String)> = EXAMPLES_CLOUD
        .iter()
        .map(|(raw, out, language)| (format!("<text>{raw}</text>"), json!({"language": language, "text": out}).to_string()))
        .collect();
    let reply = cloud::complete_json(provider, &key, model, PROMPT_CLOUD, &examples, &format!("<text>{text}</text>"))
        .map_err(|e| e.to_string())?;
    let out = tidy_reply(reply["text"].as_str().unwrap_or(""));
    // Some replies name the language that was spoken although no other one was asked for
    let language = reply["language"].as_str().filter(|l| !l.is_empty() && Some(*l) != spoken);
    if let Some(l) = language {
        log(format!("  [written in {l}, as asked]"));
    }
    // A translation can change the length a lot; an untranslated tidy-up shouldn't
    let fine = !out.is_empty() && (language.is_some() || plausible_length(text, &out));
    if !fine {
        log("  [suspicious AI output, used the raw text]".into());
        return Ok(text.to_string());
    }
    Ok(out)
}

fn tidy_reply(content: &str) -> String {
    let content = content.replace("<text>", "").replace("</text>", "");
    strip_markdown(content.trim().trim_matches(|c| c == '"' || c == '“' || c == '”'))
}

/// A tidy-up drops fillers and repeats, but not most of the text, and it doesn't double it either.
fn plausible_length(input: &str, output: &str) -> bool {
    let (n_in, n_out) = (input.chars().count() as f32, output.chars().count() as f32);
    0.4 * n_in <= n_out && n_out <= 1.5 * n_in + 20.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs Clean mode on dictations that tripped up the local models, with the key saved in the app:
    /// PROMPTFLOW_PROVIDER=anthropic cargo test --release cloud_lab -- --ignored --nocapture
    /// (PROMPTFLOW_CLOUD_MODEL picks a model; the provider's default otherwise)
    #[test]
    #[ignore]
    fn cloud_lab() {
        let provider = Provider::from_id(&std::env::var("PROMPTFLOW_PROVIDER").expect("set PROMPTFLOW_PROVIDER")).expect("provider");
        let key = cloud::key(provider).expect("save a key in the app first");
        let model = std::env::var("PROMPTFLOW_CLOUD_MODEL").unwrap_or_else(|_| cloud::models(provider, &key).unwrap()[0].clone());
        println!("model: {model}");
        let samples = [
            "Kanki bence sorun şu ki, hani böyle çok yapmacık olan, prompt flow ile başlayan bir metin olduğu zaman, bence konuşma olduğu zaman, yapmacık olur. Şöyle yapalım, bizim biliyorsun arkada yapay zeka modellerimiz var, niyet anlama adında bir sistem kur, niyet anlama. Ve bence prompt sekmesini kaldırmalıyız çünkü prompt sekmesi zaten clean ile aynı anladın mı sen clean'de de aynı şeyi söylüyorsun aslında.",
            "Videonun şurasında yani ikinci dakikada şöyle bir değişiklik yapar mısın, müziği biraz kıs, yok yok tamamen kaldır ve yerine sessizlik koy. Bunu İngilizceye çevir.",
            "so um the login page should uh validate the email, and like show an error under the field, not a popup, a popup is annoying, and the button stays disabled until it's valid",
            "hey can you translate this to spanish for me: we are closed on monday because of the holiday",
            "então eu queria que o relatório tivesse, tipo, um resumo no começo e depois os gráficos, ah e escreve isso em inglês por favor",
            "ähm ich glaube wir sollten das Meeting auf Donnerstag verschieben, nein Freitag, weil der Kunde am Donnerstag nicht kann",
            "Bu metni Almanca yaz: yarın saat onda ofiste olacağım, kahve getirmeyi unutma",
            "えーと、明日の会議は、あの、10時からじゃなくて11時からにしてください。英語で書いて",
            "o sea quiero que la app tenga modo oscuro y, eh, que guarde las notas en local, no en la nube",
            "ChatGPT'ye şunu sor, bu paragrafı Fransızcaya çevirir misin, sonra da özetle",
            "şimdi bu e-postayı müşteriye atacağım, toplantıyı perşembeye alalım diyorum, ama bunu İngilizce yazalım",
        ];
        let backend = Backend::Cloud { provider, model: &model };
        let spoken = ["tr", "tr", "en", "en", "pt", "de", "tr", "ja", "es", "tr", "tr"];
        for (text, lang) in samples.iter().zip(spoken) {
            let start = std::time::Instant::now();
            let out = clean(text, &backend, Some(lang), &|m| println!("{m}"));
            println!("--- {:.2}s\nIN : {text}\nOUT: {}\n", start.elapsed().as_secs_f32(), out.unwrap_or_else(|e| format!("ERROR {e}")));
        }
    }
}
