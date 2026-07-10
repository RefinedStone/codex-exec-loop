pub(crate) const TELEGRAM_BOT_COMMAND_USAGE: &str = "Usage: akra telegram [--allow-chat-id <chat_id>]... [--allow-user-id <user_id>]... [--poll-timeout-seconds <seconds>] [--keep-pending] [--rebind-workspace]";
pub(crate) const TELEGRAM_BOT_ALIAS_USAGE: &str = "Alias: akra telegram-bot [--allow-chat-id <chat_id>]... [--allow-user-id <user_id>]... [--poll-timeout-seconds <seconds>] [--keep-pending] [--rebind-workspace]";

const TELEGRAM_BOT_ENV_USAGE: &str = "Env: AKRA_TELEGRAM_BOT_TOKEN, AKRA_TELEGRAM_ALLOWED_CHAT_IDS=123,-456, AKRA_TELEGRAM_ALLOWED_USER_IDS=123,456";
const TELEGRAM_BOT_CONFIG_USAGE: &str = "Config: $XDG_CONFIG_HOME/akra/telegram.env, ~/.config/akra/telegram.env, or %LOCALAPPDATA%\\akra\\telegram.env (Unix mode 0600; Windows protected current-user-only ACL)";

pub(super) fn telegram_bot_usage_text() -> String {
    [
        TELEGRAM_BOT_COMMAND_USAGE,
        TELEGRAM_BOT_ALIAS_USAGE,
        TELEGRAM_BOT_ENV_USAGE,
        TELEGRAM_BOT_CONFIG_USAGE,
    ]
    .join("\n")
}
