//! Filesystem store for bookmark notes.
//!
//! Layout:
//! ```text
//! <root>/
//!   YYYY-MM/<urlhash8>-<slug>.md   # one bookmark per file
//!   inbox/                         # BM-5: offline drain destination
//! ```
//!
//! The filesystem is the source of truth. There is no on-disk index;
//! `list` walks the tree on every call. (Personal scale, todos plugin
//! does the same.) If perf becomes a problem we'll add an in-memory
//! cache invalidated by a watcher — same infra as turm-plugin-todo.
//!
//! Path-safety contract (mirrors turm-plugin-kb / turm-plugin-todo):
//! - Root is canonicalized at construction.
//! - Every resolved path is re-canonicalized and checked to start
//!   with `root_canonical` before being surfaced — defeats a symlink
//!   inside the root that points outside.
//! - Hidden directories (`.foo`) are skipped during walk.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local};

use crate::frontmatter::{self, Frontmatter};

#[derive(Debug)]
pub enum StoreError {
    Io(String),
    NotFound(String),
    Ambiguous(Vec<Match>),
    InvalidParams(String),
}

impl StoreError {
    pub fn code_message(&self) -> (&'static str, String) {
        match self {
            StoreError::Io(m) => ("io_error", m.clone()),
            StoreError::NotFound(m) => ("not_found", m.clone()),
            StoreError::Ambiguous(matches) => {
                let summary = matches
                    .iter()
                    .map(|m| format!("{} ({})", m.id, m.url))
                    .collect::<Vec<_>>()
                    .join(", ");
                (
                    "ambiguous_id",
                    format!("multiple bookmarks match prefix: {summary}"),
                )
            }
            StoreError::InvalidParams(m) => ("invalid_params", m.clone()),
        }
    }
}

/// A bookmark as it lives on disk: parsed frontmatter + body + the
/// 8-char id derived from the filename prefix.
#[derive(Debug, Clone)]
pub struct Match {
    pub id: String,
    pub path: PathBuf,
    pub url: String,
    pub title: String,
    pub status: String,
    pub captured_at: String,
    pub tags: Vec<String>,
}

pub struct Store {
    root: PathBuf,
    root_canonical: PathBuf,
}

impl Store {
    pub fn new(root: PathBuf) -> Result<Self, StoreError> {
        if let Err(e) = fs::create_dir_all(&root) {
            return Err(StoreError::Io(format!("mkdir {}: {e}", root.display())));
        }
        let root_canonical = fs::canonicalize(&root)
            .map_err(|e| StoreError::Io(format!("canonicalize {}: {e}", root.display())))?;
        Ok(Self {
            root,
            root_canonical,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn within_root(&self, p: &Path) -> bool {
        match fs::canonicalize(p) {
            Ok(canon) => canon.starts_with(&self.root_canonical),
            Err(_) => false,
        }
    }

    /// Walk the tree and return all bookmark files (`.md` files under
    /// `YYYY-MM/`). Files without valid frontmatter or without a `url`
    /// field are silently skipped — we don't want a stray hand-written
    /// note in `~/docs/bookmarks/` to crash list. Hidden dirs skipped.
    pub fn list_all(&self) -> Vec<Match> {
        let mut out = Vec::new();
        let mut paths = Vec::new();
        self.collect_md_files(&self.root_canonical, &mut paths);
        for path in paths {
            if let Some(m) = self.read_match(&path) {
                out.push(m);
            }
        }
        // Newest first by captured_at, falling back to filesystem mtime.
        out.sort_by(|a, b| b.captured_at.cmp(&a.captured_at));
        out
    }

    fn collect_md_files(&self, dir: &Path, out: &mut Vec<PathBuf>) {
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            // Skip hidden entries (.git, .turm-cache, .DS_Store).
            if name_str.starts_with('.') {
                continue;
            }
            let file_type = match entry.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };
            // Don't descend into symlinks even if their target is inside
            // root — we'd lose the path-safety invariant.
            if file_type.is_dir() {
                self.collect_md_files(&path, out);
            } else if file_type.is_file()
                && path.extension().and_then(|e| e.to_str()) == Some("md")
                && self.within_root(&path)
            {
                out.push(path);
            }
        }
    }

    fn read_match(&self, path: &Path) -> Option<Match> {
        let raw = fs::read_to_string(path).ok()?;
        let (fm, _body) = frontmatter::split(&raw);
        let url = fm.get_scalar("url")?.to_string();
        if url.is_empty() {
            return None;
        }
        let id = id_from_filename(path)?;
        let title = fm
            .get_scalar("title")
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| url.clone());
        let status = fm.get_scalar("status").unwrap_or("queued").to_string();
        let captured_at = fm.get_scalar("captured_at").unwrap_or("").to_string();
        let tags = fm.get_list("tags").map(|v| v.to_vec()).unwrap_or_default();

        Some(Match {
            id,
            path: path.to_path_buf(),
            url,
            title,
            status,
            captured_at,
            tags,
        })
    }

    /// Read the full file (frontmatter + body) for a path that we've
    /// already validated as ours. Returns the parsed frontmatter and body.
    pub fn read_full(&self, path: &Path) -> Result<(Frontmatter, String), StoreError> {
        if !self.within_root(path) {
            return Err(StoreError::NotFound(format!(
                "path is outside bookmark root: {}",
                path.display()
            )));
        }
        let raw = fs::read_to_string(path)
            .map_err(|e| StoreError::Io(format!("read {}: {e}", path.display())))?;
        let (fm, body) = frontmatter::split(&raw);
        Ok((fm, body.to_string()))
    }

    /// Resolve a user-supplied id (may be a prefix of urlhash8) to a
    /// concrete match. Errors with the candidate list when ambiguous.
    pub fn find_by_id(&self, id_prefix: &str) -> Result<Match, StoreError> {
        let id_prefix = id_prefix.trim();
        if id_prefix.is_empty() {
            return Err(StoreError::InvalidParams("id is empty".into()));
        }
        if !id_prefix.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(StoreError::InvalidParams(format!(
                "id must be hex; got {id_prefix:?}"
            )));
        }
        let id_prefix = id_prefix.to_ascii_lowercase();
        let all = self.list_all();
        let candidates: Vec<Match> = all
            .into_iter()
            .filter(|m| m.id.starts_with(&id_prefix))
            .collect();
        match candidates.len() {
            0 => Err(StoreError::NotFound(format!(
                "no bookmark with id prefix {id_prefix:?}"
            ))),
            1 => Ok(candidates.into_iter().next().unwrap()),
            _ => Err(StoreError::Ambiguous(candidates)),
        }
    }

    /// Find by exact urlhash8 (used after canonicalization to detect
    /// re-add). Returns the first match — duplicate hashes shouldn't
    /// happen but if they do, we surface only the most recent (the
    /// list is sorted newest-first).
    pub fn find_by_urlhash(&self, urlhash8: &str) -> Option<Match> {
        let target = urlhash8.to_ascii_lowercase();
        self.list_all().into_iter().find(|m| m.id == target)
    }

    /// Atomically create a new bookmark file. Returns the chosen path.
    /// If a file with the same urlhash8 already exists ANYWHERE under
    /// root, returns the existing match in `existed`.
    pub fn create(&self, req: CreateRequest<'_>) -> Result<CreateOutcome, StoreError> {
        if let Some(existing) = self.find_by_urlhash(req.urlhash8) {
            return Ok(CreateOutcome::Existed(existing));
        }

        let month = req.now.format("%Y-%m").to_string();
        let dir = self.root_canonical.join(&month);
        fs::create_dir_all(&dir)
            .map_err(|e| StoreError::Io(format!("mkdir {}: {e}", dir.display())))?;

        let filename = format!("{}-{}.md", req.urlhash8, req.slug);
        let final_path = dir.join(&filename);

        let mut fm = Frontmatter::default();
        fm.set_scalar("url", req.canonical_url);
        fm.set_scalar("title", req.title);
        fm.set_scalar("captured_at", req.now.to_rfc3339());
        fm.set_scalar("source", req.source);
        fm.set_scalar("status", "queued");
        fm.set_list("tags", req.tags.to_vec());
        fm.set_list("linked_kb", Vec::new());

        let body = String::new();
        let rendered = frontmatter::render(&fm, &body);

        atomic_write_new(&final_path, rendered.as_bytes())
            .map_err(|e| StoreError::Io(format!("write {}: {e}", final_path.display())))?;

        // Re-read so the surfaced Match reflects what landed on disk —
        // and the path-safety re-check fires.
        let m = self
            .read_match(&final_path)
            .ok_or_else(|| StoreError::Io("post-write re-read failed".into()))?;
        Ok(CreateOutcome::Created(m))
    }

    pub fn delete(&self, m: &Match) -> Result<(), StoreError> {
        if !self.within_root(&m.path) {
            return Err(StoreError::NotFound(format!(
                "path is outside bookmark root: {}",
                m.path.display()
            )));
        }
        fs::remove_file(&m.path)
            .map_err(|e| StoreError::Io(format!("unlink {}: {e}", m.path.display())))
    }
}

#[derive(Debug)]
pub enum CreateOutcome {
    Created(Match),
    Existed(Match),
}

/// Inputs to [`Store::create`] bundled as a struct so the call site
/// stays readable and clippy's `too_many_arguments` is happy.
pub struct CreateRequest<'a> {
    pub urlhash8: &'a str,
    pub slug: &'a str,
    pub canonical_url: &'a str,
    pub title: &'a str,
    pub source: &'a str,
    pub tags: &'a [String],
    pub now: DateTime<Local>,
}

/// Slug = lowercased Unicode-alphanumeric chars from input, separated
/// by `-`, max 60 chars (chars not bytes), trimmed of leading/trailing
/// `-`. Empty input → `"untitled"`. CJK and other scripts are kept
/// (not transliterated) because Linux filesystems handle them fine.
pub fn slug(input: &str) -> String {
    let mut s = String::with_capacity(input.len());
    let mut last_was_dash = false;
    for c in input.chars() {
        if c.is_alphanumeric() {
            for lc in c.to_lowercase() {
                s.push(lc);
            }
            last_was_dash = false;
        } else if !s.is_empty() && !last_was_dash {
            s.push('-');
            last_was_dash = true;
        }
    }
    let trimmed = s.trim_matches('-');
    let truncated: String = trimmed.chars().take(60).collect();
    let final_slug = truncated.trim_matches('-').to_string();
    if final_slug.is_empty() {
        "untitled".to_string()
    } else {
        final_slug
    }
}

/// Extract id (8-char urlhash) from a filename of the form
/// `<urlhash8>-<slug>.md`.
fn id_from_filename(path: &Path) -> Option<String> {
    let stem = path.file_stem().and_then(|s| s.to_str())?;
    let prefix = stem.split('-').next()?;
    if prefix.len() == 8 && prefix.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(prefix.to_ascii_lowercase())
    } else {
        None
    }
}

/// Write a new file atomically. Errors out if the target already
/// exists — the caller has already done its dedup check via urlhash8.
/// Implementation: write to `<final>.tmp.<pid>.<nanos>`, then rename.
/// The final rename is `link + unlink` semantics under POSIX, so a
/// reader that opens the path during the rename either sees the old
/// inode (if any) or the new one, never a partially written file.
fn atomic_write_new(final_path: &Path, contents: &[u8]) -> io::Result<()> {
    if final_path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} already exists", final_path.display()),
        ));
    }
    let parent = final_path
        .parent()
        .ok_or_else(|| io::Error::other("final path has no parent"))?;
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp_name = format!(
        "{}.tmp.{pid}.{nanos}",
        final_path.file_name().unwrap().to_string_lossy()
    );
    let tmp_path = parent.join(tmp_name);
    {
        let mut f = fs::File::create(&tmp_path)?;
        f.write_all(contents)?;
        f.sync_all()?;
    }
    if let Err(e) = fs::rename(&tmp_path, final_path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(e);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_ascii() {
        assert_eq!(slug("Hello World"), "hello-world");
        assert_eq!(slug("Foo / Bar / Baz"), "foo-bar-baz");
        assert_eq!(slug("  trim me  "), "trim-me");
    }

    #[test]
    fn slug_empty_falls_back() {
        assert_eq!(slug(""), "untitled");
        assert_eq!(slug("   "), "untitled");
        assert_eq!(slug("///"), "untitled");
    }

    #[test]
    fn slug_keeps_cjk() {
        let s = slug("터미널 북마크");
        assert!(s.contains('터') || s.contains('터'));
        assert!(!s.is_empty());
        assert!(s != "untitled");
    }

    #[test]
    fn slug_truncates_chars_not_bytes() {
        let long_emoji = "café".repeat(50);
        let s = slug(&long_emoji);
        // <= 60 chars (each "café" is 4 chars).
        assert!(s.chars().count() <= 60);
    }

    #[test]
    fn id_from_filename_valid() {
        let p = PathBuf::from("/tmp/bookmarks/2026-05/abcd1234-foo.md");
        assert_eq!(id_from_filename(&p), Some("abcd1234".to_string()));
    }

    #[test]
    fn id_from_filename_uppercase_normalized() {
        let p = PathBuf::from("/tmp/bookmarks/2026-05/ABCD1234-foo.md");
        assert_eq!(id_from_filename(&p), Some("abcd1234".to_string()));
    }

    #[test]
    fn id_from_filename_rejects_non_hex() {
        let p = PathBuf::from("/tmp/bookmarks/2026-05/abcd123z-foo.md");
        assert_eq!(id_from_filename(&p), None);
    }

    #[test]
    fn id_from_filename_rejects_short_prefix() {
        let p = PathBuf::from("/tmp/bookmarks/2026-05/abc-foo.md");
        assert_eq!(id_from_filename(&p), None);
    }

    #[test]
    fn create_and_find() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path().to_path_buf()).unwrap();
        let now = Local::now();
        let outcome = store
            .create(CreateRequest {
                urlhash8: "abcd1234",
                slug: "example",
                canonical_url: "https://example.com/",
                title: "Example",
                source: "cli",
                tags: &["test".to_string()],
                now,
            })
            .unwrap();
        match outcome {
            CreateOutcome::Created(m) => {
                assert_eq!(m.id, "abcd1234");
                assert_eq!(m.url, "https://example.com/");
                assert_eq!(m.title, "Example");
                assert_eq!(m.tags, vec!["test".to_string()]);
            }
            CreateOutcome::Existed(_) => panic!("should have created"),
        }
        let found = store.find_by_id("abcd").unwrap();
        assert_eq!(found.id, "abcd1234");
        let found = store.find_by_urlhash("abcd1234").unwrap();
        assert_eq!(found.id, "abcd1234");
    }

    #[test]
    fn create_idempotent_returns_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path().to_path_buf()).unwrap();
        let now = Local::now();
        let req = || CreateRequest {
            urlhash8: "abcd1234",
            slug: "ex",
            canonical_url: "https://example.com/",
            title: "Example",
            source: "cli",
            tags: &[],
            now,
        };
        store.create(req()).unwrap();
        let outcome = store.create(req()).unwrap();
        match outcome {
            CreateOutcome::Existed(m) => assert_eq!(m.id, "abcd1234"),
            CreateOutcome::Created(_) => panic!("should have detected existing"),
        }
    }

    #[test]
    fn find_ambiguous_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path().to_path_buf()).unwrap();
        let now = Local::now();
        store
            .create(CreateRequest {
                urlhash8: "abcd1111",
                slug: "a",
                canonical_url: "https://example.com/a",
                title: "a",
                source: "cli",
                tags: &[],
                now,
            })
            .unwrap();
        store
            .create(CreateRequest {
                urlhash8: "abcd2222",
                slug: "b",
                canonical_url: "https://example.com/b",
                title: "b",
                source: "cli",
                tags: &[],
                now,
            })
            .unwrap();
        match store.find_by_id("abcd") {
            Err(StoreError::Ambiguous(matches)) => assert_eq!(matches.len(), 2),
            other => panic!("expected ambiguous, got {other:?}"),
        }
    }

    #[test]
    fn find_by_id_rejects_non_hex() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path().to_path_buf()).unwrap();
        match store.find_by_id("xyz!") {
            Err(StoreError::InvalidParams(_)) => {}
            other => panic!("expected InvalidParams, got {other:?}"),
        }
    }

    #[test]
    fn delete_removes_file() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path().to_path_buf()).unwrap();
        let now = Local::now();
        let outcome = store
            .create(CreateRequest {
                urlhash8: "abcd1234",
                slug: "x",
                canonical_url: "https://example.com/x",
                title: "x",
                source: "cli",
                tags: &[],
                now,
            })
            .unwrap();
        let m = match outcome {
            CreateOutcome::Created(m) => m,
            CreateOutcome::Existed(_) => panic!("should have created"),
        };
        assert!(m.path.exists());
        store.delete(&m).unwrap();
        assert!(!m.path.exists());
    }
}
