use super::super::{IsolatedGithubNetworkContext, validate_private_directory};
use super::{GitFixture, path_str};
use std::fs;
use std::path::PathBuf;
use std::sync::Barrier;

struct OwnedRoot(PathBuf);
impl Drop for OwnedRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn concurrent_git_only_contexts_keep_the_shared_temp_boundary_private() {
    let _environment = super::github_script_lock().lock().unwrap();
    let fixture = GitFixture::new("concurrent-private-contexts");
    let _cleanup = OwnedRoot(fixture.repo.parent().unwrap().to_path_buf());
    let barrier = Barrier::new(4);
    std::thread::scope(|scope| {
        let workers = (0..4)
            .map(|worker| {
                let fixture = &fixture;
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();
                    for iteration in 0..4 {
                        let context = IsolatedGithubNetworkContext::for_git(
                            path_str(&fixture.repo),
                            path_str(&fixture.remote),
                        )
                        .unwrap_or_else(|error| {
                            panic!("worker={worker}, iteration={iteration}: {error:#}");
                        });
                        validate_private_directory(&context.root).unwrap();
                        let path = context.root.clone();
                        drop(context);
                        assert!(!path.exists(), "only the owned context should be cleaned");
                    }
                })
            })
            .collect::<Vec<_>>();
        for worker in workers {
            worker
                .join()
                .expect("Git-only context worker should finish");
        }
    });
}
