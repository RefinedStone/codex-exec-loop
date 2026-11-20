/*
 * 이슈 초안 검토 모델의 메모리 가짜 구현이다 (#2430). 모델을 부르지 않는다: 정한 답을 주고 받은 요청을 기록한다.
 * 실제 모델 어댑터가 오기 전에 검토 서비스와 클라이언트가 계약에 맞춰 일하게 한다.
 */
use std::sync::Mutex;

use anyhow::{Result, anyhow};

use crate::application::port::outbound::issue_review_model_port::{
    IssueReviewModelPort, IssueReviewModelRequest,
};

pub struct FakeIssueReviewModelAdapter {
    /// 모델의 답이다. `Err` 은 모델 호출 실패의 글이다.
    reply: std::result::Result<String, String>,
    requests: Mutex<Vec<IssueReviewModelRequest>>,
}

impl FakeIssueReviewModelAdapter {
    pub fn replying(reply: &str) -> Self {
        Self {
            reply: Ok(reply.to_string()),
            requests: Mutex::new(Vec::new()),
        }
    }

    pub fn failing(error: &str) -> Self {
        Self {
            reply: Err(error.to_string()),
            requests: Mutex::new(Vec::new()),
        }
    }

    /// 지금까지 받은 요청이다.
    pub fn requests(&self) -> Vec<IssueReviewModelRequest> {
        self.requests
            .lock()
            .map(|requests| requests.clone())
            .unwrap_or_default()
    }
}

impl IssueReviewModelPort for FakeIssueReviewModelAdapter {
    fn review(&self, request: IssueReviewModelRequest) -> Result<String> {
        self.requests
            .lock()
            .map_err(|_| anyhow!("fake issue review model lock is poisoned"))?
            .push(request);
        self.reply.clone().map_err(|error| anyhow!(error))
    }
}
