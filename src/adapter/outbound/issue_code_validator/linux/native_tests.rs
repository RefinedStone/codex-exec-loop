//! Native capability evidence; an unavailable namespace is a failure, never a skip.
use super::super::SandboxedIssueCodeValidatorRunnerAdapter;
use super::*;
use crate::application::port::outbound::issue_code_validator_runner_port::IssueCodeValidatorRunnerPort;
use crate::application::service::issue_code_validator::gate_issue_publish_with_code_validators;
use crate::application::service::issue_validation::IssuePublishBlocked;
use crate::domain::issue_policy::{
    IssueApprovedCodeValidator, IssueCodeValidatorSelection, IssueDisclosureTarget, IssuePolicy,
};
use crate::domain::issue_validation::IssueRequirements;
use crate::domain::issue_work::IssueSharedCopy;
use std::net::{TcpListener, TcpStream};
use std::os::linux::net::SocketAddrExt;
use std::os::unix::net::{SocketAddr, UnixListener, UnixStream};
use std::sync::{Arc, mpsc};

mod descendants;

fn run(request: &IssueCodeValidatorRunRequest) -> Outcome {
    super::run(request, &IssueCodeValidatorCancellation::new())
}

fn fixture(script: &str) -> (Workspace, IssueCodeValidatorRunRequest) {
    assert!(
        program().is_some(),
        "native Linux sandbox prerequisites unavailable"
    );
    let workspace = Workspace::create().unwrap();
    let source = workspace.0.join("source");
    fs::write(&source, script).unwrap();
    let request = IssueCodeValidatorRunRequest {
        command: source.to_string_lossy().into_owned(),
        sha256: format!("{:x}", Sha256::digest(script)),
        timeout: Duration::from_secs(10),
        input_json: r###"{"schemaVersion":1,"target":"publish","title":"Title","body":"## Steps\n\n[masked-1]"}"###.into(),
    };
    (workspace, request)
}

#[test]
#[ignore = "requires native Linux, system bubblewrap, Python 3 and all declared namespaces"]
fn native_profile_blocks_private_files_writes_and_host_network() {
    // Positive controls establish that these synthetic host resources are reachable.
    let outside = Workspace::create().unwrap();
    let vault = outside.0.join("vault-canary");
    let write_target = outside.0.join("write-canary");
    fs::write(&vault, b"synthetic private value").unwrap();
    let tcp = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = tcp.local_addr().unwrap().port();
    drop(TcpStream::connect(tcp.local_addr().unwrap()).unwrap());
    drop(tcp.accept().unwrap());
    tcp.set_nonblocking(true).unwrap();
    let name = format!("akra-validator-{:032x}", rand::random::<u128>());
    let address = SocketAddr::from_abstract_name(name.as_bytes()).unwrap();
    let unix = UnixListener::bind_addr(&address).unwrap();
    drop(UnixStream::connect_addr(&address).unwrap());
    drop(unix.accept().unwrap());
    unix.set_nonblocking(true).unwrap();
    let script = format!(
        r#"#!/usr/bin/python3
import json, os, socket, sys
data = json.load(sys.stdin)
assert set(data) == {{"schemaVersion", "target", "title", "body"}}
assert data["title"] == "Title" and "[masked-1]" in data["body"]
assert not any(key in os.environ for key in ("HOME", "GH_TOKEN", "GITHUB_TOKEN", "AKRA_HOME"))
for target in ({vault:?}, "/etc/passwd", "/proc/self/status", "/run/user"):
    assert not os.path.exists(target)
for target in ({write_target:?}, "/validator", "/dev/akra-canary", "/dev/shm/akra-canary"):
    try:
        with open(target, "wb") as output: output.write(b"escape")
    except OSError:
        pass
    else:
        raise AssertionError("outside write succeeded")
with open("/tmp/allowed", "wb") as output: output.write(b"scratch")
for family, target in ((socket.AF_INET, ("127.0.0.1", {port})), (socket.AF_UNIX, "\0" + {name:?})):
    client = socket.socket(family, socket.SOCK_STREAM)
    client.settimeout(0.5)
    try:
        client.connect(target)
    except OSError:
        pass
    else:
        raise AssertionError("host network reached")
    finally:
        client.close()
"#,
        vault = vault.to_str().unwrap(),
        write_target = write_target.to_str().unwrap(),
    );
    let (_fixture, request) = fixture(&script);
    assert_eq!(run(&request), Outcome::Passed);
    assert_eq!(fs::read(&vault).unwrap(), b"synthetic private value");
    assert!(!write_target.exists());
    assert_eq!(tcp.accept().unwrap_err().kind(), io::ErrorKind::WouldBlock);
    assert_eq!(unix.accept().unwrap_err().kind(), io::ErrorKind::WouldBlock);
}

#[test]
#[ignore = "requires native Linux, system bubblewrap, Python 3 and all declared namespaces"]
fn native_profile_bounds_output_and_rejects_changed_hash() {
    let (workspace, request) =
        fixture("#!/usr/bin/python3\nimport sys\nsys.stdout.write('x' * 2097152)\nsys.exit(1)\n");
    assert_eq!(
        run(&request),
        Outcome::Rejected {
            message: "x".repeat(MESSAGE_LIMIT)
        }
    );
    fs::write(workspace.0.join("source"), "#!/bin/sh\nexit 0\n").unwrap();
    assert_eq!(run(&request), Outcome::NotRun(NotRun::HashMismatch));
}

#[test]
#[ignore = "requires native Linux, system bubblewrap, Python 3 and all declared namespaces"]
fn native_profile_timeout_removes_observed_detached_descendants() {
    let (_fixture, mut request) = fixture(
        "#!/usr/bin/python3\nimport os, signal, time\nif os.fork() == 0:\n    os.setsid()\nsignal.signal(signal.SIGTERM, signal.SIG_IGN)\nwhile True: time.sleep(10)\n",
    );
    request.timeout = Duration::from_secs(2);
    let observer = descendants::Observer::start(|_| {});
    let started = Instant::now();
    let outcome = run(&request);
    let observed = observer.finish();
    assert_eq!(outcome, Outcome::NotRun(NotRun::TimedOut));
    assert!(started.elapsed() < Duration::from_secs(8));
    observed.assert_detached_validator_was_seen_and_stopped();
}

#[test]
#[ignore = "requires native Linux, system bubblewrap and all declared namespaces"]
fn native_profile_cancel_stops_live_fixture_and_removes_staging() {
    const SCRIPT: &str = "#!/bin/sh\n/bin/sleep 60 &\nwait\n";
    let (_source, mut request) = fixture(SCRIPT);
    request.timeout = Duration::from_secs(15);
    let cancellation = Arc::new(IssueCodeValidatorCancellation::new());
    let observer_cancel = Arc::clone(&cancellation);
    let (staged, evidence) = mpsc::sync_channel(1);
    let mut staged = Some(staged);
    let observer = descendants::Observer::start(move |observed| {
        if staged.is_none() || !observed.has_waiting_fixture_with_child() {
            return;
        }
        // Find the live stage for this exact fixture, never a different process's files.
        let prefix = format!("akra-validator-{}-", std::process::id());
        let stage = fs::read_dir("/tmp")
            .expect("read staging parent")
            .map(|entry| entry.expect("read staging entry"))
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
            .map(|entry| entry.path())
            .find(|path| {
                fs::read(path.join("validator")).is_ok_and(|bytes| bytes == SCRIPT.as_bytes())
                    && path.join("input.json").is_file()
            })
            .expect("the running fixture must have an executable and input staged");
        staged.take().unwrap().send(stage).unwrap();
        assert!(observer_cancel.cancel());
    });

    let outcome = SandboxedIssueCodeValidatorRunnerAdapter::new().run(request, &cancellation);
    let observed = observer.finish();
    assert_eq!(outcome, Outcome::NotRun(NotRun::Cancelled));
    observed.assert_stopped();
    let stage = evidence
        .try_recv()
        .expect("no live fixture stage was observed");
    assert_eq!(
        fs::symlink_metadata(stage).unwrap_err().kind(),
        io::ErrorKind::NotFound,
        "the real runner must remove its staging directory before returning"
    );
}

#[test]
#[ignore = "requires native Linux, system bubblewrap, Python 3 and all declared namespaces"]
fn native_profile_real_service_blocks_required_failure_and_allows_only_pass() {
    let shared = IssueSharedCopy {
        title: "Title".into(),
        body: "## Steps\n\n[masked-1]".into(),
    };
    let requirements = IssueRequirements {
        template_fields: vec![],
        required_sections: vec!["Steps".into()],
    };
    let runner = SandboxedIssueCodeValidatorRunnerAdapter::new();
    for (script, outcome) in [
        ("#!/bin/sh\nexit 0\n", Outcome::Passed),
        (
            "#!/bin/sh\nprintf 'required rejected'\nexit 1\n",
            Outcome::Rejected {
                message: "required rejected".into(),
            },
        ),
        ("#!/bin/sh\nexit 7\n", Outcome::NotRun(NotRun::Crashed)),
        (
            "#!/bin/sh\nexec sleep 30\n",
            Outcome::NotRun(NotRun::TimedOut),
        ),
    ] {
        let (_fixture, request) = fixture(script);
        let mut policy = IssuePolicy::builtin();
        policy.code_validators = vec![IssueCodeValidatorSelection {
            id: "native-fixture".into(),
            targets: vec![IssueDisclosureTarget::Publish],
            required: true,
        }];
        policy.approved_code_validators = vec![IssueApprovedCodeValidator {
            id: "native-fixture".into(),
            command: request.command,
            sha256: request.sha256,
            timeout_seconds: 1,
        }];
        let result = gate_issue_publish_with_code_validators(
            &shared,
            Some(&requirements),
            &policy,
            &runner,
            &IssueCodeValidatorCancellation::new(),
        );
        if outcome == Outcome::Passed {
            assert_eq!(result.unwrap().shared_copy(), &shared);
        } else {
            let Err(IssuePublishBlocked::CodeValidator(blocked)) = result else {
                panic!("required native validator did not block publication");
            };
            assert_eq!(blocked.len(), 1);
            assert_eq!(blocked[0].outcome, outcome);
            assert!(blocked[0].required);
        }
        policy.approved_code_validators[0].sha256 = "0".repeat(64);
        let Err(IssuePublishBlocked::CodeValidator(blocked)) =
            gate_issue_publish_with_code_validators(
                &shared,
                Some(&requirements),
                &policy,
                &runner,
                &IssueCodeValidatorCancellation::new(),
            )
        else {
            panic!("changed validator hash did not block publication");
        };
        assert_eq!(blocked[0].outcome, Outcome::NotRun(NotRun::HashMismatch));
    }
}
