use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::{Mutex, mpsc},
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpeechConfig {
    pub python: PathBuf,
    pub root: PathBuf,
    pub asr_model: String,
    pub tts_model: String,
    pub cache_path: String,
}

#[derive(Debug)]
pub(crate) struct Chunk {
    pub pcm: Vec<u8>,
    pub generation_ms: f64,
    pub synthesis_ms: f64,
}

pub(crate) struct SpeechRuntime {
    config: SpeechConfig,
    asr: Mutex<Option<Process>>,
    tts: Mutex<Option<Process>>,
}
enum Job {
    Info,
    Transcribe(Vec<u8>),
    Synthesize(String, mpsc::Sender<Option<Chunk>>),
}
enum Reply {
    Info(Value),
    Transcript(Value),
    End(f64),
}

impl SpeechRuntime {
    pub fn new(config: SpeechConfig) -> Self {
        Self {
            config,
            asr: Mutex::new(None),
            tts: Mutex::new(None),
        }
    }
    pub async fn info(&self) -> Result<Value, String> {
        match tokio::try_join!(self.run("asr", Job::Info), self.run("tts", Job::Info))? {
            (Reply::Info(asr), Reply::Info(tts)) => Ok(json!({"asr":asr["asr"], "tts":tts["tts"]})),
            _ => unreachable!(),
        }
    }
    pub async fn transcribe(&self, pcm: Vec<u8>) -> Result<Value, String> {
        match self.run("asr", Job::Transcribe(pcm)).await? {
            Reply::Transcript(value) => Ok(value),
            _ => unreachable!(),
        }
    }
    pub async fn synthesize(
        &self,
        text: String,
        send: mpsc::Sender<Option<Chunk>>,
    ) -> Result<f64, String> {
        // Every piece gets its own inference deadline; an entire reply may be long.
        let mut elapsed = 0.;
        for piece in speech_segments(&text) {
            let (piece_send, mut piece_receive) = mpsc::channel(8);
            let job = self.run("tts", Job::Synthesize(piece, piece_send));
            let forward = async {
                while let Some(Some(chunk)) = piece_receive.recv().await {
                    send.send(Some(chunk))
                        .await
                        .map_err(|_| "Synthesis consumer stopped")?;
                }
                Ok::<(), String>(())
            };
            let (reply, ()) = tokio::try_join!(job, forward)?;
            match reply {
                Reply::End(ms) => elapsed += ms,
                _ => unreachable!(),
            }
        }
        send.send(None)
            .await
            .map_err(|_| "Synthesis consumer stopped")?;
        Ok(elapsed)
    }
    async fn run(&self, role: &str, job: Job) -> Result<Reply, String> {
        tokio::time::timeout(Duration::from_secs(240), self.run_inner(role, job))
            .await
            .map_err(|_| "Speech inference timed out; worker will restart")?
    }
    async fn run_inner(&self, role: &str, job: Job) -> Result<Reply, String> {
        let mut slot = if role == "asr" {
            self.asr.lock().await
        } else {
            self.tts.lock().await
        };
        // Native inference cannot be safely interrupted in-place. An aborted
        // job owns and drops its process; a later job loads a fresh model worker.
        let mut process = match slot.take() {
            Some(process) => process,
            None => Process::start(&self.config, role).await?,
        };
        let result = process.run(job).await;
        if result.is_ok() {
            *slot = Some(process);
        } else {
            process.close().await;
        }
        result
    }
    pub async fn close(&self) {
        for slot in [&self.asr, &self.tts] {
            if let Some(mut process) = slot.lock().await.take() {
                process.close().await;
            }
        }
    }
}

struct Process {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    info: Value,
    next_id: u64,
}

impl Process {
    async fn start(config: &SpeechConfig, role: &str) -> Result<Self, String> {
        let mut command = Command::new(&config.python);
        command
            .args(["-m", "orion_speech_worker.worker"])
            .current_dir(&config.root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        if !config.cache_path.is_empty() {
            command.env("HF_HOME", &config.cache_path).env(
                "HF_HUB_CACHE",
                PathBuf::from(&config.cache_path).join("hub"),
            );
        }
        let mut child = command
            .spawn()
            .map_err(|e| format!("Cannot start speech worker: {e}"))?;
        let input = child.stdin.take().ok_or("Missing speech stdin")?;
        let output = BufReader::new(child.stdout.take().ok_or("Missing speech stdout")?);
        let mut process = Self {
            child,
            input,
            output,
            info: Value::Null,
            next_id: 0,
        };
        process
            .send(
                json!({"protocol":2, "role":role, "asr_model":config.asr_model, "tts_model":config.tts_model}),
                &[],
            )
            .await?;
        let ready = process.read().await?;
        if ready["type"] != "ready"
            || ready["protocol"] != 2
            || ready["role"] != role
            || (role == "asr" && ready["asr"]["provider"] != "qwen3-asr")
            || (role == "tts"
                && ready["tts"]["provider"]
                    != if config.tts_model == "piper-alba-medium"
                        || Path::new(&config.tts_model)
                            .join("en_GB-alba-medium.onnx")
                            .is_file()
                    {
                        "piper-tts"
                    } else {
                        "chatterbox-turbo"
                    })
        {
            return Err("Unsupported speech worker handshake".into());
        }
        process.info = ready;
        Ok(process)
    }
    async fn close(&mut self) {
        let _ = self.child.kill().await;
        let _ = self.child.wait().await;
    }
    async fn send(&mut self, value: Value, pcm: &[u8]) -> Result<(), String> {
        let mut bytes = serde_json::to_vec(&value).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        bytes.extend_from_slice(pcm);
        self.input
            .write_all(&bytes)
            .await
            .map_err(|e| e.to_string())?;
        self.input.flush().await.map_err(|e| e.to_string())
    }
    async fn read(&mut self) -> Result<Value, String> {
        let mut bytes = Vec::new();
        (&mut self.output)
            .take(65537)
            .read_until(b'\n', &mut bytes)
            .await
            .map_err(|e| e.to_string())?;
        if bytes.len() > 65536 || !bytes.ends_with(b"\n") {
            return Err("Speech worker closed or sent an invalid frame".into());
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        if value["type"] == "error" {
            return Err(format!("Speech inference failed: {}", value["message"]));
        }
        Ok(value)
    }
    async fn run(&mut self, job: Job) -> Result<Reply, String> {
        self.next_id += 1;
        let id = self.next_id;
        match job {
            Job::Info => Ok(Reply::Info(self.info.clone())),
            Job::Transcribe(pcm) => {
                if pcm.is_empty() || pcm.len() > 33 * 32000 || !pcm.len().is_multiple_of(2) {
                    return Err("Invalid ASR PCM16".into());
                }
                self.send(
                    json!({"method":"transcribe", "id":id, "bytes":pcm.len()}),
                    &pcm,
                )
                .await?;
                let value = self.read().await?;
                if value["id"] != id || value["type"] != "transcript" || !value["text"].is_string()
                {
                    return Err("Mismatched speech transcription".into());
                }
                Ok(Reply::Transcript(value))
            }
            Job::Synthesize(text, send) => {
                self.send(json!({"method":"synthesize", "id":id, "text":text}), &[])
                    .await?;
                let mut sequence = 0;
                loop {
                    let value = self.read().await?;
                    if value["id"] != id || value["sequence"] != sequence {
                        return Err("Mismatched synthesis sequence".into());
                    }
                    let synthesis_ms = duration(&value, "synthesisMs")?;
                    if value["type"] == "end" {
                        if sequence == 0 {
                            return Err("Synthesis returned no audio".into());
                        }
                        send.send(None)
                            .await
                            .map_err(|_| "Synthesis consumer stopped")?;
                        return Ok(Reply::End(synthesis_ms));
                    }
                    let samples = value["samples"]
                        .as_u64()
                        .ok_or("Invalid synthesis samples")?;
                    if value["type"] != "chunk"
                        || value["sampleRate"] != 24000
                        || !(1..=48000).contains(&samples)
                    {
                        return Err("Invalid synthesis chunk".into());
                    }
                    let generation_ms = duration(&value, "generationMs")?;
                    let mut pcm = vec![0; samples as usize * 2];
                    self.output
                        .read_exact(&mut pcm)
                        .await
                        .map_err(|e| e.to_string())?;
                    send.send(Some(Chunk {
                        pcm,
                        generation_ms,
                        synthesis_ms,
                    }))
                    .await
                    .map_err(|_| "Synthesis consumer stopped")?;
                    sequence += 1;
                }
            }
        }
    }
}
// Match the worker's speech_segments policy: sentences, then words, then Unicode characters.
const MAX_TTS_CHARACTERS: usize = 160;
fn speech_segments(text: &str) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut rest = text.trim();
    while !rest.is_empty() {
        let chars: Vec<_> = rest.char_indices().take(MAX_TTS_CHARACTERS + 1).collect();
        let limit = chars
            .get(MAX_TTS_CHARACTERS)
            .map_or(rest.len(), |(offset, _)| *offset);
        let sentence = chars
            .iter()
            .take(MAX_TTS_CHARACTERS)
            .find_map(|&(offset, ch)| {
                let end = offset + ch.len_utf8();
                (matches!(ch, '.' | '!' | '?') && rest[end..].starts_with(char::is_whitespace))
                    .then_some(end)
            });
        let cut = sentence.unwrap_or_else(|| {
            if limit == rest.len() {
                limit
            } else {
                rest[..limit]
                    .rfind(char::is_whitespace)
                    .filter(|cut| *cut > 0)
                    .unwrap_or(limit)
            }
        });
        pieces.push(rest[..cut].to_owned());
        rest = rest[cut..].trim_start();
    }
    pieces
}

fn duration(value: &Value, key: &str) -> Result<f64, String> {
    value[key]
        .as_f64()
        .filter(|ms| ms.is_finite() && *ms >= 0.)
        .ok_or_else(|| format!("Invalid {key}"))
}

#[cfg(test)]
mod isolation_tests {
    use super::*;
    #[test]
    fn long_unpunctuated_text_is_split_without_losing_words_or_unicode() {
        let text = "something ".repeat(2000);
        let pieces = speech_segments(&text);
        assert!(
            pieces
                .iter()
                .all(|s| s.chars().count() <= MAX_TTS_CHARACTERS)
        );
        assert_eq!(pieces.join(" "), text.trim());
        assert_eq!(
            speech_segments(&"灯".repeat(500)).concat(),
            "灯".repeat(500)
        );
        assert_eq!(
            speech_segments("First sentence. Then another."),
            ["First sentence.", "Then another."]
        );
    }

    #[tokio::test]
    async fn piper_handshake_matches_the_selected_model() {
        let runtime = SpeechRuntime::new(SpeechConfig {
            python: "python3".into(),
            root: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures"),
            asr_model: "fixture".into(),
            tts_model: "piper-alba-medium".into(),
            cache_path: String::new(),
        });
        assert_eq!(
            runtime.info().await.unwrap()["tts"]["provider"],
            "piper-tts"
        );
        runtime.close().await;
    }
    #[tokio::test]
    async fn cancelled_tts_preserves_asr_process() {
        let runtime = SpeechRuntime::new(SpeechConfig {
            python: "python3".into(),
            root: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures"),
            asr_model: "fixture".into(),
            tts_model: "fixture".into(),
            cache_path: String::new(),
        });
        runtime.info().await.unwrap();
        let asr = runtime.asr.lock().await.as_ref().unwrap().child.id();
        let tts = runtime.tts.lock().await.as_ref().unwrap().child.id();
        let (send, _receive) = mpsc::channel(8);
        assert!(
            tokio::time::timeout(
                Duration::from_millis(200),
                runtime.synthesize("hang-tts".into(), send)
            )
            .await
            .is_err()
        );
        assert!(runtime.tts.lock().await.is_none());
        runtime
            .transcribe(b"Still listening!".to_vec())
            .await
            .unwrap();
        assert_eq!(runtime.asr.lock().await.as_ref().unwrap().child.id(), asr);
        runtime.info().await.unwrap();
        assert_ne!(runtime.tts.lock().await.as_ref().unwrap().child.id(), tts);
        runtime.close().await;
    }
}
