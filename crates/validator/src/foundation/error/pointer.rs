//! Internal helpers for converting between dot/bracket paths and RFC 6901 JSON Pointers.

pub(crate) fn normalize_pointer(pointer: &str) -> Option<String> {
    let pointer = pointer.trim();
    if pointer.is_empty() || pointer == "#" {
        return None;
    }

    if let Some(rest) = pointer.strip_prefix("#") {
        return normalize_pointer(rest);
    }

    if pointer.starts_with('/') {
        return Some(pointer.to_owned());
    }

    None
}

pub(crate) fn to_json_pointer(path: &str) -> Option<String> {
    let path = path.trim();
    if path.is_empty() {
        return None;
    }

    if let Some(pointer) = normalize_pointer(path) {
        return Some(pointer);
    }

    let mut segments: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut chars = path.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '.' => {
                if !current.is_empty() {
                    segments.push(std::mem::take(&mut current));
                }
            },
            '[' => {
                if !current.is_empty() {
                    segments.push(std::mem::take(&mut current));
                }
                let mut idx = String::new();
                let mut closed = false;
                for c in chars.by_ref() {
                    if c == ']' {
                        closed = true;
                        break;
                    }
                    idx.push(c);
                }

                if closed && !idx.is_empty() {
                    segments.push(idx);
                } else {
                    // Unclosed bracket — treat `[` and contents as literal text.
                    current.push('[');
                    current.push_str(&idx);
                }
            },
            _ => current.push(ch),
        }
    }

    if !current.is_empty() {
        segments.push(current);
    }

    if segments.is_empty() {
        return None;
    }

    let pointer = segments
        .into_iter()
        .filter(|segment| !segment.is_empty())
        .map(|segment| segment.replace('~', "~0").replace('/', "~1"))
        .fold(String::new(), |mut acc, segment| {
            acc.push('/');
            acc.push_str(&segment);
            acc
        });

    if pointer.is_empty() {
        None
    } else {
        Some(pointer)
    }
}
