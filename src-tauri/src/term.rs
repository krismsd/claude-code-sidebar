//! The sidebar's own terminal window ("window" session mode): shows tmux-hosted sessions in a
//! webview terminal (xterm.js) instead of Windows Terminal. Each open view is a pseudo console
//! (ConPTY) running `wsl.exe -d <distro> -e tmux -L cc-sidebar attach -t <name>`, the same way
//! VS Code's terminal reaches WSL. Closing a view only detaches: Claude keeps running in tmux.
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, NativePtySystem, PtySize, PtySystem};
use serde::Serialize;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager};

pub const LABEL: &str = "term";

struct View {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
}

/// Open views by tmux session name.
#[derive(Default, Clone)]
pub struct Views(Arc<Mutex<HashMap<String, View>>>);

#[derive(Clone, Serialize)]
struct Output<'a> {
    name: &'a str,
    data: String,
}

/// Show the terminal window (created on first use) and ask it to open `name`.
pub fn show(app: &tauri::AppHandle, name: &str, session_id: &str) -> tauri::Result<()> {
    let open = serde_json::json!({ "name": name, "session_id": session_id });
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = app.emit_to(LABEL, "term-open", open);
        let _ = w.unminimize();
        let _ = w.show();
        return w.set_focus();
    }
    let url = format!("index.html?view=term&open={name}&session={session_id}");
    tauri::WebviewWindowBuilder::new(app, LABEL, tauri::WebviewUrl::App(url.into()))
        .title("Claude Code")
        // No title bar: the tabs share a row with the window buttons, as in Windows Terminal.
        .decorations(false)
        .inner_size(1100.0, 760.0)
        .min_inner_size(500.0, 300.0)
        .center()
        .build()?;
    Ok(())
}

impl Views {
    /// Attach to tmux session `name` in a new pseudo console of `cols`×`rows`. Its output is
    /// sent to the terminal window as "term-output" events, and "term-exit" when it ends
    /// (the session ended, or it was detached).
    pub fn open(
        &self,
        app: &tauri::AppHandle,
        name: &str,
        distro: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(), String> {
        if self.0.lock().unwrap().contains_key(name) {
            return self.resize(name, cols, rows);
        }
        let pty = NativePtySystem::default()
            .openpty(size(cols, rows))
            .map_err(|e| e.to_string())?;
        let mut cmd = CommandBuilder::new("wsl.exe");
        cmd.args([
            "-d",
            distro,
            "-e",
            "tmux",
            "-L",
            "cc-sidebar",
            "attach",
            "-t",
            name,
        ]);
        let mut child = pty.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
        drop(pty.slave);
        let killer = child.clone_killer();
        let mut reader = pty.master.try_clone_reader().map_err(|e| e.to_string())?;
        let writer = pty.master.take_writer().map_err(|e| e.to_string())?;
        self.0.lock().unwrap().insert(
            name.to_string(),
            View {
                master: pty.master,
                writer,
                killer,
            },
        );
        // When the client exits (session ended, detached, or closed here), drop the console:
        // ConPTY only ends the output stream then.
        {
            let (app, views, name) = (app.clone(), self.clone(), name.to_string());
            std::thread::spawn(move || {
                let _ = child.wait();
                views.forget(&name);
                let _ = app.emit_to(LABEL, "term-exit", &name);
            });
        }
        let (app, name) = (app.clone(), name.to_string());
        std::thread::spawn(move || {
            let mut buf = [0u8; 16 * 1024];
            // A read can end inside a UTF-8 sequence: keep that tail for the next one.
            let mut pending: Vec<u8> = Vec::new();
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        pending.extend_from_slice(&buf[..n]);
                        let valid = match std::str::from_utf8(&pending) {
                            Ok(_) => pending.len(),
                            Err(e) if e.error_len().is_none() => e.valid_up_to(),
                            Err(_) => pending.len(), // invalid bytes: send them lossily
                        };
                        let data = String::from_utf8_lossy(&pending[..valid]).into_owned();
                        pending.drain(..valid);
                        let _ = app.emit_to(LABEL, "term-output", Output { name: &name, data });
                    }
                }
            }
        });
        Ok(())
    }

    pub fn write(&self, name: &str, data: &str) -> Result<(), String> {
        let mut views = self.0.lock().unwrap();
        let v = views.get_mut(name).ok_or("not open")?;
        v.writer
            .write_all(data.as_bytes())
            .and_then(|_| v.writer.flush())
            .map_err(|e| e.to_string())
    }

    pub fn resize(&self, name: &str, cols: u16, rows: u16) -> Result<(), String> {
        let views = self.0.lock().unwrap();
        let v = views.get(name).ok_or("not open")?;
        v.master.resize(size(cols, rows)).map_err(|e| e.to_string())
    }

    /// Stop showing `name` (the tmux client goes; the session keeps running).
    pub fn close(&self, name: &str) {
        if let Some(mut v) = self.0.lock().unwrap().remove(name) {
            let _ = v.killer.kill();
        }
    }

    /// Every view (the window closed). Returns the tmux sessions they showed.
    pub fn close_all(&self) -> Vec<String> {
        let mut names = Vec::new();
        for (name, mut v) in self.0.lock().unwrap().drain() {
            let _ = v.killer.kill();
            names.push(name);
        }
        names
    }

    fn forget(&self, name: &str) {
        self.0.lock().unwrap().remove(name);
    }
}

fn size(cols: u16, rows: u16) -> PtySize {
    PtySize {
        rows: rows.max(2),
        cols: cols.max(10),
        pixel_width: 0,
        pixel_height: 0,
    }
}
