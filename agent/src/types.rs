use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelInfo {
    pub model: String,
    pub name: String,
    pub efforts: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentInfo {
    pub provider: String,
    pub model: String,
    pub effort: String,
    pub runtime: String,
    pub models: Vec<ModelInfo>,
    pub conversation_id: String,
}
