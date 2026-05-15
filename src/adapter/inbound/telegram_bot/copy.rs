pub(crate) const TELEGRAM_BOT_COMMAND_USAGE: &str = "Usage: akra telegram [--token <token>] [--allow-chat-id <chat_id>]... [--poll-timeout-seconds <seconds>] [--keep-pending]";
pub(crate) const TELEGRAM_BOT_ALIAS_USAGE: &str = "Alias: akra telegram-bot [--token <token>] [--allow-chat-id <chat_id>]... [--poll-timeout-seconds <seconds>] [--keep-pending]";

const TELEGRAM_BOT_ENV_USAGE: &str =
    "Env: AKRA_TELEGRAM_BOT_TOKEN, AKRA_TELEGRAM_ALLOWED_CHAT_IDS=123,456";
const TELEGRAM_BOT_CONFIG_USAGE: &str =
    "Config: $XDG_CONFIG_HOME/akra/telegram.env or ~/.config/akra/telegram.env";

pub(super) fn telegram_bot_usage_text() -> String {
    [
        TELEGRAM_BOT_COMMAND_USAGE,
        TELEGRAM_BOT_ALIAS_USAGE,
        TELEGRAM_BOT_ENV_USAGE,
        TELEGRAM_BOT_CONFIG_USAGE,
    ]
    .join("\n")
}
