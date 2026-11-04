use super::{Case, Issue, Options};
use crate::fs::ops::check_file_name;

pub(super) fn name(
    original: &str,
    directory: bool,
    index: usize,
    options: &Options,
) -> Result<String, Issue> {
    let (stem, extension) = split_name(original, directory);
    let mut name = options.common.as_deref().unwrap_or(stem).to_owned();
    if let Some(replacement) = &options.replacement {
        if replacement.find.is_empty() {
            return Err(Issue::InvalidOptions);
        }
        name = replace(
            &name,
            &replacement.find,
            &replacement.with,
            replacement.case_sensitive,
            replacement.all,
        );
    }
    name = match options.case {
        Case::Unchanged => name,
        Case::Lower => name.to_lowercase(),
        Case::Upper => name.to_uppercase(),
    };
    if let Some(numbering) = &options.numbering {
        if numbering.digits > 20 || numbering.step == 0 {
            return Err(Issue::InvalidOptions);
        }
        let number = (index as u64)
            .checked_mul(numbering.step)
            .and_then(|offset| numbering.start.checked_add(offset))
            .ok_or(Issue::InvalidOptions)?;
        let number = format!("{number:0width$}", width = numbering.digits);
        name = if numbering.before {
            format!("{number}{}{name}", numbering.separator)
        } else {
            format!("{name}{}{number}", numbering.separator)
        };
    }
    if let Some((prefix, suffix)) = &options.affixes {
        name = format!("{prefix}{name}{suffix}");
    }
    let extension = if directory {
        ""
    } else {
        options
            .extension
            .as_deref()
            .map(|ext| ext.strip_prefix('.').unwrap_or(ext))
            .unwrap_or(extension)
    };
    if !extension.is_empty() {
        name.push('.');
        name.push_str(extension);
    }
    check_file_name(&name).map_err(Issue::InvalidName)?;
    Ok(name)
}

fn split_name(name: &str, directory: bool) -> (&str, &str) {
    if directory {
        return (name, "");
    }
    let lower = name.to_ascii_lowercase();
    for extension in ["tar.gz", "tar.bz2", "tar.xz", "tar.zst"] {
        if lower.ends_with(&format!(".{extension}")) && name.len() > extension.len() + 1 {
            let at = name.len() - extension.len() - 1;
            return (&name[..at], &name[at + 1..]);
        }
    }
    match name.rfind('.') {
        Some(at) if at > 0 && at + 1 < name.len() => (&name[..at], &name[at + 1..]),
        _ => (name, ""),
    }
}

/// Lowercase matching preserves original UTF-8 boundaries and untouched casing.
fn replace(text: &str, find: &str, with: &str, sensitive: bool, all: bool) -> String {
    if sensitive {
        return text.replacen(find, with, if all { usize::MAX } else { 1 });
    }
    let mut folded = String::new();
    let mut boundaries = std::collections::HashMap::new();
    for (offset, ch) in text.char_indices() {
        boundaries.insert(folded.len(), offset);
        folded.extend(ch.to_lowercase());
    }
    boundaries.insert(folded.len(), text.len());
    let needle: String = find.chars().flat_map(char::to_lowercase).collect();
    let mut result = String::new();
    let mut copied = 0;
    for (start, _) in folded.match_indices(&needle) {
        let Some((&from, &to)) = boundaries
            .get(&start)
            .zip(boundaries.get(&(start + needle.len())))
        else {
            continue;
        };
        result.push_str(&text[copied..from]);
        result.push_str(with);
        copied = to;
        if !all {
            break;
        }
    }
    result.push_str(&text[copied..]);
    result
}
