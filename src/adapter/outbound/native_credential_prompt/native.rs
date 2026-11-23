use super::*;
#[cfg(any(target_os = "macos", target_os = "linux", test))]
mod unix;
#[cfg(windows)]
mod windows;

pub(super) fn policy_text(request: &ConnectRequest, migration: bool) -> String {
    let provider = match request.provider {
        Provider::Jira => "Jira",
        Provider::Linear => "Linear",
    };
    let target = request
        .configuration
        .site_url
        .as_deref()
        .unwrap_or("https://linear.app");
    let action = if migration {
        "기존 연결을 Gateway의 OS 자격 증명 저장소로 옮깁니다."
    } else {
        "API 토큰을 Gateway의 OS 자격 증명 저장소에 저장합니다."
    };
    format!(
        "{provider} 연결\n{target}\n\n{action}\n회사·조직의 데이터 정책에서 이 연결과 해당 이슈의 로컬 사용을 허용하는지 먼저 확인하세요. 허용된 계정과 프로젝트에만 사용하세요.\n\n연결 후 Desktop이 꺼져 있어도 Gateway가 이슈를 읽을 수 있습니다. 조직 정책을 확인했고 이 장치의 연결을 허용하는 경우에만 계속하세요."
    )
}
pub(super) fn token_text(request: &ConnectRequest) -> String {
    let provider = match request.provider {
        Provider::Jira => "Jira",
        Provider::Linear => "Linear",
    };
    format!(
        "{provider} API 토큰을 입력하세요. OS 로그인 암호를 입력하지 마세요.\n계정: {}\n토큰은 일반 화면·로그·녹화로 전달하지 않습니다.",
        request.configuration.email.as_deref().unwrap_or(provider)
    )
}
pub(super) fn confirm(
    request: &ConnectRequest,
    migration: bool,
    cancelled: &AtomicBool,
    deadline: Instant,
) -> CredentialResult<bool> {
    super::check_live(cancelled, deadline)?;
    #[cfg(windows)]
    {
        windows::confirm(&policy_text(request, migration), cancelled, deadline)
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        unix::confirm(&policy_text(request, migration), cancelled, deadline)
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        let _ = (request, migration);
        Err(ProviderCredentialError::Unsupported)
    }
}
pub(super) fn prompt(
    request: &ConnectRequest,
    cancelled: &AtomicBool,
    deadline: Instant,
) -> CredentialResult<SecretToken> {
    if !confirm(request, false, cancelled, deadline)? {
        return Err(ProviderCredentialError::Cancelled);
    }
    super::check_live(cancelled, deadline)?;
    #[cfg(windows)]
    {
        windows::prompt(request, &token_text(request), cancelled, deadline)
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        unix::prompt(request.provider, &token_text(request), cancelled, deadline)
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        Err(ProviderCredentialError::Unsupported)
    }
}
