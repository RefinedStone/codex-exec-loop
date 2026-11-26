use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use super::{fixture, target};
use crate::adapter::outbound::filesystem::issue_template::FilesystemIssueTemplateSourceAdapter;
use crate::application::port::inbound::issue_draft_port::{
    IssueDraftPort, IssueDraftPreview, IssueDraftPreviewRequest, IssueDraftPublishRequest,
    IssueDraftPublished, IssueDraftText,
};
use crate::application::service::issue_template::IssueTemplateService;

struct Workspace(PathBuf);

impl Workspace {
    fn new() -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-template-flow-{}-{unique}",
            std::process::id()
        ));
        crate::private_directory::ensure_private_directory(&path).unwrap();
        fs::create_dir_all(path.join(".github/ISSUE_TEMPLATE")).unwrap();
        Self(path)
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn filesystem_template_fields_gate_publication_and_changed_bytes_invalidate_approval() {
    for (path, template) in [
        (".github/ISSUE_TEMPLATE/bug.md", "## Steps\n\n## Expected\n"),
        (
            ".github/ISSUE_TEMPLATE/bug.yaml",
            "body:\n  - type: textarea\n    attributes:\n      label: Steps\n    validations:\n      required: true\n  - type: textarea\n    attributes:\n      label: Expected\n    validations:\n      required: true\n",
        ),
    ] {
        let workspace = Workspace::new();
        let template_path = workspace.0.join(path);
        fs::write(&template_path, template).unwrap();
        let mut fixture = fixture(false);
        fixture.service.template =
            IssueTemplateService::new(Arc::new(FilesystemIssueTemplateSourceAdapter::new()));
        *fixture.files.policy.lock().unwrap() =
            format!("[convention]\ntemplate = \"{path}\"\nrequired_sections = [\"Impact\"]\n");
        let mut target = target();
        target.cwd = workspace.0.to_str().unwrap().to_string();

        let opened = fixture.service.open(target.clone()).unwrap();
        assert_eq!(opened.template.status, "resolved");
        assert_eq!(
            opened
                .template
                .required_inputs
                .iter()
                .map(|input| input.label.as_str())
                .collect::<Vec<_>>(),
            ["Steps", "Expected"]
        );
        let incomplete = IssueDraftText {
            title: "Synthetic report".to_string(),
            body: "## Steps\nOpen the fixture\n\n## Impact\nCannot continue\n".to_string(),
            spans: Vec::new(),
        };
        let IssueDraftPreview::Ready {
            approval,
            errors,
            publishable,
            ..
        } = fixture
            .service
            .preview(IssueDraftPreviewRequest {
                expected_labels: Vec::new(),
                target: target.clone(),
                text: incomplete.clone(),
            })
            .unwrap()
        else {
            panic!("the incomplete draft should have a reviewable preview");
        };
        assert!(!publishable);
        assert!(
            errors
                .iter()
                .any(|error| error.label.as_deref() == Some("Expected"))
        );
        let blocked = fixture
            .service
            .publish(IssueDraftPublishRequest {
                target: target.clone(),
                text: incomplete,
                approval,
                edit_basis: opened.edit_basis.clone(),
                shared_copy_only: true,
                validation_id: None,
            })
            .unwrap();
        assert!(matches!(blocked, IssueDraftPublished::Blocked { .. }));
        assert_eq!(*fixture.remote.writes.lock().unwrap(), 0);

        let complete = IssueDraftText {
            title: "Synthetic report".to_string(),
            body: "## Steps\nOpen the fixture\n\n## Expected\nContinue\n\n## Impact\nCannot continue\n".to_string(),
            spans: Vec::new(),
        };
        let IssueDraftPreview::Ready {
            approval,
            publishable,
            ..
        } = fixture
            .service
            .preview(IssueDraftPreviewRequest {
                expected_labels: Vec::new(),
                target: target.clone(),
                text: complete.clone(),
            })
            .unwrap()
        else {
            panic!("the complete draft should have a reviewable preview");
        };
        assert!(publishable);
        // Change only bytes, leaving the same required fields. Approval must still expire.
        fs::write(&template_path, format!("{template}\n")).unwrap();
        let stale = fixture
            .service
            .publish(IssueDraftPublishRequest {
                target: target.clone(),
                text: complete.clone(),
                approval,
                edit_basis: opened.edit_basis.clone(),
                shared_copy_only: true,
                validation_id: None,
            })
            .unwrap();
        assert_eq!(
            stale,
            IssueDraftPublished::Refused {
                reason: "template_version"
            }
        );
        assert_eq!(*fixture.remote.writes.lock().unwrap(), 0);

        let IssueDraftPreview::Ready {
            approval,
            publishable,
            ..
        } = fixture
            .service
            .preview(IssueDraftPreviewRequest {
                expected_labels: Vec::new(),
                target: target.clone(),
                text: complete.clone(),
            })
            .unwrap()
        else {
            panic!("the changed template requires a new preview");
        };
        assert!(publishable);
        let published = fixture
            .service
            .publish(IssueDraftPublishRequest {
                target,
                text: complete,
                approval,
                edit_basis: opened.edit_basis,
                shared_copy_only: true,
                validation_id: None,
            })
            .unwrap();
        assert!(matches!(published, IssueDraftPublished::Confirmed { .. }));
        assert_eq!(*fixture.remote.writes.lock().unwrap(), 1);
    }
}
