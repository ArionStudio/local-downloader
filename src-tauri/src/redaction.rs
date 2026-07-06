use regex::Regex;

pub fn sanitize_log_line(line: &str) -> String {
    let mut value = line.to_string();

    let patterns = [
        (r"(?i)\b(cookie\s*:\s*)[^\r\n]+", "$1[redacted]"),
        (r"(?i)\b(authorization\s*:\s*)[^\r\n]+", "$1[redacted]"),
        (
            r"(?i)(--cookies(?:-from-browser)?\s+)[^\s]+",
            "$1[redacted]",
        ),
        (r"(?i)(X-Amz-Signature=)[A-Za-z0-9%]+", "$1[redacted]"),
        (r"(?i)(Signature=)[A-Za-z0-9%]+", "$1[redacted]"),
        (r"(?i)(Policy=)[A-Za-z0-9%]+", "$1[redacted]"),
        (r"(?i)([?&]t=)[A-Za-z0-9%_.-]+", "$1[redacted]"),
    ];

    for (pattern, replacement) in patterns {
        if let Ok(regex) = Regex::new(pattern) {
            value = regex.replace_all(&value, replacement).into_owned();
        }
    }

    value
}
