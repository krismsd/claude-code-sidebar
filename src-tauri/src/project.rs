//! Project identity (SPEC §3.4): git root, worktree, branch and a stable colour for a session's
//! cwd. Files are read from Windows through `\\wsl.localhost\<distro>\…` (or the drive letter
//! for `/mnt/<x>/…`), so no `wsl.exe` call is needed.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 12 colours that stay distinct on both light and dark backgrounds.
pub const PALETTE: [&str; 12] = [
    "#e5484d", "#f76b15", "#ffc53d", "#46a758", "#12a594", "#00a2c7", "#0090ff", "#3e63dd",
    "#8e4ec6", "#d6409f", "#a18072", "#6e56cf",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    /// Linux path of the main repo root (or the cwd if not in a repo).
    pub root: String,
    pub name: String,
    pub color: String,
    pub emoji: Option<String>,
    /// Short label shown next to the name (e.g. a team or org).
    #[serde(alias = "client")]
    pub label: Option<String>,
    /// Worktree name when the cwd is inside a linked worktree.
    pub worktree: Option<String>,
    pub branch: Option<String>,
}

/// One `[[project]]` entry in `%APPDATA%\cc-sidebar\projects.toml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Override {
    /// Linux path of the repo root this applies to.
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
    /// Short label shown next to the name; `client` is accepted too (older files).
    #[serde(alias = "client", skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Where this project's tasks come from (SPEC §3.12).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tasks: Option<TasksConfig>,
}

/// `[project.tasks]`: list tasks through Claude and a claude.ai connector.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TasksConfig {
    /// claude.ai connector name, e.g. "Atlassian"; empty for CLI trackers (see `tools`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub connector: String,
    /// Plain English, e.g. "Jira issues in project ACME assigned to me that aren't done".
    pub query: String,
    /// Allowed tools: connector tool names (defaults per connector in cc-tasks) or permission
    /// rules such as `Bash(az boards query:*)` for CLI trackers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
    /// First prompt for a session started from a task; {id} {title} {url} {summary} {status}.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
}

pub const PROJECTS_TEMPLATE: &str = r##"# cc-sidebar per-project settings. Saved changes apply within a couple of seconds.
# One [[project]] per repo root (the Linux path). Everything except `path` is optional.
#
# [[project]]
# path = "/home/you/dev/acme"
# name = "Acme portal"        # display name (default: folder name)
# color = "#3e63dd"           # default: picked from the path
# emoji = "🚀"
# label = "Platform team"  # short label shown next to the name
#
# [project.tasks]             # "Tasks…" in the project menu
# connector = "Atlassian"     # a claude.ai connector
# query = "Jira issues in project ACME assigned to me that aren't done, most recently updated first"
# # tools = ["searchJiraIssuesUsingJql"]   # read-only tools to allow (Atlassian has defaults)
#
# A CLI tracker instead of a connector, e.g. Azure DevOps through the az CLI:
# [project.tasks]
# query = "Azure DevOps work items assigned to me that aren't done, via az boards query with --org https://dev.azure.com/ORG --project \"PROJECT\". Use NOT IN (...) in the WIQL, never < or > (shell redirection gets the command refused)."
# tools = ["Bash(az boards query:*)"]
# # prompt = "Work on {id}: {title}\n\n{summary}\n\n{url}"
"##;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Overrides {
    #[serde(default)]
    pub project: Vec<Override>,
}

impl Overrides {
    pub fn parse(toml_text: &str) -> Result<Overrides, toml::de::Error> {
        toml::from_str(toml_text)
    }

    pub fn path() -> Option<PathBuf> {
        crate::config::dir().map(|d| d.join("projects.toml"))
    }

    pub fn modified() -> Option<std::time::SystemTime> {
        std::fs::metadata(Self::path()?)
            .and_then(|m| m.modified())
            .ok()
    }

    pub fn load() -> Overrides {
        let Some(path) = Self::path() else {
            return Overrides::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => Overrides::parse(&text).unwrap_or_else(|e| {
                log::warn!("{}: {e}", path.display());
                Overrides::default()
            }),
            Err(_) => Overrides::default(),
        }
    }

    pub fn tasks_for(&self, root: &str) -> Option<&TasksConfig> {
        self.find(root)?.tasks.as_ref()
    }

    fn find(&self, root: &str) -> Option<&Override> {
        self.project
            .iter()
            .find(|o| o.path.trim_end_matches('/') == root)
    }
}

/// Map a WSL path to a path Windows can read.
pub fn windows_path(distro: &str, linux: &str) -> PathBuf {
    let parts: Vec<&str> = linux.split('/').filter(|p| !p.is_empty()).collect();
    if parts.len() >= 2 && parts[0] == "mnt" && parts[1].len() == 1 {
        let mut p = PathBuf::from(format!("{}:\\", parts[1].to_ascii_uppercase()));
        p.extend(&parts[2..]);
        p
    } else {
        let mut p = PathBuf::from(format!(r"\\wsl.localhost\{distro}\"));
        p.extend(&parts);
        p
    }
}

/// FNV-1a: stable across runs and platforms (unlike std's hasher).
fn stable_hash(s: &str) -> u64 {
    s.bytes().fold(0xcbf29ce484222325, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100000001b3)
    })
}

pub fn color_for(root: &str) -> String {
    PALETTE[(stable_hash(root) % PALETTE.len() as u64) as usize].to_string()
}

/// Read a branch name from a git dir's HEAD ("ref: refs/heads/x" or a detached sha).
fn branch_from_head(head: &str) -> Option<String> {
    let head = head.trim();
    if let Some(r) = head.strip_prefix("ref:") {
        let r = r.trim();
        Some(r.strip_prefix("refs/heads/").unwrap_or(r).to_string())
    } else if head.len() >= 7 {
        Some(format!("detached {}", &head[..7]))
    } else {
        None
    }
}

/// Resolve the project for a Linux `cwd`. `to_fs` maps a Linux path to a readable path
/// (`windows_path` in the app; a temp dir in tests).
pub fn resolve(cwd: &str, to_fs: impl Fn(&str) -> PathBuf, overrides: &Overrides) -> Project {
    let cwd = cwd.trim_end_matches('/');
    let mut root = None;
    let mut worktree = None;
    let mut branch = None;

    let mut dir = cwd.to_string();
    loop {
        let dotgit = to_fs(&format!("{dir}/.git"));
        if dotgit.is_dir() {
            root = Some(dir.clone());
            branch = std::fs::read_to_string(dotgit.join("HEAD"))
                .ok()
                .as_deref()
                .and_then(branch_from_head);
            break;
        }
        if dotgit.is_file() {
            // Linked worktree: ".git" file says "gitdir: <main>/.git/worktrees/<name>".
            let text = std::fs::read_to_string(&dotgit).unwrap_or_default();
            if let Some(gitdir) = text.trim().strip_prefix("gitdir:").map(str::trim) {
                branch = std::fs::read_to_string(to_fs(&format!("{gitdir}/HEAD")))
                    .ok()
                    .as_deref()
                    .and_then(branch_from_head);
                if let Some((main_git, name)) = gitdir.rsplit_once("/worktrees/") {
                    root = main_git.strip_suffix("/.git").map(String::from);
                    worktree = Some(name.to_string());
                }
            }
            root = root.or(Some(dir.clone()));
            break;
        }
        match dir.rsplit_once('/') {
            Some((parent, _)) if !parent.is_empty() => dir = parent.to_string(),
            _ => break,
        }
    }

    let root = root.unwrap_or_else(|| cwd.to_string());
    let o = overrides.find(&root);
    let base = root.rsplit('/').next().unwrap_or(&root).to_string();
    Project {
        name: o.and_then(|o| o.name.clone()).unwrap_or(base),
        color: o
            .and_then(|o| o.color.clone())
            .unwrap_or_else(|| color_for(&root)),
        emoji: o.and_then(|o| o.emoji.clone()),
        label: o.and_then(|o| o.label.clone()),
        root,
        worktree,
        branch,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// A fake Linux tree under a temp dir: Linux "/x/y" lives at <tmp>/x/y.
    struct Tree(PathBuf);
    impl Tree {
        fn new(name: &str) -> Tree {
            let p =
                std::env::temp_dir().join(format!("cc-sidebar-test-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&p);
            fs::create_dir_all(&p).unwrap();
            Tree(p)
        }
        fn fs(&self, linux: &str) -> PathBuf {
            self.0.join(linux.trim_start_matches('/'))
        }
        fn write(&self, linux: &str, text: &str) {
            let p = self.fs(linux);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, text).unwrap();
        }
        fn mkdir(&self, linux: &str) {
            fs::create_dir_all(self.fs(linux)).unwrap();
        }
    }
    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn windows_paths() {
        assert_eq!(
            windows_path("Ubuntu", "/mnt/c/dev/cc-sidebar"),
            PathBuf::from(r"C:\dev\cc-sidebar")
        );
        assert_eq!(
            windows_path("Ubuntu", "/home/user/dev/app"),
            PathBuf::from(r"\\wsl.localhost\Ubuntu\home\user\dev\app")
        );
    }

    #[test]
    fn repo_root_and_branch_from_subdir() {
        let t = Tree::new("repo");
        t.write(
            "/home/u/dev/acme/.git/HEAD",
            "ref: refs/heads/feature/login\n",
        );
        t.mkdir("/home/u/dev/acme/src/auth");
        let p = resolve(
            "/home/u/dev/acme/src/auth",
            |l| t.fs(l),
            &Overrides::default(),
        );
        assert_eq!(p.root, "/home/u/dev/acme");
        assert_eq!(p.name, "acme");
        assert_eq!(p.branch.as_deref(), Some("feature/login"));
        assert_eq!(p.worktree, None);
        assert_eq!(p.color, color_for("/home/u/dev/acme"));
    }

    #[test]
    fn linked_worktree_resolves_to_main_repo() {
        let t = Tree::new("worktree");
        t.write("/home/u/dev/acme/.git/HEAD", "ref: refs/heads/main\n");
        t.write(
            "/home/u/dev/acme/.git/worktrees/hotfix/HEAD",
            "ref: refs/heads/hotfix-123\n",
        );
        t.write(
            "/home/u/dev/acme-hotfix/.git",
            "gitdir: /home/u/dev/acme/.git/worktrees/hotfix\n",
        );
        let p = resolve(
            "/home/u/dev/acme-hotfix",
            |l| t.fs(l),
            &Overrides::default(),
        );
        assert_eq!(p.root, "/home/u/dev/acme");
        assert_eq!(p.name, "acme");
        assert_eq!(p.worktree.as_deref(), Some("hotfix"));
        assert_eq!(p.branch.as_deref(), Some("hotfix-123"));
    }

    #[test]
    fn detached_head_and_no_repo() {
        let t = Tree::new("detached");
        t.write("/r/.git/HEAD", "0123456789abcdef0123456789abcdef01234567\n");
        let p = resolve("/r", |l| t.fs(l), &Overrides::default());
        assert_eq!(p.branch.as_deref(), Some("detached 0123456"));

        t.mkdir("/scratch/notes");
        let p = resolve("/scratch/notes", |l| t.fs(l), &Overrides::default());
        assert_eq!(p.root, "/scratch/notes");
        assert_eq!(p.name, "notes");
        assert_eq!(p.branch, None);
    }

    #[test]
    fn tasks_config_parses() {
        assert!(
            Overrides::parse(PROJECTS_TEMPLATE)
                .unwrap()
                .project
                .is_empty()
        );
        let o = Overrides::parse(
            r#"
            [[project]]
            path = "/home/u/dev/acme"
            [project.tasks]
            connector = "Atlassian"
            query = "my open ACME issues"
            "#,
        )
        .unwrap();
        let t = o.tasks_for("/home/u/dev/acme").unwrap();
        assert_eq!(
            (t.connector.as_str(), t.query.as_str()),
            ("Atlassian", "my open ACME issues")
        );
        assert!(t.tools.is_empty() && t.prompt.is_none());
        assert!(o.tasks_for("/elsewhere").is_none());
    }

    #[test]
    fn overrides_apply_by_root() {
        let t = Tree::new("overrides");
        t.write("/home/u/dev/acme/.git/HEAD", "ref: refs/heads/main\n");
        let o = Overrides::parse(
            r##"
            [[project]]
            path = "/home/u/dev/acme/"
            name = "Acme portal"
            color = "#123456"
            emoji = "🚀"
            client = "Acme Co"  # old name for `label`, still accepted
            "##,
        )
        .unwrap();
        let p = resolve("/home/u/dev/acme", |l| t.fs(l), &o);
        assert_eq!(p.name, "Acme portal");
        assert_eq!(p.color, "#123456");
        assert_eq!(p.emoji.as_deref(), Some("🚀"));
        assert_eq!(p.label.as_deref(), Some("Acme Co"));
    }

    #[test]
    fn colours_are_stable_and_spread() {
        assert_eq!(color_for("/home/u/dev/acme"), color_for("/home/u/dev/acme"));
        let distinct: std::collections::HashSet<_> = (0..40)
            .map(|i| color_for(&format!("/home/u/dev/project-{i}")))
            .collect();
        assert!(distinct.len() >= 8, "only {} colours used", distinct.len());
    }
}
