//! Argument-vector shell-outs to `git`.
//!
//! Every call goes through `Command::arg(...)` — never a shell
//! string — so caller-supplied data (branch names, base refs)
//! cannot inject extra arguments or shell metacharacters. We also
//! pass `--literal-pathspecs` and explicit `-C <repo>` everywhere
//! to keep the cwd of the calling process from changing what
//! `git` sees.
//!
//! Branch name sanitization happens BEFORE git sees the value:
//! `validate_branch_name` rejects empty / leading-dot / trailing
//! `.lock` / `..` / control chars, and refuses anything `git
//! check-ref-format` would also refuse (prefix the actual call to
//! avoid divergence on edge cases). The roadmap-specified Jira
//! transform (`PROJ-456 → proj-456`, slashes preserved as path
//! components) is in `sanitize_branch_for_jira` and is OPTIONAL —
//! callers that want it pass through that helper before
//! `worktree_add`, but `worktree_add` itself accepts any
//! validate-passing branch name verbatim.

use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug)]
pub struct GitError {
    pub code: &'static str,
    pub message: String,
}

impl GitError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct WorktreeInfo {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub head_sha: String,
    pub locked: bool,
    pub prunable: bool,
}

pub fn current_branch(repo: &Path) -> Result<String, GitError> {
    let out = run_git(repo, &["symbolic-ref", "--quiet", "--short", "HEAD"])?;
    let s = String::from_utf8(out.stdout)
        .map_err(|e| GitError::new("io_error", format!("decode HEAD: {e}")))?;
    let trimmed = s.trim().to_string();
    if trimmed.is_empty() {
        // Detached HEAD — surface explicitly rather than empty
        // string so callers can distinguish.
        return Err(GitError::new(
            "detached_head",
            "HEAD is detached (no current branch)",
        ));
    }
    Ok(trimmed)
}

/// Parses `git worktree list --porcelain`: blank-line-delimited records
/// of `worktree <path>` / `HEAD <sha>` / `branch refs/heads/<name>` plus
/// optional `locked [reason]` / `prunable [reason]` / bare `detached`.
pub fn list_worktrees(repo: &Path) -> Result<Vec<WorktreeInfo>, GitError> {
    let out = run_git(repo, &["worktree", "list", "--porcelain"])?;
    let s = String::from_utf8(out.stdout)
        .map_err(|e| GitError::new("io_error", format!("decode worktree list: {e}")))?;
    let mut result = Vec::new();
    let mut current: Option<WorktreeBuilder> = None;
    for line in s.lines() {
        if line.is_empty() {
            if let Some(b) = current.take()
                && let Some(w) = b.finish()
            {
                result.push(w);
            }
            continue;
        }
        let b = current.get_or_insert_with(WorktreeBuilder::default);
        if let Some(rest) = line.strip_prefix("worktree ") {
            b.path = Some(PathBuf::from(rest));
        } else if let Some(rest) = line.strip_prefix("HEAD ") {
            b.head_sha = Some(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("branch ") {
            b.branch = Some(rest.trim_start_matches("refs/heads/").to_string());
        } else if line == "detached" {
            b.detached = true;
        } else if line == "locked" || line.starts_with("locked ") {
            b.locked = true;
        } else if line == "prunable" || line.starts_with("prunable ") {
            b.prunable = true;
        }
    }
    if let Some(b) = current
        && let Some(w) = b.finish()
    {
        result.push(w);
    }
    Ok(result)
}

#[derive(Default)]
struct WorktreeBuilder {
    path: Option<PathBuf>,
    head_sha: Option<String>,
    branch: Option<String>,
    detached: bool,
    locked: bool,
    prunable: bool,
}

impl WorktreeBuilder {
    fn finish(self) -> Option<WorktreeInfo> {
        let path = self.path?;
        Some(WorktreeInfo {
            path,
            branch: if self.detached { None } else { self.branch },
            head_sha: self.head_sha.unwrap_or_default(),
            locked: self.locked,
            prunable: self.prunable,
        })
    }
}

#[derive(Debug, Clone)]
pub struct WorktreeAddResult {
    pub path: PathBuf,
    pub branch: String,
}

/// `git -C <repo> worktree add <target> -b <branch> <base>`. Pure
/// create primitive — fails as `branch_exists` on collision. Re-click
/// idempotency lives at the action layer where it shares the same
/// containment / symlink-ancestor / canonicalization gates; doing it
/// here too would give the action an unvalidated second return path.
pub fn worktree_add(
    repo: &Path,
    target: &Path,
    branch: &str,
    base: &str,
) -> Result<WorktreeAddResult, GitError> {
    validate_branch_name(branch)?;
    if base.is_empty() {
        return Err(GitError::new("invalid_params", "base cannot be empty"));
    }
    validate_base_ref(base)?;
    if let Some(parent) = target.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|e| {
            GitError::new("io_error", format!("mkdir -p {}: {e}", parent.display()))
        })?;
    }
    let target_str = target
        .to_str()
        .ok_or_else(|| GitError::new("invalid_params", "target path is not valid UTF-8"))?;
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .arg("worktree")
        .arg("add")
        .arg(target_str)
        .arg("-b")
        .arg(branch)
        .arg(base)
        .output()
        .map_err(|e| GitError::new("io_error", format!("spawn git: {e}")))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        let code = if stderr.contains("already exists") || stderr.contains("already used by") {
            "branch_exists"
        } else if stderr.contains("not a valid object name") {
            "base_not_found"
        } else {
            "git_error"
        };
        return Err(GitError::new(code, stderr.trim().to_string()));
    }
    Ok(WorktreeAddResult {
        path: target.to_path_buf(),
        branch: branch.to_string(),
    })
}

/// `git -C <repo> worktree remove [--force] <target>`.
pub fn worktree_remove(repo: &Path, target: &Path, force: bool) -> Result<(), GitError> {
    let target_str = target
        .to_str()
        .ok_or_else(|| GitError::new("invalid_params", "target path is not valid UTF-8"))?;
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(repo).arg("worktree").arg("remove");
    if force {
        cmd.arg("--force");
    }
    cmd.arg(target_str);
    let out = cmd
        .output()
        .map_err(|e| GitError::new("io_error", format!("spawn git: {e}")))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        let code = if stderr.contains("contains modified or untracked files") {
            "worktree_dirty"
        } else if stderr.contains("not a working tree") {
            "not_found"
        } else {
            "git_error"
        };
        return Err(GitError::new(code, stderr.trim().to_string()));
    }
    Ok(())
}

#[derive(Debug, Default, Clone)]
pub struct StatusReport {
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub staged: u32,
    pub unstaged: u32,
    pub untracked: u32,
    pub dirty: bool,
}

/// Parse `git status --porcelain=v2 --branch`. Cheaper than full
/// porcelain v1 parsing and gives us ahead/behind counts directly.
pub fn status(target: &Path) -> Result<StatusReport, GitError> {
    let out = run_git(target, &["status", "--porcelain=v2", "--branch"])?;
    let s = String::from_utf8(out.stdout)
        .map_err(|e| GitError::new("io_error", format!("decode status: {e}")))?;
    let mut report = StatusReport::default();
    for line in s.lines() {
        if let Some(rest) = line.strip_prefix("# branch.head ") {
            if rest != "(detached)" {
                report.branch = Some(rest.to_string());
            }
        } else if let Some(rest) = line.strip_prefix("# branch.upstream ") {
            report.upstream = Some(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("# branch.ab ") {
            // `# branch.ab +<N> -<M>`
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() == 2 {
                report.ahead = parts[0].trim_start_matches('+').parse().unwrap_or(0);
                report.behind = parts[1].trim_start_matches('-').parse().unwrap_or(0);
            }
        } else if line.starts_with("1 ") || line.starts_with("2 ") {
            // Tracked entry. Columns 2-3 are XY status flags.
            // Index status (col 2) != '.' → staged; worktree (col 3) → unstaged.
            let bytes = line.as_bytes();
            if bytes.len() >= 4 {
                let xy = &bytes[2..4];
                if xy[0] != b'.' {
                    report.staged += 1;
                }
                if xy[1] != b'.' {
                    report.unstaged += 1;
                }
            }
        } else if line.starts_with("? ") {
            report.untracked += 1;
        } else if line.starts_with("u ") {
            // Unmerged entry counts as both staged and unstaged for
            // a callers-eye view.
            report.staged += 1;
            report.unstaged += 1;
        }
    }
    report.dirty = report.staged > 0 || report.unstaged > 0 || report.untracked > 0;
    Ok(report)
}

/// Coarse subset of `git check-ref-format --branch` enforced before
/// shelling out — refuses partial-worktree state from git's own
/// validation landing mid-stream. Rules: non-empty, not just `@`, no
/// leading `-`, no leading or trailing `/`, no `..`/`@{`/`//`, no
/// control chars or `~^:?*[\\`/whitespace, no segment starts with `.`
/// or ends with `.lock` or `.`.
pub fn validate_branch_name(s: &str) -> Result<(), GitError> {
    if s.is_empty() {
        return Err(GitError::new(
            "invalid_branch",
            "branch name cannot be empty",
        ));
    }
    if s == "@" {
        return Err(GitError::new(
            "invalid_branch",
            "branch name cannot be just \"@\"",
        ));
    }
    if s.starts_with('-') {
        return Err(GitError::new(
            "invalid_branch",
            "branch name cannot start with '-' (would look like a flag)",
        ));
    }
    if s.starts_with('/') || s.ends_with('/') {
        return Err(GitError::new(
            "invalid_branch",
            "branch name cannot start or end with '/'",
        ));
    }
    if s.contains("..") || s.contains("@{") || s.contains("//") {
        return Err(GitError::new(
            "invalid_branch",
            "branch name contains forbidden sequence (.., @{, //)",
        ));
    }
    for c in s.chars() {
        if c.is_control() {
            return Err(GitError::new(
                "invalid_branch",
                "branch name contains control character",
            ));
        }
        if matches!(c, ' ' | '~' | '^' | ':' | '?' | '*' | '[' | '\\') {
            return Err(GitError::new(
                "invalid_branch",
                format!("branch name contains forbidden char {c:?}"),
            ));
        }
    }
    for segment in s.split('/') {
        if segment.is_empty() {
            return Err(GitError::new(
                "invalid_branch",
                "branch name has empty path segment",
            ));
        }
        if segment.starts_with('.') {
            return Err(GitError::new(
                "invalid_branch",
                format!("branch segment {segment:?} cannot start with '.'"),
            ));
        }
        if segment.ends_with(".lock") {
            return Err(GitError::new(
                "invalid_branch",
                format!("branch segment {segment:?} cannot end with .lock"),
            ));
        }
        if segment.ends_with('.') {
            // git check-ref-format rejects this: `foo.` and
            // `feature/foo.` both trip "fatal: ... is not a valid
            // branch name". Without this rule, the bad input
            // would land late inside `git worktree add` after
            // we've already created the parent directory.
            return Err(GitError::new(
                "invalid_branch",
                format!("branch segment {segment:?} cannot end with '.'"),
            ));
        }
    }
    Ok(())
}

/// Looser than `validate_branch_name` so legitimate commit-ish forms
/// (`HEAD~1`, `main^`, `tag^{commit}`) pass; the trust boundary still
/// rejects: empty, leading `-` (would be parsed as a flag — git worktree
/// add doesn't accept `--` to terminate options), control / NUL chars,
/// and any whitespace (commit-ish refs never contain spaces).
pub fn validate_base_ref(s: &str) -> Result<(), GitError> {
    if s.is_empty() {
        return Err(GitError::new("invalid_params", "base cannot be empty"));
    }
    if s.starts_with('-') {
        return Err(GitError::new(
            "invalid_params",
            format!("base {s:?} cannot start with '-' (git would parse it as a flag)"),
        ));
    }
    for c in s.chars() {
        if c == '\0' || c.is_control() {
            return Err(GitError::new(
                "invalid_params",
                "base ref contains nul or control character",
            ));
        }
        if c.is_whitespace() {
            return Err(GitError::new(
                "invalid_params",
                "base ref cannot contain whitespace",
            ));
        }
    }
    Ok(())
}

/// Lowercase + run through `validate_branch_name`. Slashes are
/// preserved (`epic/PROJ-456` → `epic/proj-456`). Used by
/// `worktree_add` when `sanitize_jira = true`, so a trigger
/// interpolating `linked_jira` from a Todo payload gets a valid
/// kebab-style branch name without per-trigger massaging.
pub fn sanitize_jira_branch(s: &str) -> Result<String, GitError> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Err(GitError::new(
            "invalid_branch",
            "cannot sanitize an empty branch name",
        ));
    }
    let lower = trimmed.to_ascii_lowercase();
    validate_branch_name(&lower)?;
    Ok(lower)
}

/// Run `git -C <repo> <args...>` and return the captured Output on
/// success, or a GitError on non-zero exit / spawn failure.
fn run_git(repo: &Path, args: &[&str]) -> Result<std::process::Output, GitError> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map_err(|e| GitError::new("io_error", format!("spawn git: {e}")))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        return Err(GitError::new("git_error", stderr.trim().to_string()));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::tempdir;

    /// Real `git` (not a fixture) because porcelain parsers depend on
    /// git's exact output format.
    fn init_repo(dir: &Path) {
        for cmd in [
            vec!["init", "--initial-branch=main", "."],
            vec!["config", "user.email", "t@example.com"],
            vec!["config", "user.name", "Test"],
            vec!["commit", "--allow-empty", "-m", "initial"],
        ] {
            let status = Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(&cmd)
                .status()
                .unwrap();
            assert!(status.success(), "git {cmd:?} failed");
        }
    }

    #[test]
    fn current_branch_returns_main() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        assert_eq!(current_branch(dir.path()).unwrap(), "main");
    }

    #[test]
    fn list_worktrees_shows_main_only() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        let wts = list_worktrees(dir.path()).unwrap();
        assert_eq!(wts.len(), 1);
        assert_eq!(wts[0].branch.as_deref(), Some("main"));
    }

    #[test]
    fn worktree_add_then_remove_round_trips() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        let target = dir.path().join("..").join(format!(
            "wt-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let r = worktree_add(dir.path(), &target, "feature/test", "main").unwrap();
        assert_eq!(r.branch, "feature/test");
        let wts = list_worktrees(dir.path()).unwrap();
        assert!(
            wts.iter()
                .any(|w| w.branch.as_deref() == Some("feature/test"))
        );
        worktree_remove(dir.path(), &target, false).unwrap();
        let wts2 = list_worktrees(dir.path()).unwrap();
        assert!(
            !wts2
                .iter()
                .any(|w| w.branch.as_deref() == Some("feature/test"))
        );
    }

    #[test]
    fn worktree_add_rejects_existing_branch_at_module_layer() {
        // git::worktree_add is the pure create primitive — idempotency
        // for re-click flows lives in action_worktree_add (see its
        // own test in main.rs). Direct callers that hit a branch
        // collision here get branch_exists / git_error, same as
        // before.
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        let target = dir.path().join("..").join(format!(
            "wt-dup-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let first = worktree_add(dir.path(), &target, "feature/dup", "main").unwrap();
        let target2 = target.with_extension("again");
        let err = worktree_add(dir.path(), &target2, "feature/dup", "main").unwrap_err();
        assert!(
            matches!(err.code, "branch_exists" | "git_error"),
            "got {err:?}"
        );
        let _ = worktree_remove(dir.path(), &first.path, false);
    }

    #[test]
    fn status_reports_dirty_after_edit() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        // Clean state.
        let r = status(dir.path()).unwrap();
        assert!(!r.dirty);
        assert_eq!(r.branch.as_deref(), Some("main"));
        // Add an untracked file.
        std::fs::write(dir.path().join("new.txt"), "hello").unwrap();
        let r2 = status(dir.path()).unwrap();
        assert!(r2.dirty);
        assert_eq!(r2.untracked, 1);
    }

    #[test]
    fn validate_branch_name_rejects_bad() {
        for bad in [
            "",
            "-flag",
            "/abs",
            "trailing/",
            "..",
            "x..y",
            "a@{b",
            "x  y",
            "has~tilde",
            "has\0nul",
            "/slash-start",
            ".dotstart",
            "branch.lock",
            "feat//double",
            "@",
            "trailing.",
            "feature/foo.",
        ] {
            assert!(validate_branch_name(bad).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn sanitize_jira_lowercases_and_preserves_slashes() {
        assert_eq!(sanitize_jira_branch("PROJ-456").unwrap(), "proj-456");
        assert_eq!(
            sanitize_jira_branch("Epic/PROJ-456").unwrap(),
            "epic/proj-456"
        );
        // Pre-lowercased input is a no-op.
        assert_eq!(sanitize_jira_branch("feat/foo").unwrap(), "feat/foo");
        // Whitespace trimmed.
        assert_eq!(sanitize_jira_branch("  PROJ-1 ").unwrap(), "proj-1");
    }

    #[test]
    fn sanitize_jira_rejects_validate_failures_after_lowercase() {
        // `..` survives lowercasing and validate_branch_name
        // still catches it.
        assert!(sanitize_jira_branch("a..b").is_err());
        // Empty trim.
        assert!(sanitize_jira_branch("   ").is_err());
        // Leading `.` after lowercasing.
        assert!(sanitize_jira_branch(".HIDDEN").is_err());
        // Leading `-` would be parsed as a flag.
        assert!(sanitize_jira_branch("-flag").is_err());
    }

    #[test]
    fn base_ref_accepts_commit_ish_syntax() {
        // Round-8 fix: HEAD~1, main^, tag^{commit} must pass —
        // they're the documented-allowed commit-ish forms for
        // `git worktree add ... <base>`. The narrower check
        // from round 7 wrongly rejected them.
        for good in [
            "HEAD",
            "HEAD~1",
            "main",
            "main^",
            "main^2",
            "v1.0.0",
            "abc1234",
            "tag^{commit}",
            "origin/main",
        ] {
            validate_base_ref(good).unwrap_or_else(|e| panic!("rejected {good:?}: {e:?}"));
        }
    }

    #[test]
    fn base_ref_rejects_injection_vectors() {
        for bad in ["", "-d", "--detach", " HEAD", "HEAD --force", "HEAD\0"] {
            assert!(validate_base_ref(bad).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn validate_branch_name_accepts_good() {
        for good in [
            "main",
            "feature/foo",
            "feat/PROJ-456",
            "release-1.2",
            "user@team/branch",
        ] {
            validate_branch_name(good).unwrap_or_else(|e| panic!("rejected {good:?}: {e:?}"));
        }
    }
}
