use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PickPrediction {
    Clean,
    AlreadyApplied,
    Conflict { paths: Vec<String> },
    AfterConflict,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PickPreview {
    pub commit: String,
    pub prediction: PickPrediction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PickAction {
    Continue,
    Skip,
    Abort,
}
