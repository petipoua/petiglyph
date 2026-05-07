use image::{GrayImage, RgbaImage};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

const DEBUG_ENV: &str = "PETIGLYPH_DEBUG";
const DEBUG_DIR_NAME: &str = "debug";
const DEBUG_ARTIFACTS_DIR_NAME: &str = "artifacts";
const DEBUG_LOG_FILE_NAME: &str = "pipeline.log";

#[derive(Debug, Clone)]
struct DebugSession {
    debug_dir: PathBuf,
    artifacts_dir: PathBuf,
    log_path: PathBuf,
    sequence: u64,
}

static DEBUG_SESSION: OnceLock<Mutex<Option<DebugSession>>> = OnceLock::new();

fn session_lock() -> &'static Mutex<Option<DebugSession>> {
    DEBUG_SESSION.get_or_init(|| Mutex::new(None))
}

fn with_session_mut<T>(f: impl FnOnce(&mut DebugSession) -> T) -> Option<T> {
    let lock = session_lock();
    let mut guard = match lock.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let session = guard.as_mut()?;
    Some(f(session))
}

pub(crate) fn debug_enabled() -> bool {
    std::env::var_os(DEBUG_ENV)
        .map(|value| {
            let value = value.to_string_lossy();
            !(value.is_empty()
                || value.eq_ignore_ascii_case("0")
                || value.eq_ignore_ascii_case("false")
                || value.eq_ignore_ascii_case("off"))
        })
        .unwrap_or(false)
}

pub(crate) fn set_debug_enabled(enabled: bool) {
    if enabled {
        // SAFETY: set from single-threaded CLI startup before worker threads are spawned.
        unsafe { std::env::set_var(DEBUG_ENV, "1") };
    } else {
        // SAFETY: unset from single-threaded CLI startup before worker threads are spawned.
        unsafe { std::env::remove_var(DEBUG_ENV) };
    }
}

pub(crate) fn begin_session(project_dir: &Path, context: &str) {
    if !debug_enabled() {
        return;
    }

    let debug_dir = project_dir.join(DEBUG_DIR_NAME);
    let artifacts_dir = debug_dir.join(DEBUG_ARTIFACTS_DIR_NAME);
    let log_path = debug_dir.join(DEBUG_LOG_FILE_NAME);

    let _ = fs::create_dir_all(&debug_dir);
    if artifacts_dir.exists() {
        let _ = fs::remove_dir_all(&artifacts_dir);
    }
    let _ = fs::create_dir_all(&artifacts_dir);

    let mut session = DebugSession {
        debug_dir,
        artifacts_dir,
        log_path,
        sequence: 0,
    };

    let now = debug_timestamp();
    let _ = fs::write(
        &session.log_path,
        format!("[{now}] debug session start: {context}\n"),
    );
    let _ = fs::write(
        session.debug_dir.join("README.txt"),
        "petiglyph debug artifacts\n\n- pipeline.log: step-by-step processing log\n- artifacts/: PNG/TXT snapshots for each image-to-glyph stage\n",
    );

    let seq = next_sequence(&mut session);
    append_log_line(
        &session.log_path,
        seq,
        "session",
        &format!("context={context}"),
    );

    let lock = session_lock();
    let mut guard = match lock.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    *guard = Some(session);
}

pub(crate) fn session_log_path(project_dir: &Path) -> PathBuf {
    project_dir.join(DEBUG_DIR_NAME).join(DEBUG_LOG_FILE_NAME)
}

pub(crate) fn read_recent_log_lines(path: &Path, max_lines: usize) -> Vec<String> {
    let Ok(content) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let lines = content.lines().map(str::to_string).collect::<Vec<_>>();
    if lines.len() <= max_lines {
        return lines;
    }
    lines[lines.len() - max_lines..].to_vec()
}

pub(crate) fn log_step(event: &str, details: impl AsRef<str>) {
    if !debug_enabled() {
        return;
    }

    let details = details.as_ref().to_string();
    let _ = with_session_mut(|session| {
        let seq = next_sequence(session);
        append_log_line(&session.log_path, seq, event, &details);
    });
}

pub(crate) fn write_rgba_png(step: &str, label: &str, image: &RgbaImage) {
    if !debug_enabled() {
        return;
    }

    let step = sanitize(step);
    let label = sanitize(label);
    let _ = with_session_mut(|session| {
        let seq = next_sequence(session);
        let filename = format!("{seq:05}_{step}_{label}.png");
        let path = session.artifacts_dir.join(filename);
        if image.save(&path).is_ok() {
            append_log_line(
                &session.log_path,
                seq,
                "artifact",
                &format!(
                    "rgba {}x{} {}",
                    image.width(),
                    image.height(),
                    path.display()
                ),
            );
        }
    });
}

pub(crate) fn write_coverage_png(step: &str, label: &str, width: u32, height: u32, data: &[u8]) {
    if !debug_enabled() {
        return;
    }
    if data.len() != (width as usize).saturating_mul(height as usize) {
        return;
    }

    let Some(image) = GrayImage::from_raw(width, height, data.to_vec()) else {
        return;
    };

    let step = sanitize(step);
    let label = sanitize(label);
    let _ = with_session_mut(|session| {
        let seq = next_sequence(session);
        let filename = format!("{seq:05}_{step}_{label}.png");
        let path = session.artifacts_dir.join(filename);
        if image.save(&path).is_ok() {
            append_log_line(
                &session.log_path,
                seq,
                "artifact",
                &format!("coverage {}x{} {}", width, height, path.display()),
            );
        }
    });
}

pub(crate) fn write_ascii_coverage(
    step: &str,
    label: &str,
    width: u32,
    height: u32,
    data: &[u8],
    threshold: u8,
) {
    if !debug_enabled() {
        return;
    }
    if data.len() != (width as usize).saturating_mul(height as usize) {
        return;
    }

    let mut lines = String::new();
    lines.push_str(&format!(
        "# {} {}x{} threshold={}\n",
        label, width, height, threshold
    ));
    for y in 0..height as usize {
        for x in 0..width as usize {
            let idx = y * width as usize + x;
            let ch = if data[idx] >= threshold { '#' } else { '.' };
            lines.push(ch);
        }
        lines.push('\n');
    }

    let step = sanitize(step);
    let label = sanitize(label);
    let _ = with_session_mut(|session| {
        let seq = next_sequence(session);
        let filename = format!("{seq:05}_{step}_{label}.txt");
        let path = session.artifacts_dir.join(filename);
        if fs::write(&path, lines.as_bytes()).is_ok() {
            append_log_line(
                &session.log_path,
                seq,
                "artifact",
                &format!("ascii {}x{} {}", width, height, path.display()),
            );
        }
    });
}

pub(crate) fn write_bitmap_png(step: &str, label: &str, width: u32, height: u32, bits: &[bool]) {
    if !debug_enabled() {
        return;
    }
    if bits.len() != (width as usize).saturating_mul(height as usize) {
        return;
    }

    let pixels = bits
        .iter()
        .map(|on| if *on { 255 } else { 0 })
        .collect::<Vec<_>>();
    let Some(image) = GrayImage::from_raw(width, height, pixels) else {
        return;
    };

    let step = sanitize(step);
    let label = sanitize(label);
    let _ = with_session_mut(|session| {
        let seq = next_sequence(session);
        let filename = format!("{seq:05}_{step}_{label}.png");
        let path = session.artifacts_dir.join(filename);
        if image.save(&path).is_ok() {
            append_log_line(
                &session.log_path,
                seq,
                "artifact",
                &format!("bitmap {}x{} {}", width, height, path.display()),
            );
        }
    });
}

fn next_sequence(session: &mut DebugSession) -> u64 {
    session.sequence = session.sequence.saturating_add(1);
    session.sequence
}

fn append_log_line(path: &Path, seq: u64, event: &str, details: &str) {
    let mut file = match OpenOptions::new().append(true).create(true).open(path) {
        Ok(file) => file,
        Err(_) => return,
    };
    let _ = writeln!(
        file,
        "[{} #{:05}] {}: {}",
        debug_timestamp(),
        seq,
        event,
        details
    );
}

fn sanitize(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut last_underscore = false;
    for ch in value.chars() {
        let keep = ch.is_ascii_alphanumeric() || ch == '-' || ch == '_';
        if keep {
            out.push(ch.to_ascii_lowercase());
            last_underscore = false;
        } else if !last_underscore {
            out.push('_');
            last_underscore = true;
        }
    }
    let trimmed = out.trim_matches('_');
    if trimmed.is_empty() {
        "item".to_string()
    } else {
        trimmed.to_string()
    }
}

fn debug_timestamp() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    millis.to_string()
}
