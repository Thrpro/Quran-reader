//! The reader's small, origin-bound window-control protocol.
//!
//! This deliberately exposes no file, process, or arbitrary script commands.

use serde::{Deserialize, Serialize};
use url::Url;

pub const MAX_MESSAGE_BYTES: usize = 4_096;
const READER_HOST: &str = "quran-mushaf.local";

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Dark,
    Black,
    Navy,
    Light,
    Sepia,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Arabic,
    English,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum WindowAction {
    Minimize,
    Maximize,
    Close,
    Drag,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Message {
    ReaderTheme {
        theme: Theme,
        #[serde(default)]
        animate: bool,
    },
    ReaderControlsState {
        open: bool,
        language: Language,
    },
    ReadingSessionState {
        active: bool,
        language: Language,
    },
    FullscreenRequest {
        active: bool,
    },
    WindowAction {
        action: WindowAction,
    },
    ReaderReady {},
}

/// Allow only the packaged reader's HTTPS origin, with no credentials.
///
/// URLs are parsed rather than checked with a hostname prefix. An explicit
/// default HTTPS port is equivalent to an omitted port; all other ports fail.
pub fn is_trusted_uri(raw: &str) -> bool {
    if raw != raw.trim() || raw.contains('\\') {
        return false;
    }
    let Ok(uri) = Url::parse(raw) else {
        return false;
    };
    // Url normalizes empty user info away, so reject even an empty "@" in
    // the original authority rather than relying on username() alone.
    let authority = raw
        .split_once("://")
        .map(|(_, tail)| tail.split(['/', '?', '#']).next().unwrap_or_default());
    uri.scheme() == "https"
        && uri.host_str() == Some(READER_HOST)
        && uri.port_or_known_default() == Some(443)
        && uri.username().is_empty()
        && uri.password().is_none()
        && authority.is_some_and(|value| !value.contains('@'))
}

/// Ignore untrusted, oversized, malformed, or unsupported messages.
pub fn parse_message(source: &str, json: &str) -> Option<Message> {
    if !is_trusted_uri(source)
        || json.len() > MAX_MESSAGE_BYTES
        || !json.trim_start().starts_with('{')
    {
        return None;
    }
    serde_json::from_str(json).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = "https://quran-mushaf.local/index.html?v=0.5.0";

    #[test]
    fn trusts_only_the_packaged_https_origin() {
        for raw in [
            SOURCE,
            "https://quran-mushaf.local/",
            "https://quran-mushaf.local:443/index.html",
            "HTTPS://QURAN-MUSHAF.LOCAL/",
            "https://quran-mushaf.local/index.html?next=https://other.example/",
        ] {
            assert!(is_trusted_uri(raw), "rejected {raw}");
        }
        for raw in [
            "http://quran-mushaf.local/index.html",
            "file:///quran-mushaf.local/index.html",
            "https://quran-mushaf.local.attacker.example/",
            "https://attacker.quran-mushaf.local/",
            "https://quran-mushaf.local:444/",
            "https://quran-mushaf.local:80/",
            "https://reader@quran-mushaf.local/",
            "https://reader:secret@quran-mushaf.local/",
            "https://@quran-mushaf.local/",
            "https://:secret@quran-mushaf.local/",
            "https://quran-mushaf.local@attacker.example/",
            "https://quran-mushaf.local./",
            "https://quran-mushaf.local\\@attacker.example/",
            " https://quran-mushaf.local/",
            "https://quran-mushaf.local/\n",
            "quran-mushaf.local",
            "about:blank",
            "data:text/html,reader",
        ] {
            assert!(!is_trusted_uri(raw), "trusted {raw}");
        }
    }

    #[test]
    fn parses_the_current_reader_messages() {
        let cases = [
            (
                r#"{"type":"reader-theme","theme":"sepia","animate":true}"#,
                Message::ReaderTheme {
                    theme: Theme::Sepia,
                    animate: true,
                },
            ),
            (
                r#"{"type":"reader-theme","theme":"dark"}"#,
                Message::ReaderTheme {
                    theme: Theme::Dark,
                    animate: false,
                },
            ),
            (
                r#"{"type":"reader-controls-state","open":false,"language":"arabic"}"#,
                Message::ReaderControlsState {
                    open: false,
                    language: Language::Arabic,
                },
            ),
            (
                r#"{"type":"reading-session-state","active":true,"language":"english"}"#,
                Message::ReadingSessionState {
                    active: true,
                    language: Language::English,
                },
            ),
            (
                r#"{"type":"fullscreen-request","active":true}"#,
                Message::FullscreenRequest { active: true },
            ),
            (
                r#"{"type":"window-action","action":"drag"}"#,
                Message::WindowAction {
                    action: WindowAction::Drag,
                },
            ),
            (r#"{"type":"reader-ready"}"#, Message::ReaderReady {}),
        ];
        for (json, expected) in cases {
            assert_eq!(parse_message(SOURCE, json), Some(expected));
        }
    }

    #[test]
    fn rejects_untrusted_messages_before_dispatch() {
        assert_eq!(
            parse_message(
                "https://quran-mushaf.local.attacker.example/",
                r#"{"type":"window-action","action":"close"}"#
            ),
            None
        );
    }

    #[test]
    fn rejects_unknown_fields_types_and_incorrect_boolean_values() {
        for json in [
            r#"{"type":"reader-theme","theme":"dark","command":"run"}"#,
            r#"{"type":"reader-ready","extra":true}"#,
            r#"{"type":"fullscreen-request","active":"true"}"#,
            r#"{"type":"fullscreen-request","active":1}"#,
            r#"{"type":"fullscreen-request","active":null}"#,
            r#"{"type":"reader-controls-state","open":true}"#,
            r#"{"type":"reader-controls-state","open":true,"language":"french"}"#,
            r#"{"type":"reader-theme","theme":"custom"}"#,
            r#"{"type":"reader-theme","theme":"dark","animate":0}"#,
            r#"{"type":"window-action","action":"execute"}"#,
            r#"{"type":"run-process","command":"example"}"#,
            r#"{"type":"fullscreen-request","active":true,"active":false}"#,
            r#"{"type":"fullscreen-request","type":"reader-ready","active":true}"#,
            r#"{"type":"fullscreen-request"}"#,
            r#"["reader-ready"]"#,
            r#""reader-ready""#,
            "null",
            "{}",
            "{",
        ] {
            assert_eq!(parse_message(SOURCE, json), None, "accepted {json}");
        }
    }

    #[test]
    fn limits_message_bytes_and_nested_json() {
        let padded = format!(
            "{}{{\"type\":\"reader-ready\"}}",
            " ".repeat(MAX_MESSAGE_BYTES)
        );
        assert_eq!(parse_message(SOURCE, &padded), None);
        let deeply_nested = format!("{}null{}", "[".repeat(200), "]".repeat(200));
        assert!(deeply_nested.len() < MAX_MESSAGE_BYTES);
        assert_eq!(parse_message(SOURCE, &deeply_nested), None);
        let padded_limit = format!(
            "{}{{\"type\":\"reader-ready\"}}",
            " ".repeat(MAX_MESSAGE_BYTES - r#"{"type":"reader-ready"}"#.len())
        );
        assert_eq!(padded_limit.len(), MAX_MESSAGE_BYTES);
        assert_eq!(
            parse_message(SOURCE, &padded_limit),
            Some(Message::ReaderReady {})
        );
    }
}
