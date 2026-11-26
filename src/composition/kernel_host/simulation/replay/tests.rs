use std::fs;
use std::path::{Path, PathBuf};

use super::scenario::{Scenario, ScenarioResponse, ScenarioStep};
use super::{SimRng, replay, replay_with_rng};

const FIXTURE: &[u8] = include_bytes!("fixtures/manual-turn-v1.json");

fn fixture() -> Scenario {
    Scenario::from_json(FIXTURE).expect("checked-in v1 fixture")
}

struct TemporaryScenario(PathBuf);

impl TemporaryScenario {
    fn new() -> Self {
        Self(crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-scenario-{}-{:016x}.json",
            std::process::id(),
            rand::random::<u64>(),
        )))
    }
}

impl Drop for TemporaryScenario {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[test]
fn kernel_simulation_replay_fixture_or_file() {
    let scenario = match std::env::var_os("AKRA_SIM_SCENARIO") {
        Some(path) => Scenario::load(Path::new(&path)).expect("load requested scenario"),
        None => fixture(),
    };
    replay(&scenario).expect("scenario should match its pinned outcome");
    if let Some(path) = std::env::var_os("AKRA_SIM_SCENARIO_SAVE") {
        scenario
            .save(Path::new(&path))
            .expect("save scenario to a new file");
    }
}

#[test]
fn kernel_simulation_replay_does_not_depend_on_seed_or_rng_call_order() {
    let scenario = fixture();
    let expected = replay_with_rng(&scenario, SimRng::new(0)).expect("first replay");
    let mut changed = SimRng::new(u64::MAX);
    for _ in 0..1_000 {
        changed.next_u64();
    }
    let actual = replay_with_rng(&scenario, changed).expect("replay after unrelated random calls");
    assert_eq!(actual, expected);
}

#[test]
fn kernel_simulation_replay_round_trips_a_saved_file_without_overwriting_it() {
    let path = TemporaryScenario::new();
    let original = fixture();
    original.save(&path.0).expect("save a new scenario");
    let loaded = Scenario::load(&path.0).expect("reload scenario");
    assert_eq!(loaded, original);
    assert_eq!(
        replay(&loaded).expect("replay saved file"),
        original.expected
    );
    let original_bytes = fs::read(&path.0).expect("read saved bytes");
    assert!(
        original.save(&path.0).is_err(),
        "existing files must not be overwritten"
    );
    assert_eq!(
        fs::read(&path.0).expect("read unchanged bytes"),
        original_bytes
    );
}

#[test]
fn kernel_simulation_replay_rejects_unknown_versions_before_running() {
    for input in [
        br#"{"version":2,"future_format":true}"#.as_slice(),
        br#"{"version":0}"#.as_slice(),
        br#"{"version":"1"}"#.as_slice(),
        br#"{}"#.as_slice(),
    ] {
        assert!(
            Scenario::from_json(input)
                .unwrap_err()
                .to_string()
                .contains("unsupported scenario version")
        );
    }
}

#[test]
fn kernel_simulation_replay_rejects_unknown_fields_and_broken_json() {
    let mut value: serde_json::Value = serde_json::from_slice(FIXTURE).expect("fixture JSON");
    value["steps"][0]["command"]["unexpected"] = true.into();
    assert!(Scenario::from_json(&serde_json::to_vec(&value).expect("JSON")).is_err());
    assert!(Scenario::from_json(b"{").is_err());
}

#[test]
fn kernel_simulation_replay_rejects_unknown_effect_correlation() {
    let mut scenario = fixture();
    scenario.steps[2] = ScenarioStep::EffectResponse {
        response: ScenarioResponse::SessionCatalog { generation: 99 },
    };
    let error = format!("{:#}", replay(&scenario).unwrap_err());
    assert!(
        error.contains("step 3") && error.contains("no matching session catalog effect"),
        "{error}"
    );
}

#[test]
fn kernel_simulation_replay_rejects_completion_before_start() {
    let mut scenario = fixture();
    scenario.steps[6] = ScenarioStep::EffectResponse {
        response: ScenarioResponse::TurnCompleted { submission: 1 },
    };
    let error = format!("{:#}", replay(&scenario).unwrap_err());
    assert!(
        error.contains("step 7") && error.contains("turn has not started"),
        "{error}"
    );
}

#[test]
fn kernel_simulation_replay_checks_the_pinned_outcome() {
    let mut scenario = fixture();
    scenario.expected.events.clear();
    let error = replay(&scenario).unwrap_err().to_string();
    assert!(error.contains("result mismatch"), "{error}");
}

#[test]
fn kernel_simulation_replay_rejects_clock_overflow_and_io_failures() {
    let mut scenario = fixture();
    scenario.steps.push(ScenarioStep::AdvanceTime {
        milliseconds: u64::MAX,
    });
    assert!(scenario.validate().is_err());
    let missing = TemporaryScenario::new();
    assert!(Scenario::load(&missing.0).is_err());
    assert!(
        fixture()
            .save(&missing.0.join("missing-parent.json"))
            .is_err()
    );
}
