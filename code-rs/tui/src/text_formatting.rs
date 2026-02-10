use unicode_segmentation::UnicodeSegmentation;

/// Truncate a tool result to fit within the given height and width. If the text is valid JSON, we format it in a
/// compact way before truncating. This is a best-effort approach that may not work perfectly for text where one
/// grapheme spans multiple terminal cells.
#[allow(dead_code)]
pub(crate) fn format_and_truncate_tool_result(
    text: &str,
    max_lines: usize,
    line_width: usize,
) -> String {
    // Work out the maximum number of graphemes we can display for a result. It's not guaranteed that one grapheme
    // equals one cell, so we subtract 1 per line as a conservative buffer.
    let max_graphemes = (max_lines * line_width).saturating_sub(max_lines);

    if let Some(formatted_json) = format_json_compact(text) {
        truncate_text(&formatted_json, max_graphemes)
    } else {
        truncate_text(text, max_graphemes)
    }
}

/// Format JSON text in a compact single-line format with spaces to improve Ratatui wrapping. Returns `None` if the
/// input is not valid JSON.
pub(crate) fn format_json_compact(text: &str) -> Option<String> {
    let json = serde_json::from_str::<serde_json::Value>(text).ok()?;
    let json_pretty = serde_json::to_string_pretty(&json).unwrap_or_else(|_| json.to_string());

    // Convert multi-line pretty JSON to compact single-line format by removing newlines and redundant whitespace.
    let mut result = String::new();
    let mut chars = json_pretty.chars().peekable();
    let mut in_string = false;
    let mut escape_next = false;

    while let Some(ch) = chars.next() {
        match ch {
            '"' if !escape_next => {
                in_string = !in_string;
                result.push(ch);
            }
            '\\' if in_string => {
                escape_next = !escape_next;
                result.push(ch);
            }
            '\n' | '\r' if !in_string => {
                // Skip newlines when not in a string literal.
            }
            ' ' | '\t' if !in_string => {
                if let Some(&next_ch) = chars.peek()
                    && let Some(last_ch) = result.chars().last()
                        && (last_ch == ':' || last_ch == ',') && !matches!(next_ch, '}' | ']') {
                            result.push(' ');
                        }
            }
            _ => {
                if escape_next && in_string {
                    escape_next = false;
                }
                result.push(ch);
            }
        }
    }

    Some(result)
}

/// Truncate `text` to at most `max_graphemes` graphemes, avoiding partial graphemes and adding an ellipsis when there
/// is enough space.
#[allow(dead_code)]
pub(crate) fn truncate_text(text: &str, max_graphemes: usize) -> String {
    let mut graphemes = text.grapheme_indices(true);

    if let Some((byte_index, _)) = graphemes.nth(max_graphemes) {
        if max_graphemes >= 3 {
            let mut truncate_graphemes = text.grapheme_indices(true);
            if let Some((truncate_byte_index, _)) = truncate_graphemes.nth(max_graphemes - 3) {
                let truncated = &text[..truncate_byte_index];
                return format!("{truncated}...");
            }
        }
        text[..byte_index].to_string()
    } else {
        text.to_string()
    }
}
