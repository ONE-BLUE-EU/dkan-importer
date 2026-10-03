/// Normalize text by replacing control characters with spaces and normalizing whitespace
/// Replaces newlines and control characters with spaces (but keeps asterisks and full text)
/// Also removes spaces before trailing asterisks for consistent field name matching
pub fn normalize_string(value: &str) -> String {
    let normalized = value
        .chars() // Process character by character
        .map(|c| {
            if c.is_control() {
                ' ' // Replace control characters (newlines, tabs, etc.) with spaces
            } else {
                c // Keep all other characters including asterisks
            }
        })
        .collect::<String>()
        .split_whitespace() // Split on whitespace to normalize multiple spaces
        .collect::<Vec<&str>>()
        .join(" ") // Join back with single spaces
        .trim() // Remove leading/trailing whitespace
        .to_string();

    // Simple approach: reverse string, remove spaces before trailing asterisks, reverse back
    let reversed: String = normalized.chars().rev().collect();

    if reversed.starts_with('*') {
        // Find where asterisks end and remove spaces until next non-space character
        let mut result = String::new();
        let mut chars = reversed.chars();

        // Add all leading asterisks
        for ch in chars.by_ref() {
            if ch == '*' {
                result.push(ch);
            } else if ch == ' ' {
                // Skip spaces after asterisks
                continue;
            } else {
                // Found first non-space, non-asterisk character
                result.push(ch);
                break;
            }
        }

        // Add the rest of the string
        result.extend(chars);

        // Reverse back
        result.chars().rev().collect()
    } else {
        normalized
    }
}
