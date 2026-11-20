use jsonschema::Validator;
use serde_json::Value;

const SCHEMA: &str = include_str!("../schema/akra-issue-automation.schema.json");
const USER_EXAMPLE: &str = include_str!("../schema/akra-issue-automation.user.example.toml");
const PROJECT_EXAMPLE: &str = include_str!("../schema/akra-issue-automation.project.example.toml");

fn schema() -> Value {
    serde_json::from_str(SCHEMA).expect("자동화 정책 스키마는 올바른 JSON이어야 한다")
}

fn validator() -> Validator {
    jsonschema::draft202012::options()
        .offline()
        .build(&schema())
        .expect("자동화 정책 스키마는 외부 참조 없이 컴파일되어야 한다")
}

fn raw_toml(source: &str) -> Value {
    // 정책 타입을 먼저 거치면 알 수 없는 필드가 사라질 수 있다. 원시 TOML을 그대로 검사한다.
    let raw: toml::Value = toml::from_str(source).expect("시험 입력은 올바른 TOML이어야 한다");
    serde_json::to_value(raw).expect("원시 TOML은 JSON 값으로 변환되어야 한다")
}

fn assert_valid(validator: &Validator, label: &str, source: &str) {
    let value = raw_toml(source);
    let errors: Vec<_> = validator
        .iter_errors(&value)
        .map(|error| error.to_string())
        .collect();
    assert!(errors.is_empty(), "{label}: {errors:?}");
}

fn assert_invalid(validator: &Validator, label: &str, source: &str) {
    assert!(
        !validator.is_valid(&raw_toml(source)),
        "{label}: 잘못된 부분 정책을 허용했다"
    );
}

#[test]
fn policy_schema_is_valid_draft_2020_12() {
    let schema = schema();
    assert_eq!(
        schema["$schema"],
        "https://json-schema.org/draft/2020-12/schema"
    );
    jsonschema::draft202012::meta::validate(&schema)
        .expect("스키마 자체가 Draft 2020-12 메타 스키마를 통과해야 한다");
    let _ = validator();
}

#[test]
fn shipped_user_and_project_examples_validate_as_raw_toml() {
    let validator = validator();
    assert_valid(&validator, "사용자 예시", USER_EXAMPLE);
    assert_valid(&validator, "저장소 예시", PROJECT_EXAMPLE);
}

#[test]
fn partial_layers_may_inherit_transition_permission() {
    let validator = validator();
    for (label, source) in [
        ("빈 부분 설정", ""),
        ("명시한 스키마 판", "schema_version = 1"),
        ("사용자 허가 상속", "[close]\nwhen = \"integration\""),
        (
            "다른 feedback 값만 덮어쓰기",
            "[feedback]\ncomment = false\n[close]\nwhen = \"integration\"",
        ),
        (
            "같은 파일의 명시한 허가",
            "[feedback]\ntransition = true\n[close]\nwhen = \"integration\"",
        ),
        (
            "외부 provider 연결 유지",
            "[feedback]\ntransition = false\n[close]\nwhen = \"provider_link\"",
        ),
        ("기본 닫힘 정책 상속", "[feedback]\ntransition = false"),
    ] {
        assert_valid(&validator, label, source);
    }
}

#[test]
fn an_explicit_transition_denial_conflicts_with_integration_close() {
    assert_invalid(
        &validator(),
        "같은 부분 설정의 닫힘 충돌",
        "[feedback]\ntransition = false\n[close]\nwhen = \"integration\"",
    );
}

#[test]
fn unknown_keys_are_rejected_without_typed_deserialization() {
    let validator = validator();
    for (label, source) in [
        ("루트", "unknown = true"),
        ("binding", "[binding]\nunknown = true"),
        ("GitHub binding", "[binding.github]\nlogin = \"maintainer\""),
        ("authority", "[authority]\nautonomy = \"auto\""),
        ("select", "[select]\nlabels_all = [\"safe\"]"),
        ("classify", "[classify]\nunknown = []"),
        (
            "분류 규칙",
            "[[classify.rule]]\nclass = \"low\"\nlabels_any = [\"docs\"]\nunknown = true",
        ),
        ("gates의 수준", "[gates.observe.low]\nstart = \"human\""),
        (
            "gates의 위험 등급",
            "[gates.auto.critical]\nstart = \"human\"",
        ),
        ("gates의 관문", "[gates.auto.low]\nclose = \"human\""),
        ("timeouts", "[timeouts]\ndelivery = \"1h\""),
        ("budget", "[budget]\nmax_cost = 1"),
        ("feedback", "[feedback]\nassign = true"),
        ("close", "[close]\nintegration = true"),
        ("protection", "[protection]\nfiles = []"),
        ("human_required", "[human_required]\nenabled = false"),
        (
            "추가 사람 필수 규칙",
            "[[human_required.additional]]\nlabels_any = [\"manual\"]\ngates = [\"start\"]\nid = \"custom\"",
        ),
    ] {
        assert_invalid(&validator, label, source);
    }
}

#[test]
fn invalid_versions_enums_and_boolean_types_are_rejected() {
    let validator = validator();
    for (label, source) in [
        ("지원하지 않는 판", "schema_version = 2"),
        ("문자열 판", "schema_version = \"1\""),
        ("바인딩 수준", "[binding.github]\nmode = \"automatic\""),
        ("자율 상한", "[authority]\nautonomy_cap = \"unrestricted\""),
        ("선택 방식", "[select]\nmode = \"labels_all\""),
        (
            "위험 등급",
            "[[classify.rule]]\nclass = \"critical\"\nlabels_any = [\"security\"]",
        ),
        ("관문 처리", "[gates.auto.low]\nstart = \"skip\""),
        ("닫힘 방식", "[close]\nwhen = \"success\""),
        ("문자열 허가", "[feedback]\ntransition = \"true\""),
        (
            "사람 필수 규칙",
            "[human_required]\nrules = [\"allow_all\"]",
        ),
        (
            "추가 규칙의 관문",
            "[[human_required.additional]]\nlabels_any = [\"manual\"]\ngates = [\"execute\"]",
        ),
    ] {
        assert_invalid(&validator, label, source);
    }
}

#[test]
fn gate_policies_preserve_human_only_boundaries() {
    let validator = validator();
    for (mode, gate, value) in [
        ("propose", "start", "auto"),
        ("propose", "start", "timeout_default"),
        ("propose", "delivery", "auto"),
        ("propose", "merge", "auto"),
        ("gated", "delivery", "timeout_arbiter"),
        ("gated", "merge", "auto"),
        ("auto", "spec", "auto"),
        ("auto", "spec", "timeout_arbiter"),
        ("auto", "plan", "auto"),
        ("auto", "plan", "timeout_default"),
        ("auto", "merge", "timeout_default"),
        ("auto", "merge", "timeout_arbiter"),
    ] {
        for risk in ["low", "normal", "high"] {
            assert_invalid(
                &validator,
                &format!("{mode}/{risk}/{gate}={value}"),
                &format!("[gates.{mode}.{risk}]\n{gate} = \"{value}\""),
            );
        }
    }
    for (mode, gate, value) in [
        ("propose", "start", "human"),
        ("propose", "plan", "timeout_arbiter"),
        ("gated", "start", "auto"),
        ("gated", "start", "timeout_default"),
        ("gated", "plan", "timeout_arbiter"),
        ("gated", "delivery", "human"),
        ("auto", "spec", "human"),
        ("auto", "delivery", "timeout_default"),
        ("auto", "merge", "auto"),
    ] {
        assert_valid(
            &validator,
            &format!("{mode}/{gate}={value}"),
            &format!("[gates.{mode}.normal]\n{gate} = \"{value}\""),
        );
    }
}

#[test]
fn timeouts_accept_canonical_units_and_reject_out_of_range_values() {
    let validator = validator();
    for value in ["1s", "30d", "720h", "43200m", "2592000s"] {
        assert_valid(
            &validator,
            value,
            &format!("[timeouts]\nstart = \"{value}\""),
        );
    }
    for value in ["0s", "0m", "0h", "0d", "30d", "2592000s"] {
        assert_valid(
            &validator,
            value,
            &format!("[timeouts]\nremind_before = \"{value}\""),
        );
    }
    for value in [
        "0s", "0m", "0h", "0d", "2592001s", "43201m", "721h", "31d", "01h", "+1h", "-1h", "1.5h",
        "1 h", "1H", " 1h", "1h ", "1",
    ] {
        assert_invalid(
            &validator,
            value,
            &format!("[timeouts]\nstart = \"{value}\""),
        );
    }
    for value in ["00s", "31d", "-1s", "2.5h"] {
        assert_invalid(
            &validator,
            value,
            &format!("[timeouts]\nremind_before = \"{value}\""),
        );
    }
    // 서로 다른 leaf와 단위의 크기 비교는 최종 병합 서비스가 담당한다.
    assert_valid(
        &validator,
        "병합 전 시간값",
        "[timeouts]\nstart = \"1h\"\nremind_before = \"2h\"",
    );
}

#[test]
fn budgets_accept_zero_and_u32_maximum_but_no_overflow_or_wrong_type() {
    let validator = validator();
    for key in ["max_running", "max_started_per_day", "max_attempts"] {
        for value in ["0", "4294967295"] {
            assert_valid(
                &validator,
                &format!("{key}={value}"),
                &format!("[budget]\n{key} = {value}"),
            );
        }
        for value in ["-1", "4294967296", "1.5", "\"2\"", "true"] {
            assert_invalid(
                &validator,
                &format!("{key}={value}"),
                &format!("[budget]\n{key} = {value}"),
            );
        }
    }
}

#[test]
fn trusted_actors_require_the_full_stable_identity_and_reject_login_keys() {
    let validator = validator();
    assert_valid(
        &validator,
        "안정 ID 삼중항",
        r#"
[select]
trusted_actors = [
  { provider = "github", server_id = "github.com", actor_id = "U_example_AbC=" },
  { provider = "jira", server_id = "Example Server", actor_id = "Opaque ID AbC" },
]
"#,
    );
    for actor in [
        r#""maintainer-login""#,
        r#"{ provider = "github", server_id = "github.com" }"#,
        r#"{ provider = "github", server_id = "github.com", login = "maintainer" }"#,
        r#"{ provider = "github", server_id = "github.com", actor_id = "U_example", login = "maintainer" }"#,
        r#"{ provider = "gitlab", server_id = "gitlab.com", actor_id = "opaque" }"#,
        r#"{ provider = "github", server_id = "", actor_id = "U_example" }"#,
        r#"{ provider = "github", server_id = " github.com", actor_id = "U_example" }"#,
        r#"{ provider = "github", server_id = "github.com", actor_id = "U_example " }"#,
        r#"{ provider = "github", server_id = "github.com", actor_id = "\u2003U_example" }"#,
        r#"{ provider = "github", server_id = "github.com", actor_id = "U_\u0085example" }"#,
    ] {
        assert_invalid(
            &validator,
            actor,
            &format!("[select]\ntrusted_actors = [{actor}]"),
        );
    }
}

#[test]
fn empty_selection_and_restriction_lists_remain_meaningful_partial_values() {
    let validator = validator();
    for source in [
        "[select]\nmode = \"labels_any\"\nlabels_any = []",
        "[select]\nmode = \"all_open\"",
        "[select]\ntrusted_actors = []",
        "[select]\nlabels_any = [\"검토 필요\", \"CaseSensitive\"]",
        "[classify]\nrule = []",
        "[protection]\npaths = []",
        "[human_required]\nrules = []\nadditional = []",
    ] {
        assert_valid(&validator, source, source);
    }
    for source in [
        "[select]\nlabels_any = [\"\"]",
        "[select]\nlabels_any = [\" docs\"]",
        "[select]\nlabels_any = [\"docs \"]",
        "[select]\nlabels_any = [\"bad\\u007fvalue\"]",
    ] {
        assert_invalid(&validator, source, source);
    }
}

#[test]
fn classification_and_additional_human_rules_need_real_conditions() {
    let validator = validator();
    for source in [
        "[[classify.rule]]\nclass = \"high\"",
        "[[classify.rule]]\nclass = \"high\"\nlabels_any = []\npaths_any = []",
        "[[classify.rule]]\nlabels_any = [\"security\"]",
        "[[human_required.additional]]\ngates = [\"start\"]",
        "[[human_required.additional]]\nlabels_any = []\npaths_any = []\ngates = [\"start\"]",
        "[[human_required.additional]]\nlabels_any = [\"manual\"]",
        "[[human_required.additional]]\nlabels_any = [\"manual\"]\ngates = []",
    ] {
        assert_invalid(&validator, source, source);
    }
    for source in [
        "[[classify.rule]]\nclass = \"high\"\nlabels_any = []\npaths_any = [\"src/**\"]",
        "[[classify.rule]]\nclass = \"normal\"\nlabels_any = [\"task\"]\npaths_any = []",
        "[[human_required.additional]]\nlabels_any = [\"manual\"]\ngates = [\"start\", \"close\"]",
        "[[human_required.additional]]\npaths_any = [\"src/auth/**\"]\ngates = [\"plan\"]",
    ] {
        assert_valid(&validator, source, source);
    }
}

#[test]
fn repository_coordinates_and_relative_globs_have_bounded_syntax() {
    let validator = validator();
    let longest_repository = format!("{}/{}", "a".repeat(39), "r".repeat(100));
    assert_valid(
        &validator,
        "최대 길이 저장소 좌표",
        &format!("[binding.github]\nrepository = \"{longest_repository}\""),
    );
    for value in [
        "owner/repo",
        "owner/repo.name",
        "owner-name/repo_name",
        "a/r",
    ] {
        assert_valid(
            &validator,
            value,
            &format!("[binding.github]\nrepository = \"{value}\""),
        );
    }
    for value in [
        "owner/.",
        "owner/..",
        "-owner/repo",
        "owner-/repo",
        "owner_name/repo",
        "owner/repo/extra",
        "owner/",
        "/repo",
        "소유자/repo",
    ] {
        assert_invalid(
            &validator,
            value,
            &format!("[binding.github]\nrepository = \"{value}\""),
        );
    }
    for value in [
        format!("{}/repo", "a".repeat(40)),
        format!("owner/{}", "r".repeat(101)),
    ] {
        assert_invalid(
            &validator,
            &value,
            &format!("[binding.github]\nrepository = \"{value}\""),
        );
    }
    for value in [
        ".github/workflows/**",
        "**/AGENTS.md",
        "**",
        "*.rs",
        "src/*/file?.rs",
        "src/**/credentials*",
        "문서/설명?.md",
    ] {
        let quoted = serde_json::to_string(value).unwrap();
        assert_valid(
            &validator,
            value,
            &format!("[protection]\npaths = [{quoted}]"),
        );
    }
    for value in [
        "",
        "/src/**",
        "src/**/",
        "src//file",
        ".",
        "..",
        "./src",
        "src/../file",
        "src/./file",
        "src/**x",
        "src/x**",
        "src/***",
        "src/[ab].rs",
        "src/{a,b}.rs",
        "!src/file",
        "C:/src/file",
        "src\\file",
        " src/**",
        "src/** ",
        "src/\u{85}file",
    ] {
        let quoted = serde_json::to_string(value).unwrap();
        assert_invalid(
            &validator,
            value,
            &format!("[protection]\npaths = [{quoted}]"),
        );
    }
}
