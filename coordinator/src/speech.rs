use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{path::PathBuf, process::Stdio, time::Duration};
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
    process: Mutex<Option<Process>>,
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
            process: Mutex::new(None),
        }
    }
    pub async fn info(&self) -> Result<Value, String> {
        match self.run(Job::Info).await? {
            Reply::Info(info) => Ok(info),
            _ => unreachable!(),
        }
    }
    pub async fn transcribe(&self, pcm: Vec<u8>) -> Result<Value, String> {
        match self.run(Job::Transcribe(pcm)).await? {
            Reply::Transcript(value) => Ok(value),
            _ => unreachable!(),
        }
    }
    pub async fn synthesize(
        &self,
        text: String,
        send: mpsc::Sender<Option<Chunk>>,
    ) -> Result<f64, String> {
        match self.run(Job::Synthesize(text, send)).await? {
            Reply::End(ms) => Ok(ms),
            _ => unreachable!(),
        }
    }
    async fn run(&self, job: Job) -> Result<Reply, String> {
        tokio::time::timeout(Duration::from_secs(240), self.run_inner(job))
            .await
            .map_err(|_| "Speech inference timed out; worker will restart")?
    }
    async fn run_inner(&self, job: Job) -> Result<Reply, String> {
        let mut slot = self.process.lock().await;
        // Native inference cannot be safely interrupted in-place. An aborted
        // job owns and drops its process; a later job loads a fresh model worker.
        let mut process = match slot.take() {
            Some(process) => process,
            None => Process::start(&self.config).await?,
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
        if let Some(mut process) = self.process.lock().await.take() {
            process.close().await;
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
    async fn start(config: &SpeechConfig) -> Result<Self, String> {
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
                json!({"protocol":1, "asr_model":config.asr_model, "tts_model":config.tts_model}),
                &[],
            )
            .await?;
        let ready = process.read().await?;
        if ready["type"] != "ready"
            || ready["protocol"] != 1
            || ready["asr"]["provider"] != "qwen3-asr"
            || ready["tts"]["provider"] != "chatterbox-turbo"
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
                if pcm.is_empty() || pcm.len() > 18 * 32000 || !pcm.len().is_multiple_of(2) {
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
                let mut total = 0;
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
                    total += samples;
                    if total > 120 * 24000 {
                        return Err("Synthesized reply exceeds 120 seconds".into());
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
fn duration(value: &Value, key: &str) -> Result<f64, String> {
    value[key]
        .as_f64()
        .filter(|ms| ms.is_finite() && *ms >= 0.)
        .ok_or_else(|| format!("Invalid {key}"))
}
