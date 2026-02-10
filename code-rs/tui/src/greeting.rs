use chrono::Local;
use chrono::Timelike;

fn current_hour() -> u32 {
    if let Ok(fake) = std::env::var("CODEX_TUI_FAKE_HOUR")
        && let Ok(parsed) = fake.parse::<u32>() {
            return parsed.min(23);
        }
    Local::now().hour()
}

/// Build a time-aware placeholder like
/// "What can I code for you this morning?".
pub(crate) fn greeting_placeholder() -> String {
    let hour = current_hour();
    // Custom mapping: show "today" for 10:00–13:59 local time.
    let when = if (10..=13).contains(&hour) {
        "today"
    } else if (5..=9).contains(&hour) {
        "this morning"
    } else if (14..=16).contains(&hour) {
        "this afternoon"
    } else if (17..=20).contains(&hour) {
        "this evening"
    } else {
        // Late night and very early hours
        "tonight"
    };
    format!("What can I code for you {when}?")
}
