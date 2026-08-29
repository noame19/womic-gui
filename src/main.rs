// WO Mic GUI - minimal Slint wrapper for the WO Mic CLI client
//
// Spawns the official AppImage micclient, parses its stdout/stderr to drive
// the UI status, and exposes settings (micclient path, auto-load snd-aloop)
// that persist to ~/.config/womic-gui/config.json.


use std::fs;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};

slint::include_modules!();
use slint::app::AppWindow;

const LOG_MAX_LINES: usize = 200;
const POLL_INTERVAL_MS: u64 = 500;
const SIGINT_GRACE_MS: u64 = 1500;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Config {
    #[serde(default)]
    address: String,
    #[serde(default = "default_mode")]
    mode: String,
    #[serde(default)]
    micclient_path: String,
    #[serde(default)]
    auto_aloop: bool,
}

fn default_mode() -> String { "Wifi".into() }

fn config_path() -> PathBuf {
    let mut p = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    p.push("womic-gui");
    let _ = fs::create_dir_all(&p);
    p.push("config.json");
    p
}

fn load_config() -> Config {
    fs::read_to_string(config_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_config(c: &Config) {
    if let Ok(s) = serde_json::to_string_pretty(c) {
        let _ = fs::write(config_path(), s);
    }
}

fn is_aloop_loaded() -> bool {
    fs::read_to_string("/proc/modules")
        .map(|s| s.lines().any(|l| l.starts_with("snd_aloop ")))
        .unwrap_or(false)
}

/// Try to load snd-aloop. Prefer pkexec (GUI password prompt), fall back to sudo.
fn load_aloop() -> Result<(), String> {
    for cmd in &["pkexec", "sudo"] {
        let status = Command::new(cmd).args(["modprobe", "snd-aloop"]).status();
        if let Ok(s) = status {
            if s.success() { return Ok(()); }
        }
    }
    Err("both pkexec and sudo failed (isPolicyKit/sudo installed?)".into())
}

fn default_mic_path() -> String {
    std::env::var("HOME")
        .map(|h| PathBuf::from(h).join("下载").join("micclient-x86_64.AppImage").to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn append_log(ui: &AppWindow, log: &Arc<Mutex<Vec<String>>>, line: impl Into<String>) {
    let mut buf = log.lock().unwrap();
    buf.push(line.into());
    let n = buf.len();
    if n > LOG_MAX_LINES { buf.drain(..n - LOG_MAX_LINES); }
    let s = buf.join("\n");
    ui.set_log(s.into());
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = AppWindow::new()?;
    let cfg = load_config();

    ui.set_address(cfg.address.clone().into());
    ui.set_mode(if cfg.mode.is_empty() { "Wifi".into() } else { cfg.mode.clone().into() });
    ui.set_micclient_path(if cfg.micclient_path.is_empty() {
        default_mic_path().into()
    } else {
        cfg.micclient_path.clone().into()
    });
    ui.set_auto_aloop(cfg.auto_aloop);
    ui.set_status("Disconnected".into());
    ui.set_status_color("#909399".into());
    ui.set_log(String::new().into());
    ui.set_connecting(false);
    ui.set_connected(false);

    let log_buf: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let mic: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));
    let cfg_arc: Arc<Mutex<Config>> = Arc::new(Mutex::new(cfg));

    // --- save settings ---
    {
        let cfg_for_save = cfg_arc.clone();
        let ui_for_save = ui.as_weak();
        ui.on_save_settings(move |auto_aloop, path| {
            let mut c = cfg_for_save.lock().unwrap();
            c.auto_aloop = auto_aloop;
            c.micclient_path = path.to_string();
            let snap = c.clone();
            drop(c);
            save_config(&snap);
            
        });
    }

    // --- browse micclient (opens Downloads in file manager; user pastes path back) ---
    {
        ui.on_browse_micclient(move || {
            let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
            let target = home.join("下载");
            let _ = Command::new("xdg-open").arg(&target).spawn();
        });
    }

    // --- connect ---
    {
        let mic = mic.clone();
        let log = log_buf.clone();
        let cfg_arc = cfg_arc.clone();
        let ui_weak = ui.as_weak();
        ui.on_connect(move |mode, address| {
            let ui = match ui_weak.upgrade() {
                Some(u) => u,
                None => return,
            };
            let cfg = cfg_arc.lock().unwrap().clone();
            let mic_path = if cfg.micclient_path.is_empty() {
                default_mic_path()
            } else {
                cfg.micclient_path.clone()
            };

            append_log(&ui, &log, format!("$ {} -t {} {}", mic_path, mode, address));

            if !PathBuf::from(&mic_path).exists() {
                append_log(&ui, &log, format!("  !! micclient not found: {mic_path}"));
                ui.set_status("micclient not found".into());
                ui.set_status_color("#f56c6c".into());
                ui.set_connecting(false);
                return;
            }

            if !is_aloop_loaded() {
                if cfg.auto_aloop {
                    append_log(&ui, &log, "Loading snd-aloop…".into());
                    match load_aloop() {
                        Ok(_) => append_log(&ui, &log, "  ok snd-aloop loaded".into()),
                        Err(e) => {
                            append_log(&ui, &log, format!("  !! load_aloop: {e}"));
                            ui.set_status("Failed to load snd-aloop".into());
                            ui.set_status_color("#f56c6c".into());
                            ui.set_connecting(false);
                            return;
                        }
                    }
                } else {
                    append_log(&ui, &log,
                        "  !! snd-aloop not loaded. Run `sudo modprobe snd-aloop` or enable Auto-load.".into());
                    ui.set_status("snd-aloop not loaded".into());
                    ui.set_status_color("#f56c6c".into());
                    ui.set_connecting(false);
                    return;
                }
            }

            // spawn micclient
            let mut cmd = Command::new(&mic_path);
            cmd.arg("-t").arg(mode.to_string()).arg(address.to_string());
            cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
            let mut child = match cmd.spawn() {
                Ok(c) => c,
                Err(e) => {
                    append_log(&ui, &log, format!("  !! spawn failed: {e}"));
                    ui.set_status("Failed to start micclient".into());
                    ui.set_status_color("#f56c6c".into());
                    ui.set_connecting(false);
                    return;
                }
            };
            let pid = child.id();
            append_log(&ui, &log, format!("  -> spawned pid={pid}"));

            let stdout = child.stdout.take().expect("stdout piped");
            let stderr = child.stderr.take().expect("stderr piped");

            // stdout reader
            let ui_w = ui.as_weak();
            let log_w = log.clone();
            thread::spawn(move || {
                let r = BufReader::new(stdout);
                for line in r.lines().map_while(Result::ok) {
                    let line_clone = line.clone();
                    let lower = line.to_lowercase();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = ui_w.upgrade() {
                            append_log(&ui, &log_w, line_clone);
                            if lower.contains("connected") && !lower.contains("disconnect") {
                                ui.set_status("Connected".into());
                                ui.set_status_color("#67c23a".into());
                                ui.set_connected(true);
                                ui.set_connecting(false);
                            }
                        }
                    });
                }
            });

            // stderr reader
            let ui_w = ui.as_weak();
            let log_w = log.clone();
            thread::spawn(move || {
                let r = BufReader::new(stderr);
                for line in r.lines().map_while(Result::ok) {
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = ui_w.upgrade() {
                            append_log(&ui, &log_w, format!("[stderr] {line}"));
                        }
                    });
                }
            });

            // store child FIRST so the watcher can pick it up
            *mic.lock().unwrap() = Some(child);

            // watcher: poll child, when it exits -> mark disconnected.
            // Tolerates None (e.g. user hit Disconnect mid-poll) by continuing.
            let ui_w = ui.as_weak();
            let log_w = log.clone();
            let mic_w = mic.clone();
            thread::spawn(move || {
                loop {
                    thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
                    let mut guard = mic_w.lock().unwrap();
                    let Some(child_ref) = guard.as_mut() else {
                        // no active child, keep polling in case a new one starts
                        drop(guard);
                        continue;
                    };
                    match child_ref.try_wait() {
                        Ok(Some(status)) => {
                            *guard = None;
                            drop(guard);
                            let _ = slint::invoke_from_event_loop(move || {
                                if let Some(ui) = ui_w.upgrade() {
                                    let msg = if status.success() {
                                        "Disconnected"
                                    } else {
                                        "micclient exited unexpectedly"
                                    };
                                    let color = if status.success() { "#909399" } else { "#f56c6c" };
                                    ui.set_status(msg.into());
                                    ui.set_status_color(color.into());
                                    ui.set_connected(false);
                                    ui.set_connecting(false);
                                    append_log(&ui, &log_w, format!("  ok micclient exited: {status}"));
                                }
                            });
                            return;
                        }
                        Ok(None) => continue,
                        Err(e) => {
                            drop(guard);
                            let _ = slint::invoke_from_event_loop(move || {
                                if let Some(ui) = ui_w.upgrade() {
                                    append_log(&ui, &log_w, format!("  wait error: {e}"));
                                }
                            });
                            return;
                        }
                    }
                }
            });
        });
    }

    // --- disconnect ---
    {
        let mic = mic.clone();
        let log = log_buf.clone();
        let ui_weak = ui.as_weak();
        ui.on_disconnect(move || {
            let ui = match ui_weak.upgrade() {
                Some(u) => u,
                None => return,
            };
            append_log(&ui, &log, "Disconnect requested…".into());

            let child_opt = mic.lock().unwrap().take();
            match child_opt {
                Some(mut child) => {
                    let pid = child.id();
                    append_log(&ui, &log, format!("  -> SIGINT to pid={pid}"));
                    unsafe { libc::kill(pid as libc::pid_t, libc::SIGINT); }
                    thread::sleep(Duration::from_millis(SIGINT_GRACE_MS));
                    match child.try_wait() {
                        Ok(Some(_)) => append_log(&ui, &log, "  ok exited cleanly".into()),
                        _ => {
                            let _ = child.kill();
                            let _ = child.wait();
                            append_log(&ui, &log, "  ok killed (SIGKILL)".into());
                        }
                    }
                }
                None => append_log(&ui, &log, "  (no running micclient)".into()),
            }

            ui.set_status("Disconnected".into());
            ui.set_status_color("#909399".into());
            ui.set_connected(false);
            ui.set_connecting(false);
        });
    }

    // --- close window: kill micclient if running ---
    {
        let mic = mic.clone();
        let ui_weak = ui.as_weak();
        ui.window().on_close_requested(move || {
            if let Some(mut child) = mic.lock().unwrap().take() {
                let pid = child.id();
                unsafe { libc::kill(pid as libc::pid_t, libc::SIGINT); }
                thread::sleep(Duration::from_millis(800));
                let _ = child.kill();
                let _ = child.wait();
                
            }
            slint::CloseRequestResponse::HideWindow
        });
    }


    ui.run()
}
