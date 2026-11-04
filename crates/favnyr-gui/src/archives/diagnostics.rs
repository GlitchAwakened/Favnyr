//! Bounded parsing of console progress and the first item-level diagnostic.

use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
};

#[derive(Debug, Default)]
pub struct Failure {
    pub path: Option<PathBuf>,
    pub kind: Option<ErrorKind>,
}

#[derive(Default)]
pub(super) struct Diagnostics {
    line: Vec<u8>,
    pub password_prompt: bool,
    pub wrong_password: bool,
    pub progress: Option<u8>,
    first: Option<Failure>,
    pending: Option<Option<ErrorKind>>,
    truncated: bool,
}

fn reason(text: &str) -> Option<ErrorKind> {
    let text = text.to_ascii_lowercase();
    if text.contains("used by another process")
        || text.contains("sharing violation")
        || text.contains("resource busy")
    {
        Some(ErrorKind::ResourceBusy)
    } else if text.contains("permission denied") || text.contains("access is denied") {
        Some(ErrorKind::PermissionDenied)
    } else if text.contains("no space") || text.contains("disk is full") {
        Some(ErrorKind::StorageFull)
    } else if text.contains("cannot find") || text.contains("no such file") {
        Some(ErrorKind::NotFound)
    } else {
        None
    }
}

impl Diagnostics {
    pub fn feed(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            if matches!(byte, b'\r' | b'\n' | 8) {
                self.inspect();
                self.record();
                self.line.clear();
                self.truncated = false;
            } else if self.line.len() < 8192 {
                self.line.push(byte);
            } else {
                self.truncated = true;
            }
        }
        // Prompts can wait for stdin without terminating their line.
        self.inspect();
    }

    fn inspect(&mut self) {
        let line = String::from_utf8_lossy(&self.line).to_ascii_lowercase();
        self.password_prompt |= line.contains("enter password");
        self.wrong_password |=
            line.contains("wrong password") || line.contains("password is incorrect");
        if let Some((number, _)) = line.trim_start().split_once('%')
            && let Ok(value) = number.parse::<u8>()
            && value <= 100
        {
            self.progress = Some(value);
        }
    }

    fn record(&mut self) {
        if self.truncated || self.first.as_ref().is_some_and(|f| f.path.is_some()) {
            self.pending = None;
            return;
        }
        let Ok(line) = std::str::from_utf8(&self.line) else {
            self.pending = None;
            return;
        };
        let line = line.trim();
        if line.is_empty() {
            return;
        }
        if let Some(message) = line
            .strip_prefix("WARNING:")
            .or_else(|| line.strip_prefix("ERROR:"))
        {
            let kind = reason(message);
            self.first.get_or_insert(Failure { path: None, kind });
            // Colon-separated diagnostics put the item last; drive-letter colons
            // have no surrounding spaces and therefore remain part of the path.
            if let Some((_, path)) = message.rsplit_once(" : ")
                && [
                    "cannot open file",
                    "can not open file",
                    "can not open output file",
                    "data error",
                    "crc failed",
                ]
                .iter()
                .any(|prefix| {
                    message
                        .trim_start()
                        .to_ascii_lowercase()
                        .starts_with(prefix)
                })
                && reason(path).is_none()
                && valid_path_line(path)
            {
                self.first = Some(Failure {
                    path: Some(path.trim().into()),
                    kind,
                });
                self.pending = None;
            } else {
                self.pending = Some(kind);
            }
        } else if let Some(kind) = self.pending.take()
            && valid_path_line(line)
        {
            self.first = Some(Failure {
                path: Some(line.into()),
                kind,
            });
        }
    }

    pub fn finish(mut self, cwd: &Path) -> Option<Failure> {
        self.record();
        let mut failure = self.first?;
        if let Some(path) = &mut failure.path
            && path.is_relative()
        {
            *path = cwd.join(&*path);
        }
        Some(failure)
    }
}

fn valid_path_line(line: &str) -> bool {
    !line.is_empty()
        && !line.chars().any(char::is_control)
        && ![
            "WARNING",
            "ERROR",
            "Warnings:",
            "Errors:",
            "Files:",
            "Folders:",
            "Files read",
            "Archive size:",
            "Scan WARNINGS:",
            "System ERROR:",
            "Everything is Ok",
            "Enter password",
            "Break signaled",
        ]
        .iter()
        .any(|s| line.starts_with(s))
        && !line.split_once('%').is_some_and(|(prefix, suffix)| {
            prefix.trim().parse::<u8>().is_ok()
                && (suffix.is_empty() || suffix.starts_with(char::is_whitespace))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_streams_keep_the_first_complete_item_and_reason() {
        let mut parser = Diagnostics::default();
        parser.feed(b"WARNING: The process cannot access the file because it is being used by another process.\n\nfolder01/my_");
        parser.feed(b"file.txt\r\nWARNING: Access is denied\nfolder02/other.txt\n");
        let failure = parser.finish(Path::new("root")).unwrap();
        assert_eq!(
            failure.path.unwrap(),
            Path::new("root").join("folder01/my_file.txt")
        );
        assert_eq!(failure.kind, Some(ErrorKind::ResourceBusy));
    }

    #[test]
    fn inline_paths_eof_and_unicode_are_preserved() {
        let mut parser = Diagnostics::default();
        parser.feed("ERROR: Cannot open file : Permission denied : folder01/文件.txt".as_bytes());
        let failure = parser.finish(Path::new("root")).unwrap();
        assert_eq!(
            failure.path.unwrap(),
            Path::new("root").join("folder01/文件.txt")
        );
        assert_eq!(failure.kind, Some(ErrorKind::PermissionDenied));
    }

    #[test]
    fn summaries_and_truncated_lines_are_not_reported_as_paths() {
        let mut parser = Diagnostics::default();
        parser.feed(b"WARNING: Access is denied\nFiles: 2\n");
        assert!(parser.finish(Path::new("root")).unwrap().path.is_none());
        let mut parser = Diagnostics::default();
        parser.feed(b"WARNING: Access is denied\n");
        parser.feed(&vec![b'a'; 9000]);
        assert!(parser.finish(Path::new("root")).unwrap().path.is_none());
    }

    #[test]
    fn numeric_percent_filename_is_not_confused_with_progress() {
        let mut parser = Diagnostics::default();
        parser.feed(b"WARNING: Access is denied\n50%.txt\n");
        assert_eq!(
            parser.finish(Path::new("root")).unwrap().path,
            Some(Path::new("root").join("50%.txt"))
        );
    }
}
