use std::process::{Command, Stdio};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GitCommandStep {
    label: String,
    args: Vec<String>,
}

impl GitCommandStep {
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
pub(super) struct GitCommandStepReport {
    pub(super) label: String,
    pub(super) args: Vec<String>,
    pub(super) exit_code: Option<i32>,
    pub(super) stdout: String,
    pub(super) stderr: String,
}

impl GitCommandStepReport {
    pub(super) fn succeeded(&self) -> bool {
        self.exit_code == Some(0)
    }

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
pub(super) struct GitCommandSequenceReport {
    pub(super) label: String,
    pub(super) steps: Vec<GitCommandStepReport>,
}

impl GitCommandSequenceReport {
    pub(super) fn succeeded(&self) -> bool {
        self.steps.iter().all(GitCommandStepReport::succeeded)
    }

    pub(super) fn failure_summary(&self) -> Option<String> {
        self.steps
            .iter()
            .find(|step| !step.succeeded())
            .map(GitCommandStepReport::failure_summary)
    }
}

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
        if !succeeded {
            break;
        }
    }

    GitCommandSequenceReport {
        label,
        steps: reports,
    }
}

fn run_git_step(step: GitCommandStep) -> GitCommandStepReport {
    let output = Command::new("git")
        .args(&step.args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .output();

    match output {
        Ok(output) => GitCommandStepReport {
            label: step.label,
            args: step.args,
            exit_code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).trim().to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        },
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
