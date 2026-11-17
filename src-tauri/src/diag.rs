//! A short log of problems the back end ran into on its own (in a timed
//! round, not in answer to a screen). They are printed as before, and kept
//! so a problem report can include them: in an installed Windows app nobody
//! sees what is printed.

use serde::Serialize;
use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Enough to show what led up to a problem without growing for ever.
const KEEP: usize = 50;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogLine {
    /// Unix seconds.
    pub at: i64,
    pub text: String,
}

static LOG: Mutex<VecDeque<LogLine>> = Mutex::new(VecDeque::new());

/// Prints `text` and keeps it for problem reports.
pub fn note(text: impl Into<String>) {
    let text = text.into();
    eprintln!("home-cc: {text}");
    let at = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let mut log = LOG.lock().unwrap_or_else(|e| e.into_inner());
    if log.len() == KEEP {
        log.pop_front();
    }
    log.push_back(LogLine { at, text });
}

/// Oldest first.
pub fn recent() -> Vec<LogLine> {
    LOG.lock().unwrap_or_else(|e| e.into_inner()).iter().cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_only_the_latest_lines_oldest_first() {
        for i in 0..KEEP + 5 {
            note(format!("diag-test line {i}"));
        }
        // Other tests may note things too, so look only at this test's lines.
        let mine: Vec<String> =
            recent().into_iter().map(|l| l.text).filter(|t| t.starts_with("diag-test")).collect();
        assert!(mine.len() <= KEEP);
        assert_eq!(mine.last().map(String::as_str), Some(format!("diag-test line {}", KEEP + 4).as_str()));
        assert!(!mine.iter().any(|t| t == "diag-test line 0"), "the oldest lines are dropped");
    }
}
