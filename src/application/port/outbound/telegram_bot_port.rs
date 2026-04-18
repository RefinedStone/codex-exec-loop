use anyhow::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelegramPollRequest {
    pub offset: Option<i64>,
    pub timeout_seconds: u16,
    pub limit: u8,
}

impl TelegramPollRequest {
    pub fn new(offset: Option<i64>, timeout_seconds: u16, limit: u8) -> Self {
        Self {
            offset,
            timeout_seconds,
            limit,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelegramInboundMessage {
    pub message_id: i64,
    pub chat_id: i64,
    pub text: Option<String>,
    pub sender_display_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelegramUpdate {
    pub update_id: i64,
    pub message: Option<TelegramInboundMessage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelegramSendMessageRequest {
    pub chat_id: i64,
    pub text: String,
}

impl TelegramSendMessageRequest {
    pub fn new(chat_id: i64, text: impl Into<String>) -> Self {
        Self {
            chat_id,
            text: text.into(),
        }
    }
}

pub trait TelegramBotPort: Send + Sync {
    fn get_updates(&self, request: &TelegramPollRequest) -> Result<Vec<TelegramUpdate>>;

    fn send_message(&self, request: &TelegramSendMessageRequest) -> Result<()>;
}
