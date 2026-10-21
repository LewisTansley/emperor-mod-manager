//! Manager log file. Windows GUI builds discard stderr, so diagnosis reports
//! read this file instead.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024;
const TAIL_BYTES: u64 = 256 * 1024;

pub fn init(log_path: Option<PathBuf>) {
    let file = log_path.and_then(|path| {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).ok()?;
        }
        RotatingFile::open(path, MAX_LOG_BYTES).ok()
    });
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .target(env_logger::Target::Pipe(Box::new(Tee {
            stderr: io::stderr(),
            file,
        })))
        .try_init();
}

/// Last warn/error lines from the manager log, for a support report.
pub fn recent_warn_lines(log_file: &Path) -> String {
    let mut lines = Vec::new();
    let backup = log_file.with_extension("log.1");
    if let Some(text) = read_tail(&backup) {
        lines.extend(warn_lines(&text));
    }
    if let Some(text) = read_tail(log_file) {
        lines.extend(warn_lines(&text));
    }
    let start = lines.len().saturating_sub(40);
    lines[start..].join("\n")
}

fn warn_lines(text: &str) -> Vec<String> {
    text.lines()
        .filter(|line| {
            let upper = line.to_ascii_uppercase();
            upper.contains(" WARN ") || upper.contains(" ERROR ")
        })
        .map(str::to_string)
        .collect()
}

fn read_tail(path: &Path) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(TAIL_BYTES);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if start > 0 {
        if let Some(idx) = text.find('\n') {
            text.drain(..=idx);
        }
    }
    Some(text)
}

struct RotatingFile {
    path: PathBuf,
    file: Option<File>,
    max_bytes: u64,
}

impl RotatingFile {
    fn open(path: PathBuf, max_bytes: u64) -> io::Result<Self> {
        if let Ok(meta) = fs::metadata(&path) {
            if meta.len() >= max_bytes {
                rotate_closed(&path)?;
            }
        }
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Self {
            path,
            file: Some(file),
            max_bytes,
        })
    }

    fn rotate_if_needed(&mut self) -> io::Result<()> {
        let len = self
            .file
            .as_ref()
            .ok_or_else(|| io::Error::other("log file is closed"))?
            .metadata()?
            .len();
        if len < self.max_bytes {
            return Ok(());
        }
        if let Some(file) = self.file.take() {
            let _ = file.sync_all();
            drop(file);
        }
        // Close the handle before renaming so Windows can rotate the file.
        let renamed = rotate_closed(&self.path);
        self.file = Some(
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)?,
        );
        renamed
    }
}

fn rotate_closed(path: &Path) -> io::Result<()> {
    let backup = path.with_extension("log.1");
    let _ = fs::remove_file(&backup);
    if path.exists() {
        fs::rename(path, &backup)?;
    }
    Ok(())
}

impl Write for RotatingFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.rotate_if_needed()?;
        self.file
            .as_mut()
            .ok_or_else(|| io::Error::other("log file is closed"))?
            .write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file
            .as_mut()
            .ok_or_else(|| io::Error::other("log file is closed"))?
            .flush()
    }
}

struct Tee {
    stderr: io::Stderr,
    file: Option<RotatingFile>,
}

impl Write for Tee {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let file_ok = if let Some(file) = self.file.as_mut() {
            file.write_all(buf).is_ok()
        } else {
            false
        };
        match self.stderr.write_all(buf) {
            Ok(()) => Ok(buf.len()),
            Err(_) if file_ok => Ok(buf.len()),
            Err(err) => Err(err),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Some(file) = self.file.as_mut() {
            let _ = file.flush();
        }
        self.stderr.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotates_once_past_the_size_cap() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("emperor.log");
        let mut log = RotatingFile::open(path.clone(), 64).unwrap();
        log.write_all(&[b'a'; 40]).unwrap();
        assert!(!path.with_extension("log.1").exists());
        log.write_all(&[b'b'; 40]).unwrap();
        log.write_all(b"c").unwrap();
        log.flush().unwrap();
        let backup = path.with_extension("log.1");
        assert!(backup.is_file(), "rotated log missing");
        assert!(fs::metadata(&backup).unwrap().len() >= 64);
        assert_eq!(fs::read(&path).unwrap(), b"c");
    }

    #[test]
    fn recent_warn_lines_keeps_warn_and_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("emperor.log");
        fs::write(
            &path,
            "[2026-01-01T00:00:00Z INFO  app] hello\n[2026-01-01T00:00:01Z WARN  app] missing loader\n[2026-01-01T00:00:02Z ERROR app] boom\n",
        )
        .unwrap();
        let text = recent_warn_lines(&path);
        assert!(text.contains("missing loader"));
        assert!(text.contains("boom"));
        assert!(!text.contains("hello"));
    }
}
