use super::*;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Stdio};
use std::sync::mpsc;

struct PreparedRefTransaction {
    child: Child,
    input: Option<ChildStdin>,
    output: BufReader<ChildStdout>,
}

impl PreparedRefTransaction {
    fn new(repository: &IssueFormRepository, reference: &str, next: &str, expected: &str) -> Self {
        let mut child = repository_command(repository, &["update-ref", "--no-deref", "--stdin"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let input = child.stdin.take();
        let output = BufReader::new(child.stdout.take().unwrap());
        let mut transaction = Self {
            child,
            input,
            output,
        };
        transaction.send(&format!(
            "start\nupdate {reference} {next} {expected}\nprepare"
        ));
        assert_eq!(transaction.reply(), "start: ok");
        assert_eq!(transaction.reply(), "prepare: ok");
        transaction
    }

    fn send(&mut self, command: &str) {
        let input = self.input.as_mut().unwrap();
        writeln!(input, "{command}").unwrap();
        input.flush().unwrap();
    }

    fn reply(&mut self) -> String {
        let mut line = String::new();
        self.output.read_line(&mut line).unwrap();
        line.trim().to_owned()
    }

    fn finish(mut self, commit: bool) {
        let command = if commit { "commit" } else { "abort" };
        self.send(command);
        assert_eq!(self.reply(), format!("{command}: ok"));
        drop(self.input.take());
        assert!(self.child.wait().unwrap().success());
    }
}

impl Drop for PreparedRefTransaction {
    fn drop(&mut self) {
        drop(self.input.take());
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

#[test]
fn ref_cas_waits_for_prepared_git_transaction_without_overwriting_its_winner() {
    for commit in [false, true] {
        let fixture = Fixture::new();
        let repository = GitIssueFormRepositoryAdapter::new()
            .inspect(fixture.repo.to_str().unwrap(), "main")
            .unwrap();
        let reference = fixture.target().journal_ref();
        let old = git_line(
            &repository,
            &["hash-object", "-w", "--stdin"],
            Some(b"initial journal"),
        )
        .unwrap();
        let winner = git_line(
            &repository,
            &["hash-object", "-w", "--stdin"],
            Some(b"competing journal"),
        )
        .unwrap();
        let next = git_line(
            &repository,
            &["hash-object", "-w", "--stdin"],
            Some(b"candidate journal"),
        )
        .unwrap();
        git_success(&repository, &["update-ref", &reference, &old], None).unwrap();
        let transaction = PreparedRefTransaction::new(&repository, &reference, &winner, &old);
        let (started_tx, started_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();

        std::thread::scope(|scope| {
            scope.spawn(|| {
                started_tx.send(()).unwrap();
                result_tx
                    .send(cas_ref(&repository, &reference, &next, Some(&old)))
                    .unwrap();
            });
            started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            let early = result_rx.recv_timeout(Duration::from_millis(300));
            // End the real Git transaction even when the old implementation fails early.
            transaction.finish(commit);
            assert!(
                matches!(early, Err(mpsc::RecvTimeoutError::Timeout)),
                "CAS must wait for the prepared transaction, got {early:?}"
            );
            assert_eq!(
                result_rx
                    .recv_timeout(Duration::from_secs(6))
                    .unwrap()
                    .unwrap(),
                !commit
            );
        });

        assert_eq!(
            read_ref(&repository, &reference).unwrap().as_deref(),
            Some(if commit {
                winner.as_str()
            } else {
                next.as_str()
            })
        );
    }
}
