//! Desktop notifications for the panel, sent over the session D-Bus.
//!
//! The panel writes one JSON object per line to the helper's stdin:
//!   {"t":"notify","title":"…","body":"…","urgent":false}
//! The text goes straight to org.freedesktop.Notifications, never into a
//! command line, so other local accounts can't read it from the process list.

use std::collections::HashMap;
use std::time::Duration;

use dbus::arg::{PropMap, RefArg, Variant};
use dbus::blocking::Connection;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};

const MAX_LINE: usize = 8 * 1024;
const MAX_TITLE: usize = 200;
const MAX_BODY: usize = 1000;

#[derive(Debug, PartialEq)]
pub struct Note {
    pub title: String,
    pub body: String,
    pub urgent: bool,
}

fn clean(text: &str, limit: usize) -> String {
    text.chars().filter(|c| *c == '\n' || !c.is_control()).take(limit).collect()
}

/// One stdin line -> a notification, or None for anything else.
pub fn parse(line: &str) -> Option<Note> {
    if line.len() > MAX_LINE {
        return None;
    }
    let v: Value = serde_json::from_str(line).ok()?;
    if v.get("t")?.as_str()? != "notify" {
        return None;
    }
    let title = clean(v.get("title")?.as_str()?, MAX_TITLE);
    if title.trim().is_empty() {
        return None;
    }
    Some(Note {
        title,
        body: clean(v.get("body").and_then(Value::as_str).unwrap_or(""), MAX_BODY),
        urgent: v.get("urgent").and_then(Value::as_bool).unwrap_or(false),
    })
}

fn send(note: &Note) -> Result<(), dbus::Error> {
    let conn = Connection::new_session()?;
    let proxy = conn.with_proxy("org.freedesktop.Notifications", "/org/freedesktop/Notifications", Duration::from_secs(5));
    let mut hints: PropMap = HashMap::new();
    let urgency: u8 = if note.urgent { 2 } else { 1 };
    hints.insert("urgency".to_owned(), Variant(Box::new(urgency) as Box<dyn RefArg>));
    let (_id,): (u32,) = proxy.method_call(
        "org.freedesktop.Notifications",
        "Notify",
        ("Helio Strap", 0u32, "weather-clear-night", note.title.as_str(), note.body.as_str(), Vec::<String>::new(), hints, -1i32),
    )?;
    Ok(())
}

/// Read notification requests from stdin until it closes.
pub async fn serve() {
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let Some(note) = parse(&line) else { continue };
        let result = tokio::task::spawn_blocking(move || send(&note)).await;
        if let Ok(Err(e)) = result {
            eprintln!("Could not show a notification: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_requests() {
        let n = parse(r#"{"t":"notify","title":"7h 25m of sleep","body":"Deep 1h\nlight 3h","urgent":true}"#).unwrap();
        assert_eq!(n, Note { title: "7h 25m of sleep".into(), body: "Deep 1h\nlight 3h".into(), urgent: true });
        assert_eq!(parse(r#"{"t":"notify","title":"a\u001b[31mb"}"#).unwrap().title, "a[31mb");
        assert_eq!(parse(r#"{"t":"notify","title":"x"}"#).unwrap().body, "");
        for bad in [r#"{"t":"other","title":"x"}"#, r#"{"t":"notify","title":"  "}"#, r#"{"t":"notify"}"#, "not json", ""] {
            assert!(parse(bad).is_none(), "{bad}");
        }
        assert_eq!(parse(&format!(r#"{{"t":"notify","title":"{}"}}"#, "x".repeat(500))).unwrap().title.len(), MAX_TITLE);
        assert!(parse(&"x".repeat(MAX_LINE + 1)).is_none());
    }
}
