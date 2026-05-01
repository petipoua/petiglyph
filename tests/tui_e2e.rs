#![cfg(unix)]

use expectrl::{session::OsSession, Eof, Expect, Session};
use image::{Rgba, RgbaImage};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static TUI_E2E_LOCK: Mutex<()> = Mutex::new(());

fn make_temp_dir(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time is valid")
        .as_nanos();
    let dir = env::temp_dir().join(format!("petiglyph-tui-e2e-{name}-{nonce}"));
    fs::create_dir_all(&dir).expect("temp dir is created");
    dir
}

fn env_flag(name: &str) -> bool {
    env::var(name)
        .ok()
        .map(|value| {
            let value = value.trim().to_ascii_lowercase();
            !matches!(value.as_str(), "" | "0" | "false" | "off" | "no")
        })
        .unwrap_or(false)
}

fn env_u64(name: &str, default: u64) -> u64 {
    env::var(name)
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .unwrap_or(default)
}

fn e2e_lock() -> MutexGuard<'static, ()> {
    TUI_E2E_LOCK.lock().unwrap_or_else(|err| err.into_inner())
}

fn write_test_png(path: &Path) {
    let mut img = RgbaImage::from_pixel(8, 8, Rgba([255, 255, 255, 0]));
    img.put_pixel(2, 2, Rgba([0, 0, 0, 255]));
    img.put_pixel(5, 5, Rgba([0, 0, 0, 255]));
    img.save(path).expect("test image should be written");
}

fn run_petiglyph(cwd: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_petiglyph"))
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("petiglyph command should run")
}

fn create_project_with_icon(workspace: &Path, project_name: &str) -> PathBuf {
    let create = run_petiglyph(workspace, &["create", project_name, "--no-launch"]);
    assert!(create.status.success(), "create command should succeed");

    let project_dir = workspace.join(project_name);
    let icons_dir = project_dir.join("icons");
    write_test_png(&icons_dir.join("alpha.png"));
    project_dir
}

fn spawn_tui_session(cwd: &Path, args: &[&str]) -> OsSession {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_petiglyph"));
    cmd.current_dir(cwd).args(args);
    if env_flag("PETIGLYPH_E2E_TUI_DEBUG") {
        cmd.env("PETIGLYPH_TUI_DEBUG", "1");
    }

    let mut session = Session::spawn(cmd).expect("petiglyph TUI session should start");
    session.set_expect_timeout(Some(Duration::from_secs(10)));
    session
}

fn trace_step(label: &str) {
    if env_flag("PETIGLYPH_E2E_TRACE") {
        eprintln!("[tui-e2e] {label}");
    }
}

fn send_step(session: &mut OsSession, payload: &str, label: &str) {
    trace_step(label);
    session.send(payload).expect("send should succeed");
    let pause_ms = env_u64("PETIGLYPH_E2E_STEP_DELAY_MS", 0);
    if pause_ms > 0 {
        thread::sleep(Duration::from_millis(pause_ms));
    }
}

fn wait_for_path(path: &Path, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if path.exists() {
            return;
        }
        thread::sleep(Duration::from_millis(30));
    }

    assert!(
        path.exists(),
        "expected path to appear before timeout: {}",
        path.display()
    );
}

fn find_ttf_in_dir(dir: &Path) -> Option<PathBuf> {
    let entries = fs::read_dir(dir).ok()?;
    for entry in entries {
        let path = entry.ok()?.path();
        let is_ttf = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("ttf"));
        if is_ttf {
            return Some(path);
        }
    }
    None
}

fn wait_for_ttf_in_dir(dir: &Path, timeout: Duration) -> PathBuf {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Some(path) = find_ttf_in_dir(dir) {
            return path;
        }
        thread::sleep(Duration::from_millis(30));
    }

    panic!(
        "expected at least one .ttf in build directory before timeout: {}",
        dir.display()
    );
}

#[test]
fn tui_e2e_launch_and_quit_from_existing_project() {
    let _guard = e2e_lock();
    let workspace = make_temp_dir("launch-quit");
    let project_dir = create_project_with_icon(&workspace, "launch-quit-demo");

    let mut session = spawn_tui_session(&project_dir, &[]);
    trace_step("waiting for TUI header");
    session
        .expect(" petiglyph ")
        .expect("tui header should be visible");
    send_step(&mut session, "q", "quit (q)");
    session
        .expect("tui session closed for")
        .expect("session should close cleanly");
    session.expect(Eof).expect("tui process should exit");

    fs::remove_dir_all(workspace).expect("temp dir is removed");
}

#[test]
fn tui_e2e_create_project_from_home_panel() {
    let _guard = e2e_lock();
    let workspace = make_temp_dir("create-project");
    let project_name = "from-tui-e2e";
    let project_dir = workspace.join(project_name);
    let manifest_path = project_dir.join("petiglyph.toml");
    let icons_dir = project_dir.join("icons");

    let mut session = spawn_tui_session(&workspace, &[]);
    thread::sleep(Duration::from_millis(200));

    send_step(&mut session, "\r", "enter typing mode");
    send_step(&mut session, project_name, "type project name");
    send_step(&mut session, "\r", "focus create button");
    send_step(&mut session, "\r", "submit project creation");
    send_step(&mut session, "q", "quit after creation");

    session
        .expect("tui session closed for")
        .expect("session should close cleanly");
    session.expect(Eof).expect("tui process should exit");

    assert!(
        project_dir.is_dir(),
        "created project directory should exist"
    );
    assert!(manifest_path.is_file(), "manifest should be created");
    assert!(icons_dir.is_dir(), "icons directory should be created");

    fs::remove_dir_all(workspace).expect("temp dir is removed");
}

#[test]
fn tui_e2e_build_shortcut_produces_outputs() {
    let _guard = e2e_lock();
    let workspace = make_temp_dir("build");
    let project_dir = create_project_with_icon(&workspace, "build-demo");
    let build_dir = project_dir.join("build");
    let mapping_path = build_dir.join("glyph-map.json");
    let sample_path = build_dir.join("glyph-sample.txt");

    let mut session = spawn_tui_session(&project_dir, &[]);
    trace_step("waiting for TUI header");
    session
        .expect(" petiglyph ")
        .expect("tui header should be visible");
    send_step(&mut session, "b", "trigger build shortcut (b)");

    trace_step("waiting for build artifacts");
    wait_for_path(&mapping_path, Duration::from_secs(10));
    wait_for_path(&sample_path, Duration::from_secs(10));
    let ttf_path = wait_for_ttf_in_dir(&build_dir, Duration::from_secs(10));

    send_step(&mut session, "q", "quit after build");
    session
        .expect("tui session closed for")
        .expect("session should close cleanly");
    session.expect(Eof).expect("tui process should exit");

    assert!(mapping_path.is_file(), "glyph map should exist after build");
    assert!(sample_path.is_file(), "sample should exist after build");
    assert!(ttf_path.is_file(), "ttf should exist after build");

    fs::remove_dir_all(workspace).expect("temp dir is removed");
}
