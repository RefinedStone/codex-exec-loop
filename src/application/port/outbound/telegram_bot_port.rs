// Telegram outbound port는 네트워크, bot token, HTTP response parsing 실패를 application layer로
// 돌려줄 수 있어야 한다. Telegram runner는 실패를 retry/logging 정책으로 바꾸고, HTTP adapter는
// transport 세부사항을 소유하므로 공통 오류 타입인 `anyhow::Result`를 port 반환 계약으로 사용한다.
use anyhow::Result;

// Poll request는 adapter가 Telegram Bot API의 getUpdates 호출로 변환할 입력 DTO이다. derive된
// trait들은 service test에서 request 값을 비교하고, 실패 메시지에 cursor/batch 구조를 출력하게 해 준다.
#[derive(Debug, Clone, PartialEq, Eq)]
// `TelegramPollRequest`는 "어디서부터, 얼마나 오래, 몇 개까지 update를 가져올지"를 application이
// adapter에 전달하는 outbound command이다. runner는 이 값으로 polling tick의 backpressure와
// Telegram stream cursor를 함께 제어한다.
pub struct TelegramPollRequest {
    // offset은 Telegram update stream cursor이다. 마지막으로 처리한 update_id 다음 값을 넣으면
    // adapter가 이미 처리한 message를 다시 가져오지 않는다.
    pub offset: Option<i64>,
    // timeout_seconds는 long polling 대기 시간이다. 짧으면 반응은 빠르지만 요청이 잦고, 길면 빈
    // polling 비용을 줄일 수 있다.
    pub timeout_seconds: u16,
    // limit은 한 번에 가져올 update 수이다. application service가 한 tick에서 처리할 작업량을
    // 제어하는 backpressure 입력이다.
    pub limit: u8,
}

// constructor를 DTO 옆에 두면 service code가 필드 순서를 직접 기억하지 않고 Telegram polling
// 의도를 이름 있는 타입 생성으로 표현할 수 있다.
impl TelegramPollRequest {
    // `new`는 cursor, long polling timeout, batch limit을 한 번에 묶어 poll request를 만든다.
    // validation은 없고, 숫자 정책은 이 타입을 만드는 application service가 결정한다.
    pub fn new(offset: Option<i64>, timeout_seconds: u16, limit: u8) -> Self {
        Self {
            offset,
            timeout_seconds,
            limit,
        }
    }
}

// inbound message는 Telegram update 중 application이 실제로 명령 해석에 쓰는 message 부분만
// 추려 낸 DTO이다. adapter는 Telegram JSON을 이 구조로 mapping하고, command parser는 외부 JSON
// shape를 몰라도 chat/text/sender만 보고 제어 명령을 해석한다.
#[derive(Debug, Clone, PartialEq, Eq)]
// `TelegramInboundMessage`는 외부 채팅 message를 application boundary 안쪽에서 다룰 수 있는
// 안정적인 형태로 줄인 값이다.
pub struct TelegramInboundMessage {
    // message_id는 채팅 안에서 message를 식별한다. update cursor와는 별개라서 reply/logging을
    // 할 때 message 단위 추적에 쓰인다.
    pub message_id: i64,
    // chat_id는 답장을 보낼 대상 채팅방이다. send_message request의 chat_id로 그대로 이어지는
    // inbound/outbound 연결점이다.
    pub chat_id: i64,
    // text는 사용자가 보낸 명령 본문이다. Telegram message에는 사진이나 스티커처럼 text가 없는
    // event도 있으므로 Optional로 둔다.
    pub text: Option<String>,
    // sender_display_name은 UI/log에 보여 줄 발신자 이름이다. Telegram profile 정보가 없거나
    // adapter가 이름을 만들 수 없으면 None이 된다.
    pub sender_display_name: Option<String>,
}

// update DTO는 Telegram polling cursor의 단위이다. message가 없을 수도 있기 때문에 application
// service는 update_id로 cursor를 전진시키면서 message 유무를 따로 판단한다.
#[derive(Debug, Clone, PartialEq, Eq)]
// `TelegramUpdate`는 getUpdates 응답 배열의 한 칸을 application boundary에 맞게 단순화한 값이다.
pub struct TelegramUpdate {
    // update_id는 offset 계산에 쓰이는 Telegram stream cursor이다. message_id와 달리 bot이 받은
    // update 전체 순서를 나타낸다.
    pub update_id: i64,
    // message가 None이면 application이 처리할 채팅 text는 없지만, update_id는 이미 본 update로
    // 기록해야 같은 update를 반복 polling하지 않는다.
    pub message: Option<TelegramInboundMessage>,
}

// send request는 application이 사용자에게 응답을 보낼 때 outbound adapter에 넘기는 command DTO이다.
#[derive(Debug, Clone, PartialEq, Eq)]
// `TelegramSendMessageRequest`는 "어느 chat에 어떤 text를 보낼지"만 담는다. HTTP endpoint,
// token, serialization은 adapter 책임으로 남겨 두고, application은 응답 문구와 routing만 결정한다.
pub struct TelegramSendMessageRequest {
    // chat_id는 inbound message에서 온 채팅 식별자이거나 service가 선택한 대상이다.
    pub chat_id: i64,
    // text는 Telegram에 보낼 응답 본문이다. 이 boundary에서는 이미 완성된 문자열이어야 하며,
    // formatting이나 command 해석은 application service가 끝낸 뒤이다.
    pub text: String,
}

// send request constructor는 `impl Into<String>`을 받아 caller가 `String`과 `&str`을 모두 편하게
// 넘기게 한다. DTO 내부는 항상 owned String으로 보관한다.
impl TelegramSendMessageRequest {
    // `new`는 chat routing key와 응답 text를 묶어 adapter가 바로 sendMessage로 변환할 수 있는
    // command를 만든다.
    pub fn new(chat_id: i64, text: impl Into<String>) -> Self {
        Self {
            chat_id,
            // Into 변환은 caller 편의용 borrowed text를 boundary 밖에서도 안전한 owned String으로 바꾼다.
            text: text.into(),
        }
    }
}

// `TelegramBotPort`는 application service가 Telegram이라는 외부 시스템을 직접 알지 않고 polling과
// sending만 요청하게 하는 outbound boundary이다. 실제 HTTP adapter와 test fake가 같은 trait을
// 구현하므로, runner tests는 token/HTTP 없이 cursor 전진, allowed chat filtering, response send
// 여부를 검증할 수 있다.
pub trait TelegramBotPort: Send + Sync {
    // `get_updates`는 Telegram update stream을 읽는 operation이다. request는 cursor와 batch
    // 정책을 담고, 결과는 adapter가 domain-safe DTO로 mapping한 update 목록이다.
    fn get_updates(&self, request: &TelegramPollRequest) -> Result<Vec<TelegramUpdate>>;

    // `send_message`는 application이 만든 응답을 Telegram chat으로 내보내는 operation이다. 성공
    // 시 반환값은 없고, 네트워크/API 실패만 Result의 error로 올라온다.
    fn send_message(&self, request: &TelegramSendMessageRequest) -> Result<()>;
}
