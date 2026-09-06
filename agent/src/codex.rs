use crate::{AgentConfig, AgentInfo, ModelInfo, ORION_INSTRUCTIONS, spoken_response};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    time::timeout,
};

const MAX_EVENT_BYTES: u64 = 2 * 1024 * 1024;

pub(crate) struct Codex {
    // Drop the process before releasing its temporary working directory.
    _child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    _workspace: tempfile::TempDir,
    next_id: u64,
    pending: VecDeque<Value>,
    pub info: AgentInfo,
}

fn candidates(config: &AgentConfig) -> Vec<PathBuf> {
    if let Some(path) = &config.codex_bin {
        return vec![path.clone()];
    }
    let mut paths: Vec<_> = [
        "/Applications/Codex.app/Contents/Resources/codex",
        "/Applications/ChatGPT.app/Contents/Resources/codex",
    ]
    .into_iter()
    .map(PathBuf::from)
    .filter(|path| path.is_file())
    .collect();
    paths.push(PathBuf::from("codex"));
    paths
}

impl Codex {
    pub async fn close(&mut self) {
        let _ = self._child.kill().await;
        let _ = self._child.wait().await;
    }

    pub async fn connect(config: &AgentConfig) -> Result<Self, String> {
        let mut failures = Vec::new();
        for path in candidates(config) {
            match timeout(Duration::from_secs(30), Self::start(&path, config)).await {
                Ok(Ok(client)) => return Ok(client),
                Ok(Err(error)) => failures.push(format!("{}: {error}", path.display())),
                Err(_) => failures.push(format!("{}: initialization timed out", path.display())),
            }
        }
        Err(format!(
            "No compatible Codex runtime. {}",
            failures.join("; ")
        ))
    }

    async fn start(path: &Path, config: &AgentConfig) -> Result<Self, String> {
        let workspace = tempfile::Builder::new()
            .prefix("orion-agent-")
            .tempdir()
            .map_err(|e| e.to_string())?;
        let mut child = Command::new(path)
            .arg("app-server")
            .current_dir(workspace.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| e.to_string())?;
        let input = child.stdin.take().ok_or("Codex stdin unavailable")?;
        let output = BufReader::new(child.stdout.take().ok_or("Codex stdout unavailable")?);
        let mut client = Self {
            _child: child,
            input,
            output,
            _workspace: workspace,
            next_id: 0,
            pending: VecDeque::new(),
            info: AgentInfo {
                provider: "codex".into(),
                model: config.model.clone(),
                effort: config.effort.clone(),
                runtime: path.display().to_string(),
                models: vec![],
                conversation_id: String::new(),
            },
        };
        client
            .rpc(
                "initialize",
                json!({"clientInfo": {"name":"orion-agent", "version": env!("CARGO_PKG_VERSION")}}),
            )
            .await?;
        client.send(json!({"method":"initialized"})).await?;
        if client.rpc("account/read", json!({})).await?["account"].is_null() {
            return Err("Codex is not signed in; run codex login".into());
        }
        let mut catalog = Vec::new();
        let mut cursor = Value::Null;
        loop {
            let page = client.rpc("model/list", json!({"cursor":cursor})).await?;
            catalog.extend(
                page["data"]
                    .as_array()
                    .ok_or("Invalid Codex model catalog")?
                    .iter()
                    .cloned(),
            );
            let next = page["nextCursor"].clone();
            if next.is_null() {
                break;
            }
            if next == cursor || catalog.len() > 10_000 {
                return Err("Invalid Codex model pagination".into());
            }
            cursor = next;
        }
        let compatible = catalog.iter().any(|model| {
            model["model"] == config.model
                && model["supportedReasoningEfforts"]
                    .as_array()
                    .is_some_and(|efforts| {
                        efforts
                            .iter()
                            .any(|effort| effort["reasoningEffort"] == config.effort)
                    })
        });
        if !compatible {
            return Err(format!(
                "Runtime does not advertise {} with {} effort",
                config.model, config.effort
            ));
        }
        client.info.models = catalog
            .iter()
            .filter(|model| model["hidden"] != true)
            .map(|model| ModelInfo {
                model: model["model"].as_str().unwrap_or_default().into(),
                name: model["displayName"].as_str().unwrap_or_default().into(),
                efforts: model["supportedReasoningEfforts"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|effort| effort["reasoningEffort"].as_str().map(str::to_owned))
                    .collect(),
            })
            .collect();
        let thread = client
            .rpc(
                "thread/start",
                json!({
                    "model":config.model, "baseInstructions":ORION_INSTRUCTIONS,
                    "approvalPolicy":"never", "sandbox":"read-only", "ephemeral":true,
                    "cwd":client._workspace.path(),
                }),
            )
            .await?;
        client.info.conversation_id = thread["thread"]["id"]
            .as_str()
            .ok_or("Codex returned no conversation ID")?
            .into();
        client.pending.clear();
        Ok(client)
    }

    async fn send(&mut self, message: Value) -> Result<(), String> {
        let mut bytes = serde_json::to_vec(&message).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        self.input
            .write_all(&bytes)
            .await
            .map_err(|e| e.to_string())?;
        self.input.flush().await.map_err(|e| e.to_string())
    }

    async fn read(&mut self) -> Result<Value, String> {
        let mut bytes = Vec::new();
        (&mut self.output)
            .take(MAX_EVENT_BYTES + 1)
            .read_until(b'\n', &mut bytes)
            .await
            .map_err(|e| e.to_string())?;
        if bytes.is_empty() {
            return Err("Codex process closed its output".into());
        }
        if bytes.len() as u64 > MAX_EVENT_BYTES || !bytes.ends_with(b"\n") {
            return Err("Codex event exceeded the size limit or was incomplete".into());
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        // This first extraction has no tool dispatcher or interactive approvals.
        // Never leave an unexpected server request waiting indefinitely.
        if value.get("id").is_some() && value.get("method").is_some() {
            self.send(json!({"id":value["id"], "error":{"code":-32601,
                "message":"Orion does not support interactive server requests"}}))
                .await?;
            return Err("Codex requested an unsupported tool or approval".into());
        }
        Ok(value)
    }

    async fn rpc(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({"id":id, "method":method, "params":params}))
            .await?;
        loop {
            let message = self.read().await?;
            if message["id"] == id {
                if let Some(error) = message.get("error") {
                    return Err(format!("Codex {method}: {error}"));
                }
                return message
                    .get("result")
                    .cloned()
                    .ok_or_else(|| "Invalid Codex response".into());
            }
            if message.get("method").is_some() {
                if self.pending.len() >= 1024 {
                    return Err("Too many pending Codex events".into());
                }
                self.pending.push_back(message);
            }
        }
    }

    pub async fn respond(&mut self, text: &str) -> Result<String, String> {
        self.pending.clear();
        let start = self
            .rpc(
                "turn/start",
                json!({"threadId":self.info.conversation_id,
            "model":self.info.model, "effort":self.info.effort,
            "input":[{"type":"text", "text":text}]}),
            )
            .await?;
        let turn_id = start["turn"]["id"]
            .as_str()
            .ok_or("Codex returned no turn ID")?
            .to_owned();
        let mut final_text = String::new();
        loop {
            let message = match self.pending.pop_front() {
                Some(message) => message,
                None => self.read().await?,
            };
            let params = &message["params"];
            if params["threadId"] != self.info.conversation_id {
                continue;
            }
            if message["method"] == "item/completed"
                && params["turnId"] == turn_id
                && let Some(text) = final_message(&params["item"])
            {
                final_text = text.into();
            }
            if message["method"] == "turn/completed" && params["turn"]["id"] == turn_id {
                if params["turn"]["status"] != "completed" {
                    return Err(format!(
                        "Codex turn did not complete: {}",
                        params["turn"]["error"]
                    ));
                }
                for item in params["turn"]["items"].as_array().into_iter().flatten() {
                    if let Some(text) = final_message(item) {
                        final_text = text.into();
                    }
                }
                return spoken_response(&final_text);
            }
        }
    }
}

fn final_message(item: &Value) -> Option<&str> {
    (item["type"] == "agentMessage" && (item["phase"].is_null() || item["phase"] == "final_answer"))
        .then(|| item["text"].as_str())
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "Requires an installed signed-in Codex runtime; reads catalog and creates an ephemeral thread, without inference"]
    async fn installed_runtime_handshake() {
        let mut client = Codex::connect(&AgentConfig::default()).await.unwrap();
        assert!(!client.info.conversation_id.is_empty());
        assert_eq!(client.info.model, crate::DEFAULT_MODEL);
        client.close().await;
    }
}
