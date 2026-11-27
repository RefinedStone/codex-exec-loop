//! Bound superseded journal entries only when the writer proves no remote request or vault staging occurred.
use super::IssuePublishIntentRecord;
use crate::domain::issue_publish_intent::IssuePublishIntentState;
use std::collections::{HashMap, HashSet};

const RECENT_UNSENT_HISTORY: usize = 128;

pub(super) fn trim(records: &mut Vec<IssuePublishIntentRecord>) -> HashSet<String> {
    if records.len() <= RECENT_UNSENT_HISTORY {
        return HashSet::new();
    }
    // Keep the newest event for every exact reference, including separate workspace aliases. This
    // also preserves the repo-wide maximum used to allocate publication sequence numbers.
    let mut newest = HashMap::new();
    for record in records.iter() {
        let high = newest.entry(&record.issue).or_insert(0);
        *high = (*high).max(record.sequence);
    }
    let mut eligible: Vec<_> = records
        .iter()
        .filter(|record| {
            matches!(
                record.state,
                IssuePublishIntentState::Conflict | IssuePublishIntentState::Abandoned
            ) && record.write_attempted == Some(false)
                && record.pending_vault_fields.is_empty()
                && record.restore_binding_attempt.is_none()
                && !record.restore_link_finalized
                && !record.write_response_mismatch
                && record.sequence > 0
                && newest
                    .get(&record.issue)
                    .is_some_and(|sequence| *sequence > record.sequence)
        })
        .collect();
    eligible.sort_unstable_by_key(|entry| (entry.sequence, &entry.intent_id));
    let drop_count = eligible.len().saturating_sub(RECENT_UNSENT_HISTORY);
    let removed: HashSet<_> = eligible
        .into_iter()
        .take(drop_count)
        .map(|record| record.intent_id.clone())
        .collect();
    records.retain(|record| !removed.contains(&record.intent_id));
    removed
}

#[cfg(test)]
mod tests {
    use super::super::tests::record;
    use super::*;
    use crate::domain::issue_publish_intent::IssuePublishVaultField;

    #[test]
    fn preserves_unknown_sent_staged_and_each_issues_latest_record() {
        let inert = |index: u64| {
            let mut rec = record(&format!("inert-{index}"), IssuePublishIntentState::Conflict);
            rec.write_attempted = Some(false);
            rec.sequence = index;
            rec
        };
        let mut unknown = inert(1);
        unknown.intent_id = "unknown".into();
        unknown.write_attempted = None;
        let mut sent = inert(1);
        sent.intent_id = "sent".into();
        sent.state = IssuePublishIntentState::NeedsConfirmation;
        let mut staged = inert(1);
        staged.intent_id = "staged".into();
        staged
            .pending_vault_fields
            .push(IssuePublishVaultField::Body);
        let mut other = inert(1);
        other.intent_id = "other".into();
        other.issue.issue_id = "I_2".into();
        let mut records = vec![unknown, sent, staged, other];
        records.extend((1..=130).map(inert));
        assert_eq!(trim(&mut records), HashSet::from(["inert-1".to_owned()]));
        assert!(trim(&mut records).is_empty());
    }
}
