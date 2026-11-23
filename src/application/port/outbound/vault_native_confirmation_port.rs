//! 실제 OS 대화상자의 사용자 확인이다. 외부 입력으로 확인 결과나 목적을 주입하지 않는다.
use std::sync::atomic::AtomicBool;
use std::time::Instant;

#[cfg(any(windows, test))]
pub(crate) mod helper_protocol {
    //! Bounded messages on the inherited native-confirmation pipes; no approval input.
    use super::{
        VaultNativeConfirmationOutcome as Outcome, VaultNativeConfirmationPurpose as Purpose,
        VaultNativeConfirmationRequest,
    };
    use serde::{Deserialize, Serialize};

    pub(crate) const COMMAND: &str = "vault-native-confirmation";
    pub(crate) const MAX_REQUEST_BYTES: usize = 16 * 1024;
    pub(crate) const MAX_RESPONSE_BYTES: usize = 1024;
    pub(crate) const MAX_LIFETIME_MILLIS: u32 = 300_000;

    #[derive(Debug, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub(crate) struct Request {
        pub version: u8,
        pub id: String,
        pub purpose: Purpose,
        pub issue_url: String,
        pub workspace_dir: String,
        pub lifetime_millis: u32,
    }

    impl Request {
        pub(crate) fn from_target(
            target: &VaultNativeConfirmationRequest,
            id: String,
            lifetime_millis: u32,
        ) -> Self {
            Self {
                version: 1,
                id,
                purpose: target.purpose,
                issue_url: target.issue_url.clone(),
                workspace_dir: target.workspace_dir.clone(),
                lifetime_millis,
            }
        }

        pub(crate) fn target(&self) -> Option<VaultNativeConfirmationRequest> {
            (self.version == 1
                && self.id.len() == 32
                && self.id.bytes().all(|byte| byte.is_ascii_hexdigit())
                && (1..=MAX_LIFETIME_MILLIS).contains(&self.lifetime_millis)
                && self.issue_url.starts_with("https://github.com/")
                && plain_text(&self.issue_url, 2048)
                && plain_text(&self.workspace_dir, 4096))
            .then(|| VaultNativeConfirmationRequest {
                issue_url: self.issue_url.clone(),
                workspace_dir: self.workspace_dir.clone(),
                purpose: self.purpose,
            })
        }
    }

    fn plain_text(value: &str, maximum: usize) -> bool {
        !value.is_empty() && value.len() <= maximum && !value.chars().any(char::is_control)
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub(crate) enum Decision {
        Confirmed,
        Cancelled,
        Unavailable,
    }

    #[derive(Debug, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub(crate) struct Receipt {
        pub version: u8,
        pub id: String,
        pub purpose: Purpose,
        pub decision: Decision,
    }

    impl Receipt {
        pub(crate) fn new(request: &Request, decision: Decision) -> Self {
            Self {
                version: 1,
                id: request.id.clone(),
                purpose: request.purpose,
                decision,
            }
        }

        pub(crate) fn outcome(&self, request: &Request) -> Option<Outcome> {
            if self.version != 1 || self.id != request.id || self.purpose != request.purpose {
                return None;
            }
            Some(match self.decision {
                Decision::Confirmed => Outcome::Confirmed,
                Decision::Cancelled => Outcome::Cancelled,
                Decision::Unavailable => Outcome::Unavailable,
            })
        }
    }

    pub(crate) fn encode<T: Serialize>(value: &T, maximum: usize) -> Option<Vec<u8>> {
        let mut bytes = serde_json::to_vec(value).ok()?;
        bytes.push(b'\n');
        (bytes.len() <= maximum).then_some(bytes)
    }

    pub(crate) fn decode_request(bytes: &[u8]) -> Option<Request> {
        let request: Request = decode(bytes, MAX_REQUEST_BYTES)?;
        request.target()?;
        Some(request)
    }

    pub(crate) fn decode_receipt(bytes: &[u8], request: &Request) -> Option<Outcome> {
        decode::<Receipt>(bytes, MAX_RESPONSE_BYTES)?.outcome(request)
    }

    fn decode<T: for<'de> Deserialize<'de>>(bytes: &[u8], maximum: usize) -> Option<T> {
        if bytes.len() > maximum {
            return None;
        }
        let body = bytes.strip_suffix(b"\n")?;
        if body.contains(&b'\n') || body.contains(&b'\r') {
            return None;
        }
        serde_json::from_slice(body).ok()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn request() -> Request {
            Request::from_target(
                &VaultNativeConfirmationRequest {
                    issue_url: "https://github.com/acme/project/issues/1".into(),
                    workspace_dir: r"C:\work\한글 project".into(),
                    purpose: Purpose::ViewOriginal,
                },
                "0123456789abcdef0123456789abcdef".into(),
                20_000,
            )
        }

        #[test]
        fn native_pipe_request_preserves_target_and_distinct_purpose() {
            assert_eq!(COMMAND, "vault-native-confirmation");
            for purpose in [Purpose::ViewOriginal, Purpose::RewriteBinding] {
                let mut input = request();
                input.purpose = purpose;
                let bytes = encode(&input, MAX_REQUEST_BYTES).unwrap();
                let decoded = decode_request(&bytes).unwrap();
                assert_eq!(decoded.workspace_dir, input.workspace_dir);
                assert_eq!(decoded.issue_url, input.issue_url);
                assert_eq!(decoded.purpose, purpose);
            }
        }

        #[test]
        fn native_pipe_has_no_approval_override_and_rejects_unbounded_or_extra_requests() {
            let input = request();
            let valid = encode(&input, MAX_REQUEST_BYTES).unwrap();
            let mut value = serde_json::to_value(&input).unwrap();
            value["approved"] = true.into();
            assert!(decode_request(&encode(&value, MAX_REQUEST_BYTES).unwrap()).is_none());
            let mut twice = valid.clone();
            twice.extend_from_slice(&valid);
            assert!(decode_request(&twice).is_none());
            assert!(decode_request(&valid[..valid.len() - 1]).is_none());
            assert!(decode_request(&vec![b'x'; MAX_REQUEST_BYTES + 1]).is_none());
            for lifetime in [0, MAX_LIFETIME_MILLIS + 1] {
                let mut input = request();
                input.lifetime_millis = lifetime;
                assert!(decode_request(&encode(&input, MAX_REQUEST_BYTES).unwrap()).is_none());
            }
            for invalid in ["", "line\nbreak", "nul\0byte"] {
                let mut input = request();
                input.workspace_dir = invalid.into();
                assert!(decode_request(&encode(&input, MAX_REQUEST_BYTES).unwrap()).is_none());
            }
        }

        #[test]
        fn native_receipt_is_bound_to_one_request_purpose_and_exact_frame() {
            let input = request();
            let mut receipt = Receipt::new(&input, Decision::Confirmed);
            let bytes = encode(&receipt, MAX_RESPONSE_BYTES).unwrap();
            assert_eq!(decode_receipt(&bytes, &input), Some(Outcome::Confirmed));
            receipt.purpose = Purpose::RewriteBinding;
            assert!(
                decode_receipt(&encode(&receipt, MAX_RESPONSE_BYTES).unwrap(), &input).is_none()
            );
            receipt.purpose = input.purpose;
            receipt.id.push('0');
            assert!(
                decode_receipt(&encode(&receipt, MAX_RESPONSE_BYTES).unwrap(), &input).is_none()
            );
            let mut extra = bytes.clone();
            extra.extend_from_slice(b"\n");
            assert!(decode_receipt(&extra, &input).is_none());
            let mut value =
                serde_json::to_value(Receipt::new(&input, Decision::Confirmed)).unwrap();
            value["approved"] = true.into();
            assert!(decode_receipt(&encode(&value, MAX_RESPONSE_BYTES).unwrap(), &input).is_none());
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(windows, test), derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(any(windows, test), serde(rename_all = "camelCase"))]
pub(crate) enum VaultNativeConfirmationPurpose {
    ViewOriginal,
    RewriteBinding,
}

pub(crate) struct VaultNativeConfirmationRequest {
    pub issue_url: String,
    pub workspace_dir: String,
    pub purpose: VaultNativeConfirmationPurpose,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VaultNativeConfirmationOutcome {
    #[cfg(any(target_os = "linux", target_os = "macos", windows, test))]
    Confirmed,
    Cancelled,
    #[cfg(any(not(windows), test))]
    Unsupported,
    Unavailable,
}
pub(crate) trait VaultNativeConfirmationPort: Send + Sync {
    fn confirm(
        &self,
        request: &VaultNativeConfirmationRequest,
        cancelled: &AtomicBool,
        deadline: Instant,
    ) -> VaultNativeConfirmationOutcome;
}
