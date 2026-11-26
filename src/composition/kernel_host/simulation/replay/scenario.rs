//! 시나리오 파일은 seed 대신 실행할 입력과 응답, 가짜 시간을 명시한다.
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use crate::domain::kernel_event::KernelEvent;

pub(super) const SCENARIO_VERSION: u32 = 1;
const MAX_FILE_BYTES: usize = 1024 * 1024;
const MAX_STEPS: usize = 10_000;
const MAX_ELAPSED_MS: u64 = 7 * 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Scenario {
    pub version: u32,
    pub name: String,
    pub max_auto_turns: usize,
    pub steps: Vec<ScenarioStep>,
    pub expected: ScenarioResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum ScenarioStep {
    Command {
        command: ScenarioCommand,
    },
    EffectResponse {
        response: ScenarioResponse,
    },
    /// 시간을 더하고 커널 주기를 한 번 돌린다. 0도 입력함을 처리하는 명시적인 한 주기다.
    AdvanceTime {
        milliseconds: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum ScenarioCommand {
    OpenClient {},
    SubmitTurn {
        prompt: String,
        continue_viewed: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum ScenarioResponse {
    SessionCatalog {
        generation: u64,
    },
    TurnStarted {
        submission: u64,
    },
    TurnCompleted {
        submission: u64,
    },
    TurnFailed {
        submission: u64,
    },
    Evaluation {
        generation: u64,
        thread_id: String,
        completed_turn_id: String,
        skip_auto_follow: bool,
    },
}

/// 벽시계·writer 스레드의 시각은 비교하지 않는다. 커널 판단 사건은 내용과 순서 모두 비교한다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ScenarioResult {
    pub elapsed_ms: u64,
    pub quiet: bool,
    pub events: Vec<KernelEvent>,
}

impl Scenario {
    pub fn from_json(bytes: &[u8]) -> Result<Self> {
        ensure!(bytes.len() <= MAX_FILE_BYTES, "scenario exceeds 1 MiB");
        // 새 버전의 필드가 v1과 다르더라도 버전부터 명확하게 거절한다.
        let value: serde_json::Value =
            serde_json::from_slice(bytes).context("invalid scenario JSON")?;
        ensure!(
            value.get("version").and_then(serde_json::Value::as_u64)
                == Some(u64::from(SCENARIO_VERSION)),
            "unsupported scenario version; expected {SCENARIO_VERSION}"
        );
        let scenario: Self = serde_json::from_value(value).context("invalid v1 scenario")?;
        scenario.validate()?;
        Ok(scenario)
    }

    pub fn load(path: &Path) -> Result<Self> {
        let mut bytes = Vec::new();
        File::open(path)
            .with_context(|| format!("cannot open scenario {}", path.display()))?
            .take(MAX_FILE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .context("cannot read scenario")?;
        Self::from_json(&bytes)
    }

    /// 명시한 새 파일에만 저장한다. 기존 회귀 fixture나 사용자 파일을 덮어쓰지 않는다.
    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        let mut bytes = serde_json::to_vec_pretty(self).context("cannot serialize scenario")?;
        bytes.push(b'\n');
        ensure!(bytes.len() <= MAX_FILE_BYTES, "scenario exceeds 1 MiB");
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .with_context(|| format!("cannot create scenario {}", path.display()))?
            .write_all(&bytes)
            .context("cannot save scenario")
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == SCENARIO_VERSION,
            "unsupported scenario version"
        );
        ensure!(
            !self.name.trim().is_empty(),
            "scenario name must not be empty"
        );
        ensure!(
            !self.steps.is_empty() && self.steps.len() <= MAX_STEPS,
            "scenario must contain 1..={MAX_STEPS} steps"
        );
        ensure!(
            self.max_auto_turns <= 100,
            "scenario auto-follow limit exceeds 100"
        );
        let mut elapsed = 0_u64;
        for step in &self.steps {
            if let ScenarioStep::AdvanceTime { milliseconds } = step {
                elapsed = elapsed
                    .checked_add(*milliseconds)
                    .context("scenario time overflow")?;
                ensure!(
                    elapsed <= MAX_ELAPSED_MS,
                    "scenario time exceeds seven days"
                );
            }
        }
        ensure!(
            self.expected.elapsed_ms == elapsed,
            "expected elapsed time differs from steps"
        );
        Ok(())
    }
}
