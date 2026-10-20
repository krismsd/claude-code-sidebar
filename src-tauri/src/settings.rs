//! Backing for the settings window: reads config.toml and projects.toml for it, and writes
//! its changes back into the files, keeping the user's comments and layout. The file watcher
//! then applies them like any hand edit.
use crate::config::{self, Config};
use crate::project::{self, Override, Overrides};
use serde::Serialize;
use toml_edit::{DocumentMut, Item, Table};

#[derive(Debug, Serialize)]
pub struct Settings {
    pub config: Config,
    pub projects: Vec<Override>,
    /// Tracked projects, with what they show when nothing is overridden.
    pub tracked: Vec<TrackedDefaults>,
    pub palette: Vec<&'static str>,
    pub config_path: String,
    pub projects_path: String,
}

#[derive(Debug, Serialize)]
pub struct TrackedDefaults {
    pub root: String,
    pub name: String,
    pub color: String,
}

pub fn load(tracked_roots: &[String]) -> Settings {
    let tracked = tracked_roots
        .iter()
        .map(|root| TrackedDefaults {
            root: root.clone(),
            name: root
                .trim_end_matches('/')
                .rsplit('/')
                .next()
                .unwrap_or(root)
                .to_string(),
            color: project::color_for(root),
        })
        .collect();
    Settings {
        config: Config::load(),
        projects: Overrides::load().project,
        tracked,
        palette: project::PALETTE.to_vec(),
        config_path: path_str(config::path()),
        projects_path: path_str(Overrides::path()),
    }
}

fn path_str(p: Option<std::path::PathBuf>) -> String {
    p.map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Check a config from the window before it is written.
pub fn validate(c: &Config) -> Result<(), String> {
    let one_of = |name: &str, v: &str, ok: &[&str]| {
        if ok.contains(&v) {
            Ok(())
        } else {
            Err(format!("{name} must be one of {}", ok.join(", ")))
        }
    };
    one_of("mode", &c.mode, &["docked", "floating"])?;
    one_of("edge", &c.edge, &["right", "left"])?;
    one_of("toasts", &c.toasts, &["when_hidden", "always", "never"])?;
    one_of(
        "session_mode",
        &c.session_mode,
        &["terminal", "tmux", "window"],
    )?;
    if !(200..=900).contains(&c.width) {
        return Err("Width must be between 200 and 900".into());
    }
    if c.distro.trim().is_empty() || c.launcher.trim().is_empty() {
        return Err("WSL distro and launcher can't be empty".into());
    }
    if c.terminal_window.trim().is_empty() || c.terminal_window.contains(';') {
        return Err("Terminal window must be \"0\", \"new\" or a name without ';'".into());
    }
    Ok(())
}

pub fn validate_projects(projects: &[Override]) -> Result<(), String> {
    for p in projects {
        if let Some(c) = &p.color
            && !(c.len() == 7
                && c.starts_with('#')
                && c[1..].chars().all(|h| h.is_ascii_hexdigit()))
        {
            return Err(format!("{}: colour must look like #3e63dd", p.path));
        }
        if p.tasks.as_ref().is_some_and(|t| t.query.trim().is_empty()) {
            return Err(format!("{}: tasks need a query", p.path));
        }
    }
    Ok(())
}

pub fn save_config(c: &Config) -> Result<(), String> {
    validate(c)?;
    let path = config::path().ok_or("no APPDATA")?;
    let _ = Config::load(); // creates the file from the template if missing
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let new = toml::to_string(c).map_err(|e| e.to_string())?;
    write_if_changed(&path, &text, &merged(&text, &new, false)?)
}

pub fn save_projects(projects: &[Override]) -> Result<(), String> {
    validate_projects(projects)?;
    let path = Overrides::path().ok_or("no APPDATA")?;
    let text = std::fs::read_to_string(&path).unwrap_or_else(|_| project::PROJECTS_TEMPLATE.into());
    let new = toml::to_string(&Overrides {
        project: projects.to_vec(),
    })
    .map_err(|e| e.to_string())?;
    // With no projects left the serializer writes nothing, so drop the array explicitly.
    let mut out = merged(&text, &new, true)?;
    if projects.is_empty()
        && let Ok(mut doc) = out.parse::<DocumentMut>()
    {
        doc.remove("project");
        out = doc.to_string();
    }
    write_if_changed(&path, &text, &out)
}

/// Writing an unchanged file would still trip the watcher (and move the AppBar), so don't.
fn write_if_changed(path: &std::path::Path, old: &str, new: &str) -> Result<(), String> {
    if old == new {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::write(path, new).map_err(|e| e.to_string())
}

/// `old` with the values of `new` (both TOML) merged in. Unchanged values keep their exact
/// text and comments; changed ones keep their comment. Keys missing from `new` are removed,
/// except at the top level when `prune_root` is false (so unknown settings survive).
fn merged(old: &str, new: &str, prune_root: bool) -> Result<String, String> {
    let mut doc: DocumentMut = old
        .parse()
        .map_err(|e| format!("can't edit the file: {e}"))?;
    let new: DocumentMut = new.parse().map_err(|e| format!("{e}"))?;
    merge(doc.as_table_mut(), new.as_table(), prune_root);
    let mut n = 0;
    renumber(doc.as_table_mut(), &mut n);
    Ok(doc.to_string())
}

fn merge(old: &mut Table, new: &Table, prune: bool) {
    if prune {
        let gone: Vec<String> = old
            .iter()
            .filter(|(k, _)| !new.contains_key(k))
            .map(|(k, _)| k.to_string())
            .collect();
        for k in gone {
            old.remove(&k);
        }
    }
    for (k, nv) in new.iter() {
        match (old.get_mut(k), nv) {
            (Some(Item::Value(ov)), Item::Value(v)) => {
                if !same_value(ov, v) {
                    let decor = ov.decor().clone();
                    *ov = v.clone();
                    *ov.decor_mut() = decor;
                }
            }
            (Some(Item::Table(ot)), Item::Table(t)) => merge(ot, t, true),
            (Some(Item::ArrayOfTables(oa)), Item::ArrayOfTables(a)) => {
                // Entries are matched up by `path` (projects.toml); unmatched ones are new.
                let path_of = |t: &Table| t.get("path").and_then(|p| p.as_str()).map(String::from);
                let mut out = toml_edit::ArrayOfTables::new();
                for t in a.iter() {
                    let existing = oa
                        .iter()
                        .find(|o| path_of(o).is_some() && path_of(o) == path_of(t));
                    let mut entry = existing.cloned().unwrap_or_else(|| {
                        let mut e = Table::new();
                        e.decor_mut().set_prefix("\n");
                        e
                    });
                    merge(&mut entry, t, true);
                    out.push(entry);
                }
                *oa = out;
            }
            _ => {
                old.insert(k, nv.clone());
            }
        }
    }
}

/// Equal as TOML values, however they are written (quoting, escapes, spacing).
fn same_value(a: &toml_edit::Value, b: &toml_edit::Value) -> bool {
    let parse = |v: &toml_edit::Value| {
        let mut bare = v.clone();
        bare.decor_mut().clear();
        toml::from_str::<toml::Table>(&format!("v = {bare}")).ok()
    };
    parse(a).is_some_and(|x| Some(x) == parse(b))
}

/// Tables are written in the order of their positions; give new ones (taken from another
/// document) positions that follow the logical order, so each sits under its parent.
fn renumber(table: &mut Table, n: &mut isize) {
    table.set_position(Some(*n));
    *n += 1;
    for (_, item) in table.iter_mut() {
        match item {
            Item::Table(t) => renumber(t, n),
            Item::ArrayOfTables(a) => a.iter_mut().for_each(|t| renumber(t, n)),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_merge_keeps_comments_and_unknown_keys() {
        let old = "# top\nwidth = 320 # px\nfuture_key = 1\n\n[sounds]\ndenied = \"SystemHand\" # 🚫\nslow = \"\"\n";
        let mut c: Config = toml::from_str(old).unwrap();
        c.width = 400;
        c.sounds.remove("slow");
        c.launch_env.insert("A".into(), "b".into());
        let out = merged(old, &toml::to_string(&c).unwrap(), false).unwrap();
        // Same value, other quoting: left alone.
        let quoted = "launcher = 'claudes'\n";
        let c: Config = toml::from_str(quoted).unwrap();
        assert!(
            merged(quoted, &toml::to_string(&c).unwrap(), false)
                .unwrap()
                .starts_with(quoted)
        );
        assert!(
            out.starts_with("# top\nwidth = 400 # px\nfuture_key = 1\n"),
            "{out}"
        );
        assert!(out.contains("denied = \"SystemHand\" # 🚫\n"), "{out}");
        assert!(!out.contains("slow ="), "{out}");
        let back: Config = toml::from_str(&out).unwrap();
        assert_eq!(back.width, 400);
        assert_eq!(back.launch_env["A"], "b");
        // Nothing changed: identical text.
        let same: Config = toml::from_str(&out).unwrap();
        assert_eq!(
            merged(&out, &toml::to_string(&same).unwrap(), false).unwrap(),
            out
        );
    }

    #[test]
    fn template_round_trips_unchanged() {
        let t = config::TEMPLATE;
        let c: Config = toml::from_str(t).unwrap();
        assert_eq!(merged(t, &toml::to_string(&c).unwrap(), false).unwrap(), t);
    }

    #[test]
    fn projects_merge_by_path() {
        let old = format!(
            "{}\n[[project]]\npath = \"/a\"\nname = \"A\" # mine\n\n[project.tasks]\nquery = \"q\"\n\n[[project]]\npath = \"/b\"\nemoji = \"🚀\"\n",
            project::PROJECTS_TEMPLATE
        );
        let mut p = Overrides::parse(&old).unwrap().project;
        p.remove(1); // drop /b
        p[0].label = Some("Team".into());
        p.push(Override {
            path: "/c".into(),
            name: Some("C".into()),
            tasks: Some(project::TasksConfig {
                connector: "Atlassian".into(),
                query: "mine".into(),
                tools: vec![],
                prompt: None,
            }),
            ..Default::default()
        });
        let new = toml::to_string(&Overrides { project: p }).unwrap();
        let out = merged(&old, &new, true).unwrap();
        assert!(out.starts_with(project::PROJECTS_TEMPLATE), "{out}");
        assert!(out.contains("name = \"A\" # mine"), "{out}");
        assert!(!out.contains("/b"), "{out}");
        let back = Overrides::parse(&out).unwrap().project;
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].label.as_deref(), Some("Team"));
        assert_eq!(back[0].tasks.as_ref().unwrap().query, "q");
        assert_eq!(back[1].path, "/c");
        assert_eq!(back[1].tasks.as_ref().unwrap().connector, "Atlassian");
    }
}
