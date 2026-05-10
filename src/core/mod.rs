/*
 * Core is the headless application runtime boundary. It is not a replacement for
 * domain or application services: it coordinates app-level command/event flow
 * while staying independent from TUI, HTTP, Telegram, and concrete outbound
 * adapters.
 */
pub mod app;
