pub(crate) mod codex;

use crate::{AgentConfig, AgentEvent, AgentInfo};
use std::{future::Future, pin::Pin};
use tokio::sync::mpsc;

type Response<'a> = Pin<Box<dyn Future<Output = Result<String, String>> + Send + 'a>>;
type Shutdown<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

/// Orion owns turn cancellation, memory, personality and validated robot tools.
/// Backends translate those requests to a provider's conversation protocol.
pub(crate) trait AgentBackend: Send {
    fn info(&self) -> AgentInfo;
    fn respond<'a>(
        &'a mut self,
        text: &'a str,
        events: Option<&'a mpsc::Sender<AgentEvent>>,
    ) -> Response<'a>;
    fn close(&mut self) -> Shutdown<'_>;
}

impl AgentBackend for codex::Codex {
    fn info(&self) -> AgentInfo {
        self.info.clone()
    }
    fn respond<'a>(
        &'a mut self,
        text: &'a str,
        events: Option<&'a mpsc::Sender<AgentEvent>>,
    ) -> Response<'a> {
        Box::pin(codex::Codex::respond(self, text, events))
    }
    fn close(&mut self) -> Shutdown<'_> {
        Box::pin(codex::Codex::close(self))
    }
}

pub(crate) async fn connect(config: &AgentConfig) -> Result<Box<dyn AgentBackend>, String> {
    Ok(Box::new(codex::Codex::connect(config).await?))
}
