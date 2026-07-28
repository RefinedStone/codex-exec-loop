/*
 * Core is the framework-free client runtime outside the business hexagon, at
 * its inbound client boundary. It owns process-local
 * command/state/effect/completion flow for a long-lived client, but it is not
 * another business layer and does not replace domain or application services.
 * Composition interprets its effects without making Core depend on TUI, HTTP,
 * Telegram, application service implementations, or concrete outbound
 * adapters.
 */
pub mod app;
pub(crate) mod runtime;
