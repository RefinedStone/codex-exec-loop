/*
 * git_sequence는 parallel mode가 수행하는 destructive-ish git 작업을 작은 단계와
 * report로 감싸는 실행 경계다. cleanup, rollback, recovery 같은 상위 서비스는 shell 문자열을 직접
 * 만들지 않고 이 구조를 통해 "무엇을 실행했고 어디서 멈췄는지"를 일관된 진단으로 받는다.
 */
use std::process::{Command, Stdio};

use crate::subprocess;

#[derive(Debug, Clone, PartialEq, Eq)]
/*
GitCommandStep은 cleanup/reconcile 같은 복합 git 작업의 한 단계를 이름과 인자로
표현한다. shell string이 아니라 args vector로 보관하므로 quoting에 덜 취약하고, 실패 시
어떤 단계가 어떤 인자로 실행되었는지 report에 그대로 남길 수 있다.
*/
pub(super) struct GitCommandStep {
    // label은 operator notice와 failure summary에 노출되는 사람이 읽는 단계 이름이다.
    label: String,
    // args는 `git` 뒤에 붙는 argv 그대로라 quoting 없이 실행/진단을 재현할 수 있다.
    args: Vec<String>,
}

impl GitCommandStep {
    // 생성자는 caller가 &str 배열을 넘겨도 owned report로 보존되도록 label과 args를 소유 문자열로 바꾼다.
    pub(super) fn new(
        label: impl Into<String>,
        args: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            label: label.into(),
            args: args.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/*
step report는 실제 git command 실행 결과를 손실 없이 담는 진단 구조이다.
exit code, stdout, stderr를 모두 저장해 상위 cleanup/reconcile 로직이 단순 true/false뿐 아니라
사용자에게 보여 줄 실패 원인을 만들 수 있다.
*/
pub(super) struct GitCommandStepReport {
    // report에도 label/args를 복사해 caller가 원래 step collection을 보관하지 않아도 진단할 수 있다.
    pub(super) label: String,
    pub(super) args: Vec<String>,
    // spawn 실패는 exit code가 없으므로 None으로 두어 "git이 실패"와 "git을 실행하지 못함"을 구분한다.
    pub(super) exit_code: Option<i32>,
    // stdout/stderr는 trimmed string으로 저장해 TUI notice와 test assertion이 불필요한 개행을 다루지 않게 한다.
    pub(super) stdout: String,
    pub(super) stderr: String,
}

impl GitCommandStepReport {
    // git convention상 exit code 0만 성공으로 보고, signal/launch failure는 실패로 남긴다.
    pub(super) fn succeeded(&self) -> bool {
        self.exit_code == Some(0)
    }

    /*
     * failure summary는 상위 recovery notice에 붙을 한 줄 진단이다. stderr를 우선하는 이유는
     * git이 실패 원인을 보통 stderr에 쓰기 때문이고, 비어 있으면 stdout/기본 문구로 fallback한다.
     */
    fn failure_summary(&self) -> String {
        let detail = self
            .stderr
            .lines()
            .find(|line| !line.trim().is_empty())
            .or_else(|| self.stdout.lines().find(|line| !line.trim().is_empty()))
            .unwrap_or("git command exited without diagnostic output");
        format!("{} failed: {detail}", self.label)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/*
sequence report는 여러 git step을 하나의 논리 작업으로 묶은 결과이다. 예를 들어
slot cleanup은 checkout detach, hard reset, clean을 한 sequence로 실행한다. 상위 코드는
`succeeded`로 전체 성공을 확인하고, `failure_summary`로 처음 실패한 단계의 진단만 꺼낼 수 있다.
*/
pub(super) struct GitCommandSequenceReport {
    // sequence label은 "slot cleanup"처럼 여러 git step을 묶는 상위 작업 이름이다.
    pub(super) label: String,
    // steps에는 실행된 단계만 남으므로 실패 후 실행하지 않은 step은 report에 들어가지 않는다.
    pub(super) steps: Vec<GitCommandStepReport>,
}

impl GitCommandSequenceReport {
    // 전체 sequence 성공은 실행된 모든 step이 성공했는지로 판단한다.
    pub(super) fn succeeded(&self) -> bool {
        self.steps.iter().all(GitCommandStepReport::succeeded)
    }

    // 상위 caller는 보통 첫 실패 원인만 operator에게 보여 주면 충분하므로 첫 실패 summary만 반환한다.
    pub(super) fn failure_summary(&self) -> Option<String> {
        self.steps
            .iter()
            .find(|step| !step.succeeded())
            .map(GitCommandStepReport::failure_summary)
    }
}

/*
git sequence 실행은 첫 실패에서 멈춘다. reset/cleanup처럼 순서가 있는 작업에서는
앞 단계가 실패했는데 뒤 단계가 실행되면 상태를 더 복잡하게 만들 수 있기 때문이다. 이미 실행된
step report는 모두 남기므로, 실패 후에도 어디까지 진행되었는지 추적할 수 있다.
*/
pub(super) fn run_git_sequence(
    label: impl Into<String>,
    steps: Vec<GitCommandStep>,
) -> GitCommandSequenceReport {
    let label = label.into();
    let mut reports = Vec::new();

    for step in steps {
        let report = run_git_step(step);
        let succeeded = report.succeeded();
        reports.push(report);
        // 실패 뒤의 reset/clean/cherry-pick 단계를 실행하지 않는 것이 worktree 보존에 더 안전하다.
        if !succeeded {
            break;
        }
    }

    GitCommandSequenceReport {
        label,
        steps: reports,
    }
}

/*
개별 git step은 `GIT_TERMINAL_PROMPT=0`과 null stdin으로 실행된다. 자동화 중에
credential prompt나 interactive input이 뜨면 TUI/background workflow가 멈출 수 있으므로,
실패는 stderr로 수집하고 호출자가 block/retry 정책을 결정하게 한다.
*/
fn run_git_step(step: GitCommandStep) -> GitCommandStepReport {
    let mut command = Command::new("git");
    command
        .args(&step.args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null());
    let command_label = format!("git {}", step.args.join(" "));
    let output = subprocess::command_output(&mut command, &command_label);

    match output {
        Ok(output) => GitCommandStepReport {
            label: step.label,
            args: step.args,
            exit_code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).trim().to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        },
        // git binary 실행 자체가 실패해도 같은 report shape로 올려 상위 정책이 동일하게 처리한다.
        Err(error) => GitCommandStepReport {
            label: step.label,
            args: step.args,
            exit_code: None,
            stdout: String::new(),
            stderr: error.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{GitCommandStep, run_git_sequence};

    #[test]
    fn git_sequence_stops_at_first_failed_step_and_keeps_diagnostic() {
        /*
         * 실제 cleanup/recovery sequence에서 중간 단계가 실패하면 이후 destructive step을
         * 실행하면 안 된다. 이 테스트는 실패 직전까지의 report가 남고, unreached step은 실행되지 않으며,
         * operator-facing failure summary가 실패 label을 포함하는지 고정한다.
         */
        let report = run_git_sequence(
            "invalid sequence",
            vec![
                GitCommandStep::new("git version", ["--version"]),
                GitCommandStep::new("invalid subcommand", ["definitely-not-a-git-command"]),
                GitCommandStep::new("unreached", ["--version"]),
            ],
        );

        assert!(!report.succeeded());
        assert_eq!(report.steps.len(), 2);
        assert_eq!(report.steps[0].label, "git version");
        assert_eq!(report.steps[1].label, "invalid subcommand");
        assert!(
            report
                .failure_summary()
                .expect("failed sequence should have summary")
                .contains("invalid subcommand failed")
        );
    }
}
