//! Supervised 7-Zip jobs. Output is staged privately and published only after
//! a successful exit; the caller owns conflict handling and final placement.

use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use favnyr_core::openers::Opener;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Compress,
    ExtractHere,
    ExtractFolder,
}

/// Legacy recipes are recognized by executable AND exact argument template,
/// never by their translated label or decorative icon. Edited commands keep
/// their custom semantics.
pub fn action(opener: &Opener) -> Option<Action> {
    if opener.assoc.is_some() {
        return None;
    }
    if opener.archive_dialog {
        return Some(Action::Compress);
    }
    // Elevated custom commands retain the OS-managed launch path: a UAC
    // process cannot be supervised through these ordinary child pipes.
    if opener.elevated {
        return None;
    }
    let program = crate::actions::expand_program_path(&opener.program);
    let name = program.file_name()?.to_str()?.to_ascii_lowercase();
    if !matches!(
        name.as_str(),
        "7z" | "7zz" | "7za" | "7z.exe" | "7zz.exe" | "7za.exe"
    ) {
        return None;
    }
    match opener
        .args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["x", "-o{dir}", "{file}"] => Some(Action::ExtractHere),
        ["x", "-o{dir}/{stem}", "{file}"] => Some(Action::ExtractFolder),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Zip,
    SevenZip,
    Tar,
}

impl Format {
    pub fn from_index(index: i32) -> Self {
        match index {
            1 => Self::SevenZip,
            2 => Self::Tar,
            _ => Self::Zip,
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Zip => ".zip",
            Self::SevenZip => ".7z",
            Self::Tar => ".tar",
        }
    }

    pub fn supports_password(self) -> bool {
        self != Self::Tar
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TarCompression {
    #[default]
    None,
    Gzip,
    Xz,
    Bzip2,
}

impl TarCompression {
    pub fn from_index(index: i32) -> Self {
        match index {
            1 => Self::Gzip,
            2 => Self::Xz,
            3 => Self::Bzip2,
            _ => Self::None,
        }
    }

    fn archive_type(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Gzip => Some("gzip"),
            Self::Xz => Some("xz"),
            Self::Bzip2 => Some("bzip2"),
        }
    }

    fn suffix(self) -> &'static str {
        match self {
            Self::None => ".tar",
            Self::Gzip => ".tar.gz",
            Self::Xz => ".tar.xz",
            Self::Bzip2 => ".tar.bz2",
        }
    }
}

/// Builds the actual output filename from the editable base and the selected
/// format. A supported suffix typed by the user is replaced, since the format
/// selector is the single source of truth for the extension.
pub fn output_name(input: &str, format: Format, tar_compression: TarCompression) -> Option<String> {
    let input = input.trim();
    let mut base = input;
    loop {
        let lowercase = base.to_ascii_lowercase();
        let Some(suffix) = [
            ".tar.bz2", ".tar.gz", ".tar.xz", ".bzip2", ".gzip", ".tbz2", ".zip", ".7z", ".tar",
            ".bz2", ".tgz", ".txz", ".tbz", ".gz", ".xz",
        ]
        .into_iter()
        .find(|suffix| lowercase.ends_with(suffix)) else {
            break;
        };
        base = &base[..base.len() - suffix.len()];
    }
    if base.is_empty() || favnyr_core::fs::ops::check_file_name(base).is_err() {
        return None;
    }
    let suffix = if format == Format::Tar {
        tar_compression.suffix()
    } else {
        format.extension()
    };
    let result = format!("{base}{suffix}");
    (favnyr_core::fs::ops::check_file_name(&result).is_ok()
        && archive_component_length_is_valid(&result))
    .then_some(result)
}

fn archive_component_length_is_valid(name: &str) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        std::ffi::OsStr::new(name).as_bytes().len() <= 255
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        std::ffi::OsStr::new(name).encode_wide().count() <= 255
    }
    #[cfg(not(any(unix, windows)))]
    {
        name.chars().count() <= 255
    }
}

#[derive(Clone)]
pub enum Kind {
    Compress {
        name: String,
        format: Format,
        tar_compression: TarCompression,
        level: u8,
    },
    Extract {
        folder: bool,
    },
}

#[derive(Clone)]
pub struct Job {
    pub program: PathBuf,
    pub inputs: Vec<PathBuf>,
    pub kind: Kind,
}

/// An exclusively created directory. Only this owned tree may be cleaned up.
pub struct Staging(pub PathBuf);

impl Staging {
    fn create() -> std::io::Result<Self> {
        static SERIAL: AtomicU64 = AtomicU64::new(0);
        for _ in 0..100 {
            let path = std::env::temp_dir().join(format!(
                "favnyr-archive-{}-{}",
                std::process::id(),
                SERIAL.fetch_add(1, Ordering::Relaxed)
            ));
            let builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            let builder = {
                use std::os::unix::fs::DirBuilderExt;
                let mut builder = builder;
                builder.mode(0o700);
                builder
            };
            match builder.create(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(err) => return Err(err),
            }
        }
        Err(std::io::Error::other(
            "cannot allocate archive staging directory",
        ))
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        if self.0.as_os_str().is_empty() {
            return;
        }
        // Cleanup normally runs on a worker. A discarded UI delivery must not
        // freeze rendering while removing a large extracted tree.
        let path = self.0.clone();
        let fallback = path.clone();
        if std::thread::Builder::new()
            .name("archive-cleanup".into())
            .spawn(move || {
                let _ = std::fs::remove_dir_all(path);
            })
            .is_err()
        {
            let _ = std::fs::remove_dir_all(fallback);
        }
    }
}

pub struct Output {
    pub staging: Staging,
    pub sources: Vec<PathBuf>,
    pub destination: PathBuf,
}

pub enum Outcome {
    Ready(Output),
    PasswordRequired,
    Cancelled,
    Failed,
}

enum ProcessOutcome {
    Success,
    PasswordRequired,
    Cancelled,
    Failed,
}

pub fn valid_password(password: &str) -> bool {
    password.len() <= 1024 && !password.contains(['\r', '\n', '\0'])
}

/// ZIP AES creation in 7-Zip accepts only short, simple ASCII passwords.
/// Keep Unicode available for 7z creation and existing archive extraction.
pub fn valid_zip_password(password: &str) -> bool {
    password.len() <= 99 && password.bytes().all(|byte| (0x20..=0x7f).contains(&byte))
}

/// Reads bounded chunks concurrently from both pipes. A full stderr pipe must
/// never deadlock a child while the worker waits for stdout or cancellation.
fn read_pipe(mut pipe: impl Read + Send + 'static, tx: mpsc::SyncSender<Vec<u8>>) {
    std::thread::spawn(move || {
        let mut bytes = [0; 4096];
        loop {
            match pipe.read(&mut bytes) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send(bytes[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });
}

#[derive(Default)]
struct Diagnostics {
    line: Vec<u8>,
    password_prompt: bool,
    wrong_password: bool,
    progress: Option<u8>,
}

impl Diagnostics {
    fn feed(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            if matches!(byte, b'\r' | b'\n' | 8) {
                self.inspect();
                self.line.clear();
            } else if self.line.len() < 8192 {
                self.line.push(byte);
            }
        }
        // Password prompts have no terminating newline before stdin is read.
        self.inspect();
    }

    fn inspect(&mut self) {
        let line = String::from_utf8_lossy(&self.line).to_ascii_lowercase();
        self.password_prompt |= line.contains("enter password");
        self.wrong_password |=
            line.contains("wrong password") || line.contains("password is incorrect");
        // Progress records start with an integer percentage. Names later in
        // the record are deliberately not parsed as progress.
        if let Some((number, _)) = line.trim_start().split_once('%')
            && let Ok(value) = number.parse::<u8>()
            && value <= 100
        {
            self.progress = Some(value);
        }
    }
}

/// The password travels over stdin, never in argv, a config file or a log.
/// Both stdout and stderr are drained until EOF after the child is reaped.
fn execute(
    job: &Job,
    args: &[OsString],
    cwd: &Path,
    password: &str,
    cancel: &AtomicBool,
    progress: &mut impl FnMut(Option<u8>),
) -> std::io::Result<ProcessOutcome> {
    let mut command = Command::new(&job.program);
    command
        .args(args)
        .current_dir(cwd)
        .env("LC_ALL", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let mut child = command.spawn()?;
    let (tx, rx) = mpsc::sync_channel(16);
    read_pipe(child.stdout.take().expect("piped stdout"), tx.clone());
    // Each stream keeps its own parser: interleaved chunks must not corrupt
    // an error message or a percentage arriving on the other pipe.
    let (err_tx, err_rx) = mpsc::sync_channel(16);
    read_pipe(child.stderr.take().expect("piped stderr"), err_tx);
    drop(tx);
    if let Some(mut stdin) = child.stdin.take()
        && !password.is_empty()
    {
        // Creation may ask for confirmation; extraction consumes one line.
        // The bounded input fits the pipe even when no password is needed.
        let _ = writeln!(stdin, "{password}\n{password}");
    }
    let mut out = Diagnostics::default();
    let mut err = Diagnostics::default();
    let mut cancelled = false;
    let status = loop {
        if cancel.load(Ordering::Relaxed) {
            cancelled = true;
            let _ = child.kill();
        }
        while let Ok(bytes) = err_rx.try_recv() {
            err.feed(&bytes);
        }
        match rx.recv_timeout(Duration::from_millis(40)) {
            Ok(bytes) => out.feed(&bytes),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                std::thread::sleep(Duration::from_millis(20))
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        progress(out.progress.or(err.progress));
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        }
    };
    // Both queues are bounded: drain them together until their readers exit.
    loop {
        let mut live = false;
        for (queue, parser) in [(&rx, &mut out), (&err_rx, &mut err)] {
            loop {
                match queue.try_recv() {
                    Ok(bytes) => parser.feed(&bytes),
                    Err(mpsc::TryRecvError::Empty) => {
                        live = true;
                        break;
                    }
                    Err(mpsc::TryRecvError::Disconnected) => break,
                }
            }
        }
        if !live {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    if cancelled {
        return Ok(ProcessOutcome::Cancelled);
    }
    if status.success() {
        return Ok(ProcessOutcome::Success);
    }
    if matches!(job.kind, Kind::Extract { .. })
        && (out.wrong_password
            || err.wrong_password
            || (password.is_empty() && (out.password_prompt || err.password_prompt)))
    {
        return Ok(ProcessOutcome::PasswordRequired);
    }
    tracing::warn!(code = ?status.code(), "7-Zip operation failed");
    Ok(ProcessOutcome::Failed)
}

/// Rejects links that could escape the private tree during publication.
/// Directory traversal never follows symlinks, including directory links.
fn validate_tree(root: &Path, cancel: &AtomicBool) -> std::io::Result<()> {
    let canonical_root = std::fs::canonicalize(root)?;
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        if cancel.load(Ordering::Relaxed) {
            return Err(std::io::Error::other("cancelled"));
        }
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_symlink() {
                let target = std::fs::read_link(entry.path())?;
                let mut depth = dir.strip_prefix(root).unwrap().components().count();
                for part in target.components() {
                    match part {
                        std::path::Component::Normal(_) => depth += 1,
                        std::path::Component::CurDir => {}
                        std::path::Component::ParentDir if depth > 0 => depth -= 1,
                        _ => return Err(std::io::Error::other("unsafe archive link")),
                    }
                }
                if !std::fs::canonicalize(entry.path())?.starts_with(&canonical_root) {
                    return Err(std::io::Error::other(
                        "archive link leaves staging directory",
                    ));
                }
            } else if kind.is_dir() {
                pending.push(entry.path());
            } else if !kind.is_file() {
                return Err(std::io::Error::other("unsupported archive entry"));
            }
        }
    }
    Ok(())
}

pub fn run(
    job: &Job,
    password: &str,
    cancel: &AtomicBool,
    mut progress: impl FnMut(Option<u8>),
) -> Outcome {
    let result = (|| -> std::io::Result<Outcome> {
        if cancel.load(Ordering::Relaxed) {
            return Ok(Outcome::Cancelled);
        }
        if !valid_password(password) {
            return Ok(Outcome::Failed);
        }
        let first = job
            .inputs
            .first()
            .ok_or_else(|| std::io::Error::other("empty archive selection"))?;
        let destination = first
            .parent()
            .ok_or_else(|| std::io::Error::other("missing parent"))?
            .to_path_buf();
        let staging = Staging::create()?;
        let payload = staging.0.join("payload");
        std::fs::create_dir(&payload)?;
        let base_args = || -> Vec<OsString> {
            ["-bsp1", "-bso1", "-bse2", "-bb0", "-sccUTF-8", "-y"]
                .into_iter()
                .map(Into::into)
                .collect()
        };
        let mut args = base_args();
        let mut intermediate_tar = None;
        let mut split_progress = false;
        let sources = match &job.kind {
            Kind::Compress {
                name,
                format,
                tar_compression,
                level,
            } => {
                let Some(output_name) = output_name(name, *format, *tar_compression) else {
                    return Ok(Outcome::Failed);
                };
                if output_name != *name
                    || (*format == Format::Zip && !valid_zip_password(password))
                    || (!format.supports_password() && !password.is_empty())
                    || (*format != Format::Tar && *tar_compression != TarCompression::None)
                    || ![0, 1, 3, 5, 7, 9].contains(level)
                    || job
                        .inputs
                        .iter()
                        .any(|p| p.parent() != Some(destination.as_path()))
                {
                    return Ok(Outcome::Failed);
                }
                let archive = payload.join(&output_name);
                args.insert(0, "a".into());
                args.push(format!("-t{}", &format.extension()[1..]).into());
                if *format != Format::Tar {
                    args.push(format!("-mx={level}").into());
                }
                if !password.is_empty() {
                    args.push("-p".into());
                    args.push(
                        if *format == Format::Zip {
                            "-mem=AES256"
                        } else {
                            "-mhe=on"
                        }
                        .into(),
                    );
                }
                args.extend([
                    OsString::from("-spd"),
                    OsString::from("--"),
                    archive.clone().into_os_string(),
                ]);
                for path in &job.inputs {
                    // './' prevents a filename beginning with '@' from being
                    // interpreted as a 7-Zip listfile even after '--'.
                    args.push(
                        Path::new(".")
                            .join(path.file_name().unwrap())
                            .into_os_string(),
                    );
                }
                if let Some(archive_type) = tar_compression.archive_type() {
                    // 7-Zip exposes TAR and its stream compressors as separate
                    // formats. Create the container first, then compress that
                    // single file. Both outputs stay private until validation.
                    let tar = archive.with_extension("");
                    let archive_index = args.len() - job.inputs.len() - 1;
                    args[archive_index] = tar.clone().into_os_string();
                    let mut first_half = |value: Option<u8>| {
                        progress(value.map(|percent| percent / 2));
                    };
                    match execute(job, &args, &destination, "", cancel, &mut first_half)? {
                        ProcessOutcome::Success => {}
                        ProcessOutcome::Cancelled => return Ok(Outcome::Cancelled),
                        ProcessOutcome::PasswordRequired | ProcessOutcome::Failed => {
                            return Ok(Outcome::Failed);
                        }
                    }
                    args = base_args();
                    args.insert(0, "a".into());
                    args.push(format!("-t{archive_type}").into());
                    args.push(format!("-mx={level}").into());
                    args.extend([
                        OsString::from("-spd"),
                        OsString::from("--"),
                        archive.clone().into_os_string(),
                        tar.clone().into_os_string(),
                    ]);
                    intermediate_tar = Some(tar);
                    split_progress = true;
                }
                vec![archive]
            }
            Kind::Extract { folder } => {
                let output = if *folder {
                    let name = first
                        .file_stem()
                        .ok_or_else(|| std::io::Error::other("missing archive name"))?;
                    let folder = payload.join(name);
                    std::fs::create_dir(&folder)?;
                    folder
                } else {
                    payload.clone()
                };
                args.insert(0, "x".into());
                args.push("-aou".into());
                let mut output_arg = OsString::from("-o");
                output_arg.push(&output);
                args.extend([output_arg, "--".into(), first.clone().into_os_string()]);
                Vec::new()
            }
        };
        let mut final_progress = |value: Option<u8>| {
            progress(value.map(|percent| {
                if split_progress {
                    50 + percent / 2
                } else {
                    percent
                }
            }));
        };
        match execute(
            job,
            &args,
            &destination,
            password,
            cancel,
            &mut final_progress,
        )? {
            ProcessOutcome::Success => {}
            ProcessOutcome::PasswordRequired => return Ok(Outcome::PasswordRequired),
            ProcessOutcome::Cancelled => return Ok(Outcome::Cancelled),
            ProcessOutcome::Failed => return Ok(Outcome::Failed),
        }
        if let Some(path) = intermediate_tar {
            std::fs::remove_file(path)?;
        }
        validate_tree(&payload, cancel)?;
        if cancel.load(Ordering::Relaxed) {
            return Ok(Outcome::Cancelled);
        }
        let sources = if sources.is_empty() {
            std::fs::read_dir(&payload)?
                .map(|entry| entry.map(|entry| entry.path()))
                .collect::<std::io::Result<Vec<_>>>()?
        } else {
            sources
        };
        Ok(Outcome::Ready(Output {
            staging,
            sources,
            destination,
        }))
    })();
    match result {
        Ok(outcome) => outcome,
        Err(error) => {
            tracing::warn!(error = %error, "archive operation failed");
            if cancel.load(Ordering::Relaxed) {
                Outcome::Cancelled
            } else {
                Outcome::Failed
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_detection_does_not_capture_customized_commands() {
        let mut store = favnyr_core::openers::OpenerStore::default();
        let id = store.add(
            "Archive command",
            "7z",
            vec!["x".into(), "-o{dir}".into(), "{file}".into()],
        );
        let mut opener = store.get(&id).unwrap().clone();
        assert_eq!(action(&opener), Some(Action::ExtractHere));
        opener.args[1] = "-o{dir}/{stem}".into();
        assert_eq!(action(&opener), Some(Action::ExtractFolder));
        opener.args.push("-pCUSTOM".into());
        assert_eq!(action(&opener), None);
        opener.program = "other-tool".into();
        opener.args.pop();
        assert_eq!(action(&opener), None);
    }

    #[test]
    fn diagnostics_accept_split_prompts_and_terminal_progress() {
        let mut parser = Diagnostics::default();
        parser.feed(b"\r  42% 2 - file01.bin\x08\x08\r");
        assert_eq!(parser.progress, Some(42));
        parser.feed(b"Enter pass");
        parser.feed(b"word:");
        assert!(parser.password_prompt);
        parser.feed(b"\nERROR: Wrong pass");
        parser.feed(b"word\n");
        assert!(parser.wrong_password);
        assert!(!valid_password("line01\nline02"));
        assert!(valid_password(" sample ! é \" "));
        assert!(valid_zip_password(" sample ! \" "));
        assert!(!valid_zip_password("é"));
        assert!(!valid_zip_password(&"a".repeat(100)));
    }

    fn program() -> PathBuf {
        std::env::var_os("FAVNYR_TEST_7ZIP")
            .map(PathBuf::from)
            .expect("set FAVNYR_TEST_7ZIP to the console executable")
    }

    #[test]
    fn format_indexes_match_the_popup_and_encryption_capabilities() {
        assert_eq!(Format::from_index(0), Format::Zip);
        assert_eq!(Format::from_index(1), Format::SevenZip);
        assert_eq!(Format::from_index(2), Format::Tar);
        assert_eq!(TarCompression::from_index(0), TarCompression::None);
        assert_eq!(TarCompression::from_index(1), TarCompression::Gzip);
        assert_eq!(TarCompression::from_index(2), TarCompression::Xz);
        assert_eq!(TarCompression::from_index(3), TarCompression::Bzip2);
        assert!(!Format::Tar.supports_password());
        assert!(Format::Zip.supports_password());
        assert!(Format::SevenZip.supports_password());
    }

    #[test]
    fn output_names_require_a_real_base_and_follow_the_selected_format() {
        assert_eq!(
            output_name("archive", Format::Zip, TarCompression::None).as_deref(),
            Some("archive.zip")
        );
        assert_eq!(
            output_name(" archive.ZIP ", Format::SevenZip, TarCompression::None).as_deref(),
            Some("archive.7z")
        );
        assert_eq!(
            output_name("archive.7z", Format::Tar, TarCompression::None).as_deref(),
            Some("archive.tar")
        );
        assert_eq!(
            output_name("archive.tar", Format::Tar, TarCompression::Gzip).as_deref(),
            Some("archive.tar.gz")
        );
        assert_eq!(
            output_name("archive.tar.gz", Format::Tar, TarCompression::Xz).as_deref(),
            Some("archive.tar.xz")
        );
        assert_eq!(
            output_name("archive.tar.xz", Format::Tar, TarCompression::Bzip2).as_deref(),
            Some("archive.tar.bz2")
        );
        assert_eq!(
            output_name("archive.tar.7z", Format::Zip, TarCompression::None).as_deref(),
            Some("archive.zip")
        );
        assert_eq!(
            output_name(".hidden", Format::Zip, TarCompression::None).as_deref(),
            Some(".hidden.zip")
        );
        assert!(output_name(&"a".repeat(251), Format::Zip, TarCompression::None).is_some());
        assert!(output_name(&"a".repeat(252), Format::Zip, TarCompression::None).is_none());
        assert!(output_name(&"a".repeat(247), Format::Tar, TarCompression::Bzip2).is_some());
        assert!(output_name(&"a".repeat(248), Format::Tar, TarCompression::Bzip2).is_none());
        for invalid in [
            "",
            "   ",
            ".zip",
            ".ZIP",
            ".7z",
            ".tar",
            ".tar.gz",
            ".tar.xz",
            ".tar.bz2",
            ".tgz",
            ".txz",
            ".tbz2",
            ".zip.zip",
            ".",
            "..",
            "folder/name",
            "folder\\name",
            "line\nbreak",
        ] {
            assert!(
                output_name(invalid, Format::Zip, TarCompression::None).is_none(),
                "accepted {invalid:?}"
            );
        }
    }

    #[test]
    fn an_extension_only_name_is_rejected_before_launching_the_program() {
        let fixture = Staging::create().unwrap();
        let input = fixture.0.join("my_file.txt");
        std::fs::write(&input, b"data").unwrap();
        let job = Job {
            program: fixture.0.join("missing-program"),
            inputs: vec![input],
            kind: Kind::Compress {
                name: ".zip".into(),
                format: Format::Zip,
                tar_compression: TarCompression::None,
                level: 5,
            },
        };
        assert!(matches!(
            run(&job, "", &AtomicBool::new(false), |_| {}),
            Outcome::Failed
        ));
    }

    #[cfg(unix)]
    #[test]
    fn staged_links_must_stay_within_the_published_tree() {
        use std::os::unix::fs::symlink;
        let fixture = Staging::create().unwrap();
        let payload = fixture.0.join("payload");
        std::fs::create_dir(&payload).unwrap();
        std::fs::write(payload.join("my_file.txt"), b"data").unwrap();
        symlink("my_file.txt", payload.join("link01")).unwrap();
        let cancel = AtomicBool::new(false);
        assert!(validate_tree(&payload, &cancel).is_ok());
        symlink("../outside.txt", payload.join("link02")).unwrap();
        assert!(validate_tree(&payload, &cancel).is_err());
    }

    #[test]
    #[ignore = "requires FAVNYR_TEST_7ZIP"]
    fn real_multi_selection_preserves_directories_and_literal_filenames() {
        let fixture = Staging::create().unwrap();
        let names = ["@list.txt", "-option.txt", "file [01] é.txt"];
        let mut inputs = Vec::new();
        for name in names {
            let path = fixture.0.join(name);
            std::fs::write(&path, name).unwrap();
            inputs.push(path);
        }
        let folder = fixture.0.join("folder01");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("my_file.txt"), b"nested data").unwrap();
        inputs.push(folder);
        let job = Job {
            program: program(),
            inputs,
            kind: Kind::Compress {
                name: "archive.7z".into(),
                format: Format::SevenZip,
                tar_compression: TarCompression::None,
                level: 5,
            },
        };
        let cancel = AtomicBool::new(false);
        let Outcome::Ready(created) = run(&job, "", &cancel, |_| {}) else {
            panic!("compression failed")
        };
        let extraction = Job {
            program: program(),
            inputs: created.sources.clone(),
            kind: Kind::Extract { folder: false },
        };
        let Outcome::Ready(extracted) = run(&extraction, "", &cancel, |_| {}) else {
            panic!("extraction failed")
        };
        let payload = extracted.staging.0.join("payload");
        for name in names {
            assert_eq!(std::fs::read(payload.join(name)).unwrap(), name.as_bytes());
        }
        assert_eq!(
            std::fs::read(payload.join("folder01/my_file.txt")).unwrap(),
            b"nested data"
        );
    }

    #[test]
    #[ignore = "requires FAVNYR_TEST_7ZIP"]
    fn real_archives_password_retry_preserves_source_and_destination() {
        let fixture = Staging::create().unwrap();
        let input = fixture.0.join("my file.txt");
        let data = b"archive test content";
        std::fs::write(&input, data).unwrap();
        let cancel = AtomicBool::new(false);
        for format in [Format::SevenZip, Format::Zip, Format::Tar] {
            let encrypted_password = if format == Format::Zip {
                " sample ! \" "
            } else {
                " sample ! é \" "
            };
            for password in ["", encrypted_password] {
                let job = Job {
                    program: program(),
                    inputs: vec![input.clone()],
                    kind: Kind::Compress {
                        name: format!("archive{}", format.extension()),
                        format,
                        tar_compression: TarCompression::None,
                        level: 5,
                    },
                };
                if !format.supports_password() && !password.is_empty() {
                    // TAR must fail instead of silently dropping encryption.
                    assert!(matches!(
                        run(&job, password, &cancel, |_| {}),
                        Outcome::Failed
                    ));
                    continue;
                }
                let Outcome::Ready(created) = run(&job, password, &cancel, |_| {}) else {
                    panic!(
                        "compression failed: format={format:?}, encrypted={}",
                        !password.is_empty()
                    );
                };
                let archive = created.sources[0].clone();
                assert!(archive.is_file());
                for folder in [false, true] {
                    let extraction = Job {
                        program: program(),
                        inputs: vec![archive.clone()],
                        kind: Kind::Extract { folder },
                    };
                    if !password.is_empty() {
                        assert!(matches!(
                            run(&extraction, "", &cancel, |_| {}),
                            Outcome::PasswordRequired
                        ));
                        assert!(matches!(
                            run(&extraction, "incorrect", &cancel, |_| {}),
                            Outcome::PasswordRequired
                        ));
                    }
                    let Outcome::Ready(extracted) = run(&extraction, password, &cancel, |_| {})
                    else {
                        panic!("extraction failed");
                    };
                    let file = if folder {
                        extracted.sources[0].join("my file.txt")
                    } else {
                        extracted.sources[0].clone()
                    };
                    assert_eq!(std::fs::read(file).unwrap(), data);
                    // Extraction alone never writes to the final destination.
                    assert!(!created.staging.0.join("payload/my file.txt").exists());
                    assert_eq!(std::fs::read(&input).unwrap(), data);
                }
            }
        }
    }

    #[test]
    #[ignore = "requires FAVNYR_TEST_7ZIP"]
    fn real_compressed_tar_variants_are_created_without_touching_the_source() {
        let fixture = Staging::create().unwrap();
        let input = fixture.0.join("my_file.txt");
        let data = b"compressed tar content";
        std::fs::write(&input, data).unwrap();
        let cancel = AtomicBool::new(false);

        for compression in [
            TarCompression::Gzip,
            TarCompression::Xz,
            TarCompression::Bzip2,
        ] {
            let name = output_name("archive", Format::Tar, compression).unwrap();
            let job = Job {
                program: program(),
                inputs: vec![input.clone()],
                kind: Kind::Compress {
                    name,
                    format: Format::Tar,
                    tar_compression: compression,
                    level: 5,
                },
            };
            let Outcome::Ready(created) = run(&job, "", &cancel, |_| {}) else {
                panic!("compression failed: {compression:?}")
            };
            assert!(created.sources[0].is_file());
            assert_eq!(std::fs::read(&input).unwrap(), data);
        }
    }

    #[test]
    #[ignore = "requires FAVNYR_TEST_7ZIP"]
    fn real_corrupt_archive_and_cancel_do_not_publish_output() {
        let fixture = Staging::create().unwrap();
        let archive = fixture.0.join("broken.zip");
        std::fs::write(&archive, b"not an archive").unwrap();
        let cancel = AtomicBool::new(false);
        let mut job = Job {
            program: program(),
            inputs: vec![archive],
            kind: Kind::Extract { folder: false },
        };
        assert!(matches!(run(&job, "", &cancel, |_| {}), Outcome::Failed));
        let input = fixture.0.join("file01.bin");
        std::fs::write(&input, vec![42; 16 * 1024 * 1024]).unwrap();
        job.inputs = vec![input.clone()];
        job.kind = Kind::Compress {
            name: "cancelled.7z".into(),
            format: Format::SevenZip,
            tar_compression: TarCompression::None,
            level: 9,
        };
        assert!(matches!(
            run(&job, "", &cancel, |_| cancel.store(true, Ordering::Relaxed)),
            Outcome::Cancelled
        ));
        assert!(!fixture.0.join("cancelled.7z").exists());
        assert_eq!(std::fs::metadata(input).unwrap().len(), 16 * 1024 * 1024);
    }
}
