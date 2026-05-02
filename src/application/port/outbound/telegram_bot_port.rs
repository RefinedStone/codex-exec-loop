// 학습 주석: Telegram outbound port는 네트워크, token, HTTP parsing 실패를 application layer로
// 돌려줄 수 있어야 하므로 공통 오류 타입인 `anyhow::Result`를 포트 반환 계약으로 사용합니다.
use anyhow::Result;

// 학습 주석: Poll request는 adapter가 Telegram Bot API의 getUpdates 호출로 변환할 입력 DTO입니다.
// derive된 trait들은 service test에서 request 값을 비교하고, 실패 메시지에 구조를 출력하게 해 줍니다.
#[derive(Debug, Clone, PartialEq, Eq)]
// 학습 주석: `TelegramPollRequest`는 "어디서부터, 얼마나 오래, 몇 개까지 update를 가져올지"를
// application이 adapter에 전달하는 outbound command입니다.
pub struct TelegramPollRequest {
    // 학습 주석: offset은 Telegram update stream cursor입니다. 마지막으로 처리한 update_id 다음 값을
    // 넣으면 adapter가 이미 처리한 message를 다시 가져오지 않게 됩니다.
    pub offset: Option<i64>,
    // 학습 주석: timeout_seconds는 long polling 대기 시간입니다. 짧으면 반응은 빠르지만 요청이 잦고,
    // 길면 빈 polling 비용을 줄일 수 있습니다.
    pub timeout_seconds: u16,
    // 학습 주석: limit은 한 번에 가져올 update 수입니다. application service가 한 tick에서 처리할
    // 작업량을 제어하는 backpressure 입력입니다.
    pub limit: u8,
}

// 학습 주석: constructor를 DTO 옆에 두면 service code가 필드 순서를 직접 기억하지 않고
// Telegram polling 의도를 이름 있는 타입 생성으로 표현할 수 있습니다.
impl TelegramPollRequest {
    // 학습 주석: `new`는 cursor, long polling timeout, batch limit을 한 번에 묶어 poll request를
    // 만듭니다. validation은 없고, 숫자 정책은 이 타입을 만드는 application service가 결정합니다.
    pub fn new(offset: Option<i64>, timeout_seconds: u16, limit: u8) -> Self {
        Self {
            offset,
            timeout_seconds,
            limit,
        }
    }
}

// 학습 주석: inbound message는 Telegram update 중 application이 실제로 명령 해석에 쓰는 message
// 부분만 추려 낸 DTO입니다. adapter는 Telegram JSON을 이 구조로 mapping합니다.
#[derive(Debug, Clone, PartialEq, Eq)]
// 학습 주석: `TelegramInboundMessage`는 외부 채팅 message를 application boundary 안쪽에서 다룰 수
// 있는 안정적인 형태로 줄인 값입니다.
pub struct TelegramInboundMessage {
    // 학습 주석: message_id는 채팅 안에서 message를 식별합니다. update cursor와는 별개라서
    // reply/logging을 할 때 message 단위 추적에 쓰입니다.
    pub message_id: i64,
    // 학습 주석: chat_id는 답장을 보낼 대상 채팅방입니다. send_message request의 chat_id로 그대로
    // 이어지는 inbound/outbound 연결점입니다.
    pub chat_id: i64,
    // 학습 주석: text는 사용자가 보낸 명령 본문입니다. Telegram message에는 사진이나 스티커처럼
    // text가 없는 event도 있으므로 Optional로 둡니다.
    pub text: Option<String>,
    // 학습 주석: sender_display_name은 UI/log에 보여 줄 발신자 이름입니다. Telegram profile 정보가
    // 없거나 adapter가 이름을 만들 수 없으면 None이 됩니다.
    pub sender_display_name: Option<String>,
}

// 학습 주석: update DTO는 Telegram polling cursor의 단위입니다. message가 없을 수도 있기 때문에
// application service는 update_id로 cursor를 전진시키면서 message 유무를 따로 판단합니다.
#[derive(Debug, Clone, PartialEq, Eq)]
// 학습 주석: `TelegramUpdate`는 getUpdates 응답 배열의 한 칸을 application boundary에 맞게
// 단순화한 값입니다.
pub struct TelegramUpdate {
    // 학습 주석: update_id는 offset 계산에 쓰이는 Telegram stream cursor입니다. message_id와 달리
    // bot이 받은 update 전체 순서를 나타냅니다.
    pub update_id: i64,
    // 학습 주석: message가 None이면 application이 처리할 채팅 text는 없지만, update_id는 이미 본
    // update로 기록해야 같은 update를 반복 polling하지 않습니다.
    pub message: Option<TelegramInboundMessage>,
}

// 학습 주석: send request는 application이 사용자에게 응답을 보낼 때 outbound adapter에 넘기는
// command DTO입니다.
#[derive(Debug, Clone, PartialEq, Eq)]
// 학습 주석: `TelegramSendMessageRequest`는 "어느 chat에 어떤 text를 보낼지"만 담습니다. HTTP
// endpoint, token, serialization은 adapter 책임으로 남겨 둡니다.
pub struct TelegramSendMessageRequest {
    // 학습 주석: chat_id는 inbound message에서 온 채팅 식별자이거나 service가 선택한 대상입니다.
    pub chat_id: i64,
    // 학습 주석: text는 Telegram에 보낼 응답 본문입니다. 이 boundary에서는 이미 완성된 문자열이어야
    // 하며, formatting이나 command 해석은 application service가 끝낸 뒤입니다.
    pub text: String,
}

// 학습 주석: send request constructor는 `impl Into<String>`을 받아 caller가 `String`과 `&str`을
// 모두 편하게 넘기게 합니다. DTO 내부는 항상 owned String으로 보관합니다.
impl TelegramSendMessageRequest {
    // 학습 주석: `new`는 chat routing key와 응답 text를 묶어 adapter가 바로 sendMessage로 변환할 수
    // 있는 command를 만듭니다.
    pub fn new(chat_id: i64, text: impl Into<String>) -> Self {
        Self {
            chat_id,
            // 학습 주석: Into 변환은 caller 편의용 borrowed text를 boundary 밖에서도 안전한 owned
            // String으로 바꿉니다.
            text: text.into(),
        }
    }
}

// 학습 주석: `TelegramBotPort`는 application service가 Telegram이라는 외부 시스템을 직접 알지 않고
// polling과 sending만 요청하게 하는 outbound boundary입니다. 실제 HTTP adapter와 test fake가
// 같은 trait을 구현합니다.
pub trait TelegramBotPort: Send + Sync {
    // 학습 주석: `get_updates`는 Telegram update stream을 읽는 operation입니다. request는 cursor와
    // batch 정책을 담고, 결과는 adapter가 domain-safe DTO로 mapping한 update 목록입니다.
    fn get_updates(&self, request: &TelegramPollRequest) -> Result<Vec<TelegramUpdate>>;

    // 학습 주석: `send_message`는 application이 만든 응답을 Telegram chat으로 내보내는 operation입니다.
    // 성공 시 반환값은 없고, 네트워크/API 실패만 Result의 error로 올라옵니다.
    fn send_message(&self, request: &TelegramSendMessageRequest) -> Result<()>;
}
