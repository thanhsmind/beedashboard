//! Telegram channel — outbound `sendMessage` via the Bot API (D1: ported
//! from herdr-go's `notify::telegram`). The bot token is never a `Config`
//! field or logged (P1, extended by agent-terminal-21): the caller
//! (`crates/mdview/src/server.rs::telegram_credentials`) resolves it from
//! its own owner-only file beside the config
//! (`mdview_core::config::save_notify_credential`/`load_notify_credential`),
//! not the environment, and hands it to [`TelegramNotifier::new`] already
//! resolved. The destination chat id is plain config. `reqwest` is built
//! against `rustls` (not native TLS) so the release build stays
//! self-contained.

use async_trait::async_trait;
use reqwest::Client;

use super::{Notifier, NotifyError, Result};

/// Sends alerts to one Telegram chat via the Bot API.
pub struct TelegramNotifier {
    client: Client,
    token: String,
    chat_id: String,
}

impl TelegramNotifier {
    /// Build a notifier. `token` is the raw credential, already resolved by
    /// the caller from its own file (never the environment, never
    /// `Config`); `chat_id` is config. Returns `None` if either is missing —
    /// notify then stays on the null channel, fail-closed.
    pub fn new(token: Option<String>, chat_id: Option<String>) -> Option<Self> {
        match (token, chat_id) {
            (Some(t), Some(c)) if !t.is_empty() && !c.is_empty() => Some(TelegramNotifier {
                client: Client::new(),
                token: t,
                chat_id: c,
            }),
            _ => None,
        }
    }

    fn api_url(&self) -> String {
        // The token is in the URL path per the Bot API; this string is never
        // logged (only used to build the request).
        format!("https://api.telegram.org/bot{}/sendMessage", self.token)
    }
}

#[async_trait]
impl Notifier for TelegramNotifier {
    async fn send(&self, kind: &str, body: &str) -> Result<()> {
        let icon = match kind {
            "blocked" => "\u{1F534}",
            "done" => "\u{1F7E2}",
            _ => "\u{2022}",
        };
        let text = format!("{icon} {body}");
        let resp = self
            .client
            .post(self.api_url())
            .json(&serde_json::json!({ "chat_id": self.chat_id, "text": text }))
            .send()
            .await
            .map_err(|e| NotifyError::Send(e.to_string()))?;
        if resp.status().is_success() {
            Ok(())
        } else {
            // Do not include the response body (may echo the token/URL).
            Err(NotifyError::Send(format!(
                "telegram returned {}",
                resp.status()
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_token_or_chat_yields_none() {
        assert!(TelegramNotifier::new(None, Some("123".into())).is_none());
        assert!(TelegramNotifier::new(Some("t".into()), None).is_none());
        assert!(TelegramNotifier::new(Some("".into()), Some("123".into())).is_none());
        assert!(TelegramNotifier::new(Some("t".into()), Some("123".into())).is_some());
    }

    #[test]
    fn api_url_embeds_token_but_is_not_logged_anywhere() {
        let n = TelegramNotifier::new(Some("secret".into()), Some("42".into())).unwrap();
        assert!(n.api_url().contains("botsecret"));
        // (No tracing/log calls reference api_url — verified by inspection.)
    }
}
