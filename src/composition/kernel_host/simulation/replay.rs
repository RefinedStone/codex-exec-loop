//! A11: 버전 고정 파일을 기존 커널 시뮬레이션에 입력한다. 재생 경로에서는 난수를 호출하지 않는다.
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};

use super::{
    CORE_INPUT_FLUSH_THRESHOLD, FakeTurn, PendingCompletion, ScenarioSettings, SimRng, Simulation,
    TurnStage,
};

mod scenario;
use scenario::{Scenario, ScenarioCommand, ScenarioResponse, ScenarioResult, ScenarioStep};

fn replay(scenario: &Scenario) -> Result<ScenarioResult> {
    replay_with_rng(scenario, SimRng::new(0))
}

fn replay_with_rng(scenario: &Scenario, rng: SimRng) -> Result<ScenarioResult> {
    scenario.validate()?;
    let initial_rng_state = rng.0;
    let mut simulation = Simulation::with_settings(
        rng,
        ScenarioSettings {
            max_auto_turns: scenario.max_auto_turns,
            manual_turn_budget: 0,
            stop_requests: false,
            thread_prompt_budget: 0,
            restarts: false,
            approval_budget: 0,
            decision_budget: 0,
            steer_budget: 0,
            turn_concurrency: 1,
        },
    );
    for (index, step) in scenario.steps.iter().enumerate() {
        simulation.step = index + 1;
        apply_step(&mut simulation, step)
            .with_context(|| format!("scenario '{}' step {}", scenario.name, index + 1))?;
    }
    ensure!(
        simulation.rng.0 == initial_rng_state,
        "fixed scenario replay must not consume random numbers"
    );
    // 숨은 settle이나 난수 선택을 하지 않는다. 미처리 입력이나 효과가 있으면 quiet는 false다.
    let quiet =
        simulation.is_quiet() && simulation.queued_core_inputs == 0 && !simulation.host_woken;
    let elapsed_ms =
        u64::try_from(simulation.elapsed.as_millis()).context("elapsed time overflow")?;
    let log = simulation.kernel_log.clone();
    // 기존 불변식 검사와 event writer의 잔여 기록 처리를 그대로 통과한다.
    simulation.finish(Ok(())).map_err(|(violation, trace)| {
        anyhow::anyhow!(
            "scenario '{}' invariant at step {}: {}\n{}",
            scenario.name,
            violation.step,
            violation.message,
            trace.join("\n")
        )
    })?;
    let events = log
        .0
        .lock()
        .map_err(|_| anyhow::anyhow!("scenario event log poisoned"))?
        .iter()
        .map(|record| record.event.clone())
        .collect();
    let actual = ScenarioResult {
        elapsed_ms,
        quiet,
        events,
    };
    ensure!(
        actual == scenario.expected,
        "scenario '{}' result mismatch\nexpected: {:#?}\nactual: {actual:#?}",
        scenario.name,
        scenario.expected
    );
    Ok(actual)
}

fn apply_step(simulation: &mut Simulation, step: &ScenarioStep) -> Result<()> {
    match step {
        ScenarioStep::AdvanceTime { milliseconds } => simulation
            .run_host_cycle(Duration::from_millis(*milliseconds))
            .map_err(|violation| anyhow::anyhow!(violation.message)),
        ScenarioStep::Command { command } => match command {
            ScenarioCommand::OpenClient {} => {
                ensure!(simulation.client.is_none(), "client is already open");
                simulation.open_client_session();
                Ok(())
            }
            ScenarioCommand::SubmitTurn {
                prompt,
                continue_viewed,
            } => {
                ensure!(
                    simulation.client.is_some(),
                    "open a client before submitting a turn"
                );
                ensure!(
                    !continue_viewed || simulation.viewed_thread.is_some(),
                    "there is no viewed thread to continue"
                );
                ensure!(!prompt.trim().is_empty(), "prompt must not be empty");
                simulation.submit_manual_turn_with(prompt.clone(), *continue_viewed);
                Ok(())
            }
        },
        ScenarioStep::EffectResponse { response } => {
            // core 입력함은 bounded다. 임의 파일 때문에 같은 스레드가 send에서 영원히 멈추지 않게 한다.
            ensure!(
                simulation.queued_core_inputs < CORE_INPUT_FLUSH_THRESHOLD,
                "insert advance_time before another response to drain the core inbox"
            );
            apply_response(simulation, response)
        }
    }
}

fn apply_response(simulation: &mut Simulation, response: &ScenarioResponse) -> Result<()> {
    match response {
        ScenarioResponse::SessionCatalog { generation } => {
            let index = simulation
                .pending
                .iter()
                .position(|pending| {
                    matches!(pending, PendingCompletion::SessionCatalog { correlation }
                    if correlation.generation == *generation)
                })
                .context("no matching session catalog effect")?;
            let PendingCompletion::SessionCatalog { correlation } =
                simulation.pending.remove(index)
            else {
                bail!("session catalog effect changed");
            };
            simulation.complete_session_catalog(correlation);
        }
        ScenarioResponse::TurnStarted { submission } => {
            let turn = matching_turn(simulation, *submission)?;
            ensure!(
                turn.stage == TurnStage::Submitted,
                "turn has already started"
            );
            simulation.start_turn(turn);
        }
        ScenarioResponse::TurnCompleted { submission } => {
            let turn = matching_turn(simulation, *submission)?;
            ensure!(turn.stage == TurnStage::Started, "turn has not started");
            simulation.finish_turn(&turn);
        }
        ScenarioResponse::TurnFailed { submission } => {
            let turn = matching_turn(simulation, *submission)?;
            simulation.fail_turn(&turn);
        }
        ScenarioResponse::Evaluation {
            generation,
            thread_id,
            completed_turn_id,
            skip_auto_follow,
        } => {
            let index = simulation
                .pending
                .iter()
                .position(|pending| {
                    matches!(pending, PendingCompletion::PostTurnEvaluation { correlation, .. }
                    if correlation.generation == *generation
                    && correlation.thread_id == *thread_id
                    && correlation.completed_turn_id == *completed_turn_id)
                })
                .context("no matching post-turn evaluation effect")?;
            let PendingCompletion::PostTurnEvaluation { correlation, .. } =
                simulation.pending.remove(index)
            else {
                bail!("post-turn evaluation effect changed");
            };
            simulation.complete_evaluation_with(correlation, *skip_auto_follow);
        }
    }
    Ok(())
}

fn matching_turn(simulation: &Simulation, submission: u64) -> Result<FakeTurn> {
    simulation
        .turns
        .iter()
        .find(|turn| turn.correlation.generation == submission)
        .cloned()
        .context("no matching turn submission effect")
}

#[cfg(test)]
mod tests;
