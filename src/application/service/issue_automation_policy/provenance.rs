use std::collections::BTreeMap;

use super::{
    IssueAutomationPolicyOriginScope as Scope, IssueAutomationPolicyResolution as Resolution,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FieldProvenance {
    pub scopes: Vec<Scope>,
    pub resolution: Resolution,
}

impl Default for FieldProvenance {
    fn default() -> Self {
        Self {
            scopes: vec![Scope::Builtin],
            resolution: Resolution::Default,
        }
    }
}

/// 병합이 실제로 읽은 선언과 연산만 기록한다. 최종 효과값 비교로 출처를 역추론하지 않는다.
#[derive(Default)]
pub(super) struct PolicyProvenance {
    fields: BTreeMap<String, FieldProvenance>,
}

impl PolicyProvenance {
    pub fn for_field(&self, key: &str) -> FieldProvenance {
        self.fields.get(key).cloned().unwrap_or_default()
    }

    pub fn replace(&mut self, key: &str, scope: Scope) {
        self.fields.insert(
            key.to_owned(),
            FieldProvenance {
                scopes: vec![scope],
                resolution: Resolution::Override,
            },
        );
    }

    pub fn combine(&mut self, key: &str, scope: Scope, resolution: Resolution) {
        let entry = self.fields.entry(key.to_owned()).or_default();
        entry.scopes.push(scope);
        entry.scopes.sort_unstable();
        entry.scopes.dedup();
        entry.resolution = resolution;
    }

    /// 최종 leaf의 값으로 도출한 기본값은 실제 입력 leaf들의 출처를 함께 보존한다.
    pub fn derive(&mut self, key: &str, inputs: &[&str]) {
        let mut scopes = Vec::new();
        for input in inputs {
            scopes.extend(self.for_field(input).scopes);
        }
        scopes.sort_unstable();
        scopes.dedup();
        self.fields.insert(
            key.to_owned(),
            FieldProvenance {
                scopes,
                resolution: Resolution::Derived,
            },
        );
    }

    /// 요청 mode와 유효 상한 모두가 최종 mode의 입력이다. 상한의 min 출처도 버리지 않는다.
    pub fn cap(&mut self, key: &str, ceiling_key: &str) {
        let ceiling = self.for_field(ceiling_key);
        let entry = self.fields.entry(key.to_owned()).or_default();
        entry.scopes.extend(ceiling.scopes);
        entry.scopes.sort_unstable();
        entry.scopes.dedup();
        entry.resolution = Resolution::Cap;
    }
}
