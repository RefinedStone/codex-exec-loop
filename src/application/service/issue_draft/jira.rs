use super::*;
use crate::application::port::outbound::issue_field_requirement_port::IssueFieldRequirementPort;
use crate::application::port::outbound::issue_field_requirement_port::IssueFieldRequirementRequest;

pub struct JiraDraftEndpoints {
    pub snapshot: Arc<dyn IssueSnapshotPort>,
    pub requirements: Arc<dyn IssueFieldRequirementPort>,
    pub publisher: Arc<IssueJournaledPublishService>,
}
pub type JiraDraftRouter =
    Arc<dyn Fn(&JiraDraftCoordinate) -> Result<JiraDraftEndpoints> + Send + Sync>;

impl JiraDraftCoordinate {
    pub(super) fn reference(&self, cwd: &str) -> IssueReference {
        IssueReference {
            provider: crate::domain::issue_work::IssueProvider::Jira,
            server_id: self.cloud_id.clone(),
            repository_id: self.project_id.clone(),
            issue_id: self.issue_id.clone(),
            project_scope: IssueProjectScope {
                workspace_dir: cwd.to_owned(),
            },
        }
    }
}

impl IssueDraftService {
    pub(super) fn target_conventions(&self, target: &IssueDraftTarget) -> Result<Conventions> {
        let mut conventions = self.conventions(&target.cwd)?;
        let IssueDraftLocation::Jira { jira } = &target.location else {
            return Ok(conventions);
        };
        let endpoints = self.jira_endpoints(jira)?;
        let fields = endpoints
            .requirements
            .read_requirements(IssueFieldRequirementRequest {
                issue: jira.reference(&target.cwd),
            })?;
        let template = field_view(&fields);
        let basis = serde_json::to_string(&(&target.cwd, jira, template))?;
        conventions.policy_version =
            IssueFieldHash::of_fields(&conventions.policy_version, &basis).sha256;
        conventions.jira_fields = Some(fields);
        Ok(conventions)
    }
}
fn field_view(
    fields: &crate::application::port::outbound::issue_field_requirement_port::IssueFieldRequirementResponse,
) -> (
    Vec<IssueDraftRequiredInput>,
    Vec<IssueDraftUnsupportedInput>,
) {
    let supported = fields
        .required_fields
        .iter()
        .map(|f| {
            let (kind, options) = match &f.format {
                None => ("textarea", Vec::new()),
                Some(IssueFieldFormat::SingleLine) => ("input", Vec::new()),
                Some(IssueFieldFormat::OneOf(options)) => ("dropdown", options.clone()),
            };
            IssueDraftRequiredInput {
                label: f.label.clone(),
                kind,
                options,
            }
        })
        .collect();
    let unsupported = fields
        .unsupported_required_fields
        .iter()
        .map(|f| IssueDraftUnsupportedInput {
            label: f.label.clone(),
            reason: f.reason.clone(),
        })
        .collect();
    (supported, unsupported)
}
pub(super) fn template(conventions: &Conventions) -> IssueDraftTemplate {
    let mut template = template_view(&conventions.template);
    if let Some(fields) = &conventions.jira_fields {
        let (supported, unsupported) = field_view(fields);
        template.required_inputs.extend(supported);
        template.unsupported_inputs.extend(unsupported);
        if template.status == "not_configured" {
            template.status = "resolved";
        }
    }
    template
}
