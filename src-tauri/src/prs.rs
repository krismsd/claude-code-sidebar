//! Pull requests a session opened (SPEC §3.16). Found in its transcript: Claude Code's own
//! `pr-link` records (GitHub), and the results of `gh pr create` / `az repos pr create`
//! commands (Azure DevOps has no record; its output is often trimmed with `--query`, so the
//! organisation comes from the command and the id from whatever it printed). Their state,
//! checks and reviews are read with the same CLIs in WSL, read-only, with the user's logins:
//! every 5 minutes while open, and after each turn.
use crate::server::{Ctx, now_ms, publish};
use crate::session::Ms;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;

/// How often an open PR is looked at (and after each turn of its session).
pub const EVERY: Ms = 5 * 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Host {
    Github,
    Azure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Open,
    Draft,
    Merged,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Checks {
    Passing,
    Failing,
    Running,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Review {
    Approved,
    ChangesRequested,
    Required,
}

/// A PR as found in a transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub host: Host,
    pub number: u64,
    /// GitHub: the PR's URL. Azure DevOps: unknown until looked up.
    pub url: Option<String>,
    /// Azure DevOps organisation URL (None: the az CLI's default).
    pub org: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Status {
    pub state: Option<State>,
    pub checks: Option<Checks>,
    pub review: Option<Review>,
    pub title: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pr {
    pub host: Host,
    pub number: u64,
    pub url: Option<String>,
    pub org: Option<String>,
    pub title: Option<String>,
    /// None until first looked up.
    pub state: Option<State>,
    pub checks: Option<Checks>,
    pub review: Option<Review>,
    /// When it was last looked up (0: never).
    pub checked_at: Ms,
}

impl Pr {
    pub fn new(f: Found) -> Pr {
        Pr {
            host: f.host,
            number: f.number,
            url: f.url,
            org: f.org,
            title: None,
            state: None,
            checks: None,
            review: None,
            checked_at: 0,
        }
    }

    pub fn same(&self, f: &Found) -> bool {
        self.host == f.host
            && self.number == f.number
            && match self.host {
                Host::Github => self.url == f.url,
                Host::Azure => self.org == f.org,
            }
    }

    /// The same PR (an Azure DevOps PR's URL is only known once looked up).
    pub fn is(&self, other: &Pr) -> bool {
        self.host == other.host
            && self.number == other.number
            && match self.host {
                Host::Github => self.url == other.url,
                Host::Azure => self.org == other.org,
            }
    }

    pub fn open(&self) -> bool {
        !matches!(self.state, Some(State::Merged | State::Closed))
    }
}

/// PRs opened in a transcript (all of it or a tail), in order, without repeats.
pub fn find(text: &str) -> Vec<Found> {
    let mut out: Vec<Found> = Vec::new();
    let mut add = |f: Found| {
        if !out.contains(&f) {
            out.push(f);
        }
    };
    // Tool calls that create PRs, by id: their command.
    let mut pending: HashMap<String, String> = HashMap::new();
    for line in text.lines() {
        let quick = line.contains("pr-link")
            || line.contains("pr create")
            || (!pending.is_empty() && line.contains("tool_result"));
        if !quick {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match v["type"].as_str() {
            Some("pr-link") => {
                if let (Some(url), Some(n)) = (v["prUrl"].as_str(), v["prNumber"].as_u64()) {
                    add(Found {
                        host: Host::Github,
                        number: n,
                        url: Some(url.to_string()),
                        org: None,
                    });
                }
            }
            Some("assistant") => {
                for c in v["message"]["content"].as_array().into_iter().flatten() {
                    let cmd = c["input"]["command"].as_str().unwrap_or("");
                    if c["type"] == "tool_use"
                        && (cmd.contains("gh pr create") || cmd.contains("az repos pr create"))
                        && let Some(id) = c["id"].as_str()
                    {
                        pending.insert(id.to_string(), cmd.to_string());
                    }
                }
            }
            Some("user") => {
                for c in v["message"]["content"].as_array().into_iter().flatten() {
                    let Some(cmd) = c["tool_use_id"].as_str().and_then(|id| pending.remove(id))
                    else {
                        continue;
                    };
                    let stdout = v["toolUseResult"]["stdout"]
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| result_text(&c["content"]));
                    if cmd.contains("gh pr create") {
                        for url in github_urls(&stdout) {
                            let number = url.rsplit('/').next().and_then(|n| n.parse().ok());
                            if let Some(number) = number {
                                add(Found {
                                    host: Host::Github,
                                    number,
                                    url: Some(url),
                                    org: None,
                                });
                            }
                        }
                    } else if let Some(number) = azure_id(&stdout) {
                        add(Found {
                            host: Host::Azure,
                            number,
                            url: None,
                            // Only this command's options (a heredoc before it may hold
                            // anything, apostrophes included).
                            org: flag(
                                &cmd[cmd.find("az repos pr create").unwrap_or(0)..],
                                &["--org", "--organization"],
                            ),
                        });
                    }
                }
            }
            _ => {}
        }
    }
    out
}

fn result_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| p["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// `https://github.com/<owner>/<repo>/pull/<n>` links in some text.
fn github_urls(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (i, _) in text.match_indices("https://github.com/") {
        let url: String = text[i..]
            .chars()
            .take_while(|c| !c.is_whitespace() && !"\"'<>)]".contains(*c))
            .collect();
        let parts: Vec<&str> = url.trim_end_matches('/').split('/').collect();
        // https: "" github.com owner repo pull n
        if parts.len() == 7 && parts[5] == "pull" && parts[6].parse::<u64>().is_ok() {
            out.push(parts.join("/"));
        }
    }
    out
}

/// The id `az repos pr create` printed: its JSON (`pullRequestId`, or `id` when trimmed with
/// `--query`), a `/pullrequest/<n>` link, or just the number.
fn azure_id(stdout: &str) -> Option<u64> {
    let t = stdout.trim();
    if let Ok(v) = serde_json::from_str::<Value>(t) {
        if let Some(n) = v["pullRequestId"].as_u64().or_else(|| v["id"].as_u64()) {
            return Some(n);
        }
        if let Some(n) = v.as_u64() {
            return Some(n);
        }
    }
    if let Some(i) = t.find("/pullrequest/") {
        let n: String = t[i + 13..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        return n.parse().ok();
    }
    t.parse().ok()
}

/// The value of a command-line option (`--org X`, `--org=X`, quoted or not).
fn flag(cmd: &str, names: &[&str]) -> Option<String> {
    let words = shell_words(cmd);
    for (i, w) in words.iter().enumerate() {
        for n in names {
            if w == n {
                return words.get(i + 1).cloned();
            }
            if let Some(v) = w.strip_prefix(&format!("{n}=")) {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// Split a command line into words, honouring simple quotes (enough for option values).
fn shell_words(cmd: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    for c in cmd.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => cur.push(c),
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                started = true;
            }
            None if c.is_whitespace() => {
                if started || !cur.is_empty() {
                    words.push(std::mem::take(&mut cur));
                }
                started = false;
            }
            None => cur.push(c),
        }
    }
    if started || !cur.is_empty() {
        words.push(cur);
    }
    words
}

fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// The shell script that looks a PR up (prints JSON; Azure: two documents split by "@@").
/// Run in the session's directory, so an Azure DevOps PR without a known organisation is
/// found from the repository's git remote, as `az` does when run there.
pub fn script(pr: &Pr, cwd: Option<&str>) -> Option<String> {
    let cd = cwd
        .map(|d| format!("cd {} 2>/dev/null; ", quote(d)))
        .unwrap_or_default();
    Some(format!("{cd}{}", lookup(pr)?))
}

fn lookup(pr: &Pr) -> Option<String> {
    match pr.host {
        Host::Github => Some(format!(
            "gh pr view {} --json state,isDraft,title,reviewDecision,statusCheckRollup",
            quote(pr.url.as_deref()?)
        )),
        Host::Azure => {
            let org = pr
                .org
                .as_deref()
                .map(|o| format!(" --org {}", quote(o)))
                .unwrap_or_default();
            let id = pr.number;
            Some(format!(
                "az repos pr show --id {id}{org} --query \
                 '{{status:status,isDraft:isDraft,title:title,votes:reviewers[].vote,repo:repository.webUrl}}' -o json \
                 && echo @@ && az repos pr policy list --id {id}{org} --query \
                 '[].{{status:status,type:configuration.type.displayName}}' -o json"
            ))
        }
    }
}

/// `gh pr view --json …` output.
pub fn github_status(json: &str) -> Option<Status> {
    let v: Value = serde_json::from_str(json).ok()?;
    let state = match v["state"].as_str()? {
        "MERGED" => State::Merged,
        "CLOSED" => State::Closed,
        _ if v["isDraft"].as_bool() == Some(true) => State::Draft,
        _ => State::Open,
    };
    let review = match v["reviewDecision"].as_str().unwrap_or("") {
        "APPROVED" => Some(Review::Approved),
        "CHANGES_REQUESTED" => Some(Review::ChangesRequested),
        "REVIEW_REQUIRED" => Some(Review::Required),
        _ => None,
    };
    let mut fail = false;
    let mut running = false;
    let mut any = false;
    for c in v["statusCheckRollup"].as_array().into_iter().flatten() {
        any = true;
        // CheckRun: status + conclusion. StatusContext: state.
        let s = c["conclusion"]
            .as_str()
            .filter(|s| !s.is_empty())
            .or_else(|| c["state"].as_str())
            .unwrap_or("");
        let done = c["status"].as_str().is_none_or(|s| s == "COMPLETED");
        match s {
            "FAILURE" | "ERROR" | "TIMED_OUT" | "CANCELLED" | "ACTION_REQUIRED"
            | "STARTUP_FAILURE" => fail = true,
            "PENDING" | "EXPECTED" => running = true,
            _ if !done => running = true,
            _ => {}
        }
    }
    Some(Status {
        state: Some(state),
        checks: checks(any, fail, running),
        review,
        title: v["title"].as_str().map(str::to_string),
        url: None,
    })
}

fn checks(any: bool, fail: bool, running: bool) -> Option<Checks> {
    match (any, fail, running) {
        (false, ..) => None,
        (_, true, _) => Some(Checks::Failing),
        (_, _, true) => Some(Checks::Running),
        _ => Some(Checks::Passing),
    }
}

/// `az repos pr show` and `az repos pr policy list` output (see `script`).
pub fn azure_status(out: &str, number: u64) -> Option<Status> {
    let (show, policies) = out.split_once("@@").unwrap_or((out, "[]"));
    let v: Value = serde_json::from_str(show.trim()).ok()?;
    let state = match v["status"].as_str()? {
        "completed" => State::Merged,
        "abandoned" => State::Closed,
        _ if v["isDraft"].as_bool() == Some(true) => State::Draft,
        _ => State::Open,
    };
    // Votes: 10 approved, 5 approved with suggestions, -5 waiting for author, -10 rejected.
    let votes: Vec<i64> = v["votes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_i64)
        .collect();
    let review = if votes.iter().any(|&n| n < 0) {
        Some(Review::ChangesRequested)
    } else if votes.iter().any(|&n| n >= 5) {
        Some(Review::Approved)
    } else {
        Some(Review::Required)
    };
    // Checks are the build policies; the others are about reviewers and comments.
    let p: Value = serde_json::from_str(policies.trim()).unwrap_or(Value::Null);
    let (mut any, mut fail, mut running) = (false, false, false);
    for x in p.as_array().into_iter().flatten() {
        if x["type"].as_str() != Some("Build") {
            continue;
        }
        any = true;
        match x["status"].as_str().unwrap_or("") {
            "rejected" | "broken" => fail = true,
            "running" | "queued" => running = true,
            _ => {}
        }
    }
    Some(Status {
        state: Some(state),
        checks: checks(any, fail, running),
        review,
        title: v["title"].as_str().map(str::to_string),
        url: v["repo"]
            .as_str()
            .map(|r| format!("{r}/pullrequest/{number}")),
    })
}

/// Look up one PR. None if the CLI failed (not logged in, no network, gone).
async fn look_up(distro: &str, cwd: Option<&str>, pr: &Pr) -> Option<Status> {
    let out = crate::liveness::wsl_for(distro, &script(pr, cwd)?, 60).await?;
    match pr.host {
        Host::Github => github_status(&out),
        Host::Azure => azure_status(&out, pr.number),
    }
}

/// Every minute: find PRs in transcripts not read in full yet (once per session), then look
/// up the PRs that are due.
pub async fn watch(ctx: Ctx) {
    loop {
        tokio::time::sleep(Duration::from_secs(60)).await;
        let unscanned = ctx.store.lock().unwrap().prs_unscanned();
        for (id, distro, transcript) in unscanned {
            let path = crate::project::windows_path(&distro, &transcript);
            let found = tauri::async_runtime::spawn_blocking(move || {
                std::fs::read(&path)
                    .ok()
                    .map(|b| find(&String::from_utf8_lossy(&b)))
            })
            .await
            .ok()
            .flatten();
            if let Some(found) = found
                && ctx.store.lock().unwrap().add_prs(&id, found, true)
            {
                publish(&ctx);
            }
        }
        let due = ctx.store.lock().unwrap().prs_due(now_ms());
        for (session_id, distro, cwd, pr) in due {
            let status = look_up(&distro, cwd.as_deref(), &pr).await;
            let mut store = ctx.store.lock().unwrap();
            store.set_pr_status(&session_id, &pr, status, now_ms());
            drop(store);
            publish(&ctx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_use(id: &str, cmd: &str) -> String {
        serde_json::json!({"type": "assistant", "message": {"content": [
            {"type": "tool_use", "id": id, "name": "Bash", "input": {"command": cmd}}]}})
        .to_string()
    }

    fn tool_result(id: &str, stdout: &str) -> String {
        serde_json::json!({"type": "user", "message": {"content": [
            {"type": "tool_result", "tool_use_id": id, "content": stdout}]},
            "toolUseResult": {"stdout": stdout}})
        .to_string()
    }

    #[test]
    fn finds_prs_the_session_opened() {
        let t = [
            r#"{"type":"pr-link","prNumber":12,"prUrl":"https://github.com/o/app/pull/12","prRepository":"o/app"}"#.to_string(),
            r#"{"type":"pr-link","prNumber":12,"prUrl":"https://github.com/o/app/pull/12","prRepository":"o/app"}"#.to_string(),
            tool_use("a", "cd /p && gh pr create --base main --title x --body y"),
            tool_result("a", "https://github.com/o/app/pull/13\n"),
            tool_use("b", r#"az repos pr create --org https://dev.azure.com/acme --project "Big Project" --repository r --query id -o tsv"#),
            tool_result("b", "4209"),
            tool_use("c", "az repos pr create --organization=https://dev.azure.com/acme --repository r"),
            tool_result("c", "{\n  \"id\": 4207,\n  \"status\": \"active\"\n}"),
            // A heredoc with an apostrophe before the command.
            tool_use("e", "cat > /tmp/d.md <<'EOF'\nthe team's notes\nEOF\naz repos pr create --org https://dev.azure.com/acme --repository r"),
            tool_result("e", "4210"),
            // Mentioned, not created: not ours.
            tool_use("d", "gh pr view https://github.com/o/app/pull/99"),
            tool_result("d", "https://github.com/o/app/pull/99"),
        ]
        .join("\n");
        let got = find(&t);
        let gh = |n: u64| Found {
            host: Host::Github,
            number: n,
            url: Some(format!("https://github.com/o/app/pull/{n}")),
            org: None,
        };
        let az = |n: u64| Found {
            host: Host::Azure,
            number: n,
            url: None,
            org: Some("https://dev.azure.com/acme".into()),
        };
        assert_eq!(got, vec![gh(12), gh(13), az(4209), az(4207), az(4210)]);
    }

    #[test]
    fn github_status_from_gh() {
        let j = r#"{"state":"OPEN","isDraft":false,"title":"Fix login","reviewDecision":"APPROVED",
            "statusCheckRollup":[{"__typename":"CheckRun","status":"COMPLETED","conclusion":"SUCCESS"},
            {"__typename":"CheckRun","status":"IN_PROGRESS","conclusion":""}]}"#;
        let s = github_status(j).unwrap();
        assert_eq!(s.state, Some(State::Open));
        assert_eq!(s.checks, Some(Checks::Running));
        assert_eq!(s.review, Some(Review::Approved));
        let failed = j
            .replace("IN_PROGRESS", "COMPLETED")
            .replace(r#""conclusion":"""#, r#""conclusion":"FAILURE""#);
        assert_eq!(
            github_status(&failed).unwrap().checks,
            Some(Checks::Failing)
        );
        let merged = r#"{"state":"MERGED","isDraft":false,"title":"x","reviewDecision":"","statusCheckRollup":[]}"#;
        let s = github_status(merged).unwrap();
        assert_eq!(
            (s.state, s.checks, s.review),
            (Some(State::Merged), None, None)
        );
    }

    #[test]
    fn azure_status_from_az() {
        let out = r#"{"isDraft": false, "status": "active", "title": "Add the export button",
            "repo": "https://acme.visualstudio.com/P/_git/fe", "votes": []}
@@
[{"status": "approved", "type": "Build"}, {"status": "queued", "type": "Minimum number of reviewers"}]"#;
        let s = azure_status(out, 4209).unwrap();
        assert_eq!(s.state, Some(State::Open));
        assert_eq!(s.checks, Some(Checks::Passing));
        assert_eq!(s.review, Some(Review::Required));
        assert_eq!(
            s.url.as_deref(),
            Some("https://acme.visualstudio.com/P/_git/fe/pullrequest/4209")
        );
        let rejected = out
            .replace(r#""votes": []"#, r#""votes": [10, -5]"#)
            .replace("approved", "rejected");
        let s = azure_status(&rejected, 1).unwrap();
        assert_eq!(
            (s.checks, s.review),
            (Some(Checks::Failing), Some(Review::ChangesRequested))
        );
        let done = out.replace("active", "completed");
        assert_eq!(azure_status(&done, 1).unwrap().state, Some(State::Merged));
    }

    #[test]
    fn shell_words_and_flags() {
        assert_eq!(
            flag(r#"az x --org "https://a b" --p y"#, &["--org"]).as_deref(),
            Some("https://a b")
        );
        assert_eq!(flag("az x --org=o", &["--org"]).as_deref(), Some("o"));
        assert_eq!(flag("az x", &["--org"]), None);
        assert_eq!(quote("it's"), r"'it'\''s'");
    }
}
