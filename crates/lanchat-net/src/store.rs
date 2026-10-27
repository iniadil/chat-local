//! On-disk history: one JSON line per message, attachments as plain files.

use crate::types::Message;
use crate::Error;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

const MAX_IN_MEMORY: usize = 5000;

pub struct Store {
    log_path: PathBuf,
    files_dir: PathBuf,
    messages: Vec<Message>,
}

impl Store {
    pub fn open(data_dir: &Path) -> Result<Self, Error> {
        fs::create_dir_all(data_dir)?;
        let files_dir = data_dir.join("files");
        fs::create_dir_all(&files_dir)?;
        let log_path = data_dir.join("messages.jsonl");
        let mut messages = Vec::new();
        if log_path.exists() {
            let reader = BufReader::new(File::open(&log_path)?);
            for line in reader.lines() {
                let line = line?;
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<Message>(&line) {
                    Ok(m) => messages.push(m),
                    Err(e) => tracing::warn!("skipping unreadable history line: {e}"),
                }
            }
        }
        if messages.len() > MAX_IN_MEMORY {
            messages.drain(..messages.len() - MAX_IN_MEMORY);
        }
        Ok(Self {
            log_path,
            files_dir,
            messages,
        })
    }

    pub fn files_dir(&self) -> &Path {
        &self.files_dir
    }

    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    pub fn find(&self, id: &str) -> Option<&Message> {
        self.messages.iter().find(|m| m.id == id)
    }

    pub fn append(&mut self, message: Message) -> Result<(), Error> {
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)?;
        f.write_all(serde_json::to_string(&message)?.as_bytes())?;
        f.write_all(b"\n")?;
        self.messages.push(message);
        if self.messages.len() > MAX_IN_MEMORY {
            self.messages.remove(0);
        }
        Ok(())
    }

    /// Forget every message. Attachment files are left in place.
    pub fn clear(&mut self) -> Result<(), Error> {
        self.messages.clear();
        if self.log_path.exists() {
            fs::remove_file(&self.log_path)?;
        }
        Ok(())
    }

    /// Write attachment bytes to the files directory and return the path.
    pub fn save_attachment(
        &self,
        message_id: &str,
        file_name: &str,
        bytes: &[u8],
    ) -> Result<PathBuf, Error> {
        let short = message_id.chars().take(8).collect::<String>();
        let path = self
            .files_dir
            .join(format!("{short}-{}", sanitize_file_name(file_name)));
        fs::write(&path, bytes)?;
        Ok(path)
    }
}

/// Keep a file name safe to write anywhere: strip path separators and control
/// characters, and never produce an empty or dot-only name.
pub fn sanitize_file_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let cleaned: String = base
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.');
    if trimmed.is_empty() {
        "file".to_string()
    } else {
        trimmed.chars().take(120).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_names() {
        assert_eq!(sanitize_file_name("../../etc/passwd"), "passwd");
        assert_eq!(
            sanitize_file_name("C:\\x\\report:v2?.pdf"),
            "report_v2_.pdf"
        );
        assert_eq!(sanitize_file_name("..."), "file");
        assert_eq!(sanitize_file_name("  hi.txt "), "hi.txt");
    }

    #[test]
    fn persists_and_reloads() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut s = Store::open(dir.path()).unwrap();
            s.append(Message {
                id: "m1".into(),
                from: "a".into(),
                from_name: "A".into(),
                to: None,
                sent_at: 1,
                text: Some("hello".into()),
                attachment: None,
            })
            .unwrap();
        }
        let s = Store::open(dir.path()).unwrap();
        assert_eq!(s.messages().len(), 1);
        assert_eq!(s.messages()[0].text.as_deref(), Some("hello"));
    }
}
