use anyhow::Result;

/*
 * 외부 문서의 지금 버전을 알려 주는 제공자이다 (plan 22 2단계). 서비스 종류 하나가 구현 하나이다. 버전은 그
 * 서비스가 주는 글자이다 (예: 마지막 수정 시각). 같은 글자이면 문서가 그대로이다. 모델을 부르지 않는다.
 */
pub trait ExternalResourceVersionPort: Send + Sync {
    /// 이 제공자가 아는 문서 종류이다. 인식기의 `kind()` 와 같은 값이다.
    fn kind(&self) -> &'static str;

    /// 문서의 지금 버전이다. `key` 는 장부의 문서 키이다. 읽을 수 없으면(권한 없음, 지워짐, 서비스에 닿지 않음,
    /// 이 제공자가 모르는 키) 오류이다.
    fn current_version(&self, key: &str) -> Result<String>;
}
