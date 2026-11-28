use super::*;

impl PreparedTaskVerification for PreparedVerification {
    fn manifest(&self) -> &TaskVerificationExecutionManifest {
        &self.manifest
    }

    fn run(self: Box<Self>, cancelled: &dyn Fn() -> bool) -> TaskVerificationRunOutcome {
        self.run_with_consent(declared::confirm, cancelled)
    }
}

impl PreparedVerification {
    pub(super) fn run_with_consent(
        mut self,
        confirm: impl FnOnce(
            &std::process::Command,
            &TaskVerificationExecutionManifest,
            Instant,
            &dyn Fn() -> bool,
        ) -> Result<(), Reason>,
        cancelled: &dyn Fn() -> bool,
    ) -> TaskVerificationRunOutcome {
        if cancelled() {
            return not_run(Reason::Cancelled);
        }
        if Instant::now() >= self.deadline {
            return not_run(Reason::Unavailable);
        }
        if !self.environment_matches()
            || self.context.verify().is_err()
            || self.binary.verify().is_err()
        {
            return not_run(Reason::ConfigurationChanged);
        }
        let Ok(mut command) = self.context.command(&self.binary) else {
            return not_run(Reason::ConfigurationChanged);
        };
        match &self.declared {
            Some(grep) => {
                command.args(grep.arguments(&self.manifest.source.tree_sha));
                if let Err(reason) = confirm(&command, &self.manifest, self.deadline, cancelled) {
                    return not_run(reason);
                }
                if self.binary.verify().is_err()
                    || self.context.verify().is_err()
                    || !self.environment_matches()
                {
                    return not_run(Reason::ConfigurationChanged);
                }
            }
            None => environment::append_diff_arguments(
                &mut command,
                &self.manifest.base_tree_sha,
                &self.manifest.source.tree_sha,
            ),
        }
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        if cancelled() {
            return not_run(Reason::Cancelled);
        }
        if Instant::now() >= self.deadline {
            return not_run(Reason::Unavailable);
        }
        let started_at = chrono::Utc::now();
        let started = Instant::now();
        let child = match akra_native::subprocess::spawn(&mut command) {
            Ok(child) => child,
            Err(_) => return not_run(Reason::SpawnFailed),
        };
        let output = wait_with_output_controlled(
            child,
            self.deadline,
            cancelled,
            MAX_TASK_VERIFICATION_STREAM_BYTES as usize,
            MAX_TASK_VERIFICATION_STREAM_BYTES as usize,
        );
        let mut outcome = match output.termination {
            ControlledTermination::Exited(status) => match status.code() {
                Some(exit_code) => TaskVerificationExecutionOutcome::Exited { exit_code },
                None => TaskVerificationExecutionOutcome::Signalled {},
            },
            ControlledTermination::TimedOut => TaskVerificationExecutionOutcome::TimedOut {},
            ControlledTermination::Cancelled => TaskVerificationExecutionOutcome::Cancelled {},
            ControlledTermination::OutputLimit => TaskVerificationExecutionOutcome::OutputLimit {},
            ControlledTermination::IoFailure => TaskVerificationExecutionOutcome::IoFailure {},
        };
        if self.context.verify().is_err()
            || self.binary.verify().is_err()
            || !self.environment_matches()
        {
            outcome = TaskVerificationExecutionOutcome::IoFailure {};
        }
        let finished_at = chrono::Utc::now();
        let observation = TaskVerificationObservation {
            outcome,
            started_at: started_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            finished_at: finished_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            stdout: stream(&output.stdout, output.stdout_complete),
            stderr: stream(&output.stderr, output.stderr_complete),
        };
        TaskVerificationRunOutcome::Executed {
            observation: Box::new(observation),
            stdout: output.stdout,
            stderr: output.stderr,
        }
    }
}

impl PreparedVerification {
    fn environment_matches(&self) -> bool {
        HostEnvironment::read()
            .and_then(|current| current.recipe_fingerprint(&self.binary, &self.manifest))
            .is_ok_and(|value| value == self.manifest.environment_sha256)
    }
}

fn stream(bytes: &[u8], complete: bool) -> TaskVerificationStream {
    TaskVerificationStream {
        sha256: digest(bytes),
        size_bytes: bytes.len() as u64,
        complete,
    }
}

fn not_run(reason: Reason) -> TaskVerificationRunOutcome {
    TaskVerificationRunOutcome::NotRun { reason }
}
