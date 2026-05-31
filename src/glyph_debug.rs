use crossterm::terminal;
use image::imageops::FilterType;
use image::{GrayImage, RgbaImage};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

const DEBUG_ENV: &str = "PETIGLYPH_DEBUG";
const DEBUG_CELL_ENV: &str = "PETIGLYPH_DEBUG_CELL";
const DEBUG_DIR_NAME: &str = "debug";
const DEBUG_ARTIFACTS_DIR_NAME: &str = "artifacts";
const DEBUG_LOG_FILE_NAME: &str = "pipeline.log";

#[derive(Debug, Clone)]
struct DebugSession {
    artifacts_dir: PathBuf,
    log_path: PathBuf,
    sequence: u64,
    terminal_cell_width_px: u32,
    terminal_cell_height_px: u32,
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
        artifacts_dir,
        log_path,
        sequence: 0,
        terminal_cell_width_px: 1,
        terminal_cell_height_px: 2,
    };
    let (cell_w, cell_h, cell_source) = detect_terminal_cell_geometry();
    session.terminal_cell_width_px = cell_w;
    session.terminal_cell_height_px = cell_h;

    let now = debug_timestamp();
    let _ = fs::write(
        &session.log_path,
        format!("[{now}] debug session start: {context}\n"),
    );

    let seq = next_sequence(&mut session);
    append_log_line(
        &session.log_path,
        seq,
        "session",
        &format!(
            "context={context} cell={}x{} source={}",
            session.terminal_cell_width_px, session.terminal_cell_height_px, cell_source
        ),
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

        if let Some(terminal_image) = render_terminal_preview(
            &image,
            session.terminal_cell_width_px,
            session.terminal_cell_height_px,
        ) {
            let preview_seq = next_sequence(session);
            let preview_filename = format!("{preview_seq:05}_{step}_{label}_terminal_preview.png");
            let preview_path = session.artifacts_dir.join(preview_filename);
            if terminal_image.save(&preview_path).is_ok() {
                append_log_line(
                    &session.log_path,
                    preview_seq,
                    "artifact",
                    &format!(
                        "coverage-terminal {}x{} cell={}x{} {}",
                        terminal_image.width(),
                        terminal_image.height(),
                        session.terminal_cell_width_px,
                        session.terminal_cell_height_px,
                        preview_path.display()
                    ),
                );
            }
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

        if let Some(terminal_image) = render_terminal_preview(
            &image,
            session.terminal_cell_width_px,
            session.terminal_cell_height_px,
        ) {
            let preview_seq = next_sequence(session);
            let preview_filename = format!("{preview_seq:05}_{step}_{label}_terminal_preview.png");
            let preview_path = session.artifacts_dir.join(preview_filename);
            if terminal_image.save(&preview_path).is_ok() {
                append_log_line(
                    &session.log_path,
                    preview_seq,
                    "artifact",
                    &format!(
                        "bitmap-terminal {}x{} cell={}x{} {}",
                        terminal_image.width(),
                        terminal_image.height(),
                        session.terminal_cell_width_px,
                        session.terminal_cell_height_px,
                        preview_path.display()
                    ),
                );
            }
        }
    });
}

fn render_terminal_preview(
    image: &GrayImage,
    cell_width_px: u32,
    cell_height_px: u32,
) -> Option<GrayImage> {
    if cell_width_px == 0 || cell_height_px == 0 {
        return None;
    }

    let width = image.width();
    let height = image.height();
    if width == 0 || height == 0 {
        return None;
    }

    let scaled_w = (u64::from(width) * u64::from(cell_width_px) * 2
        + u64::from(cell_height_px) / 2)
        / u64::from(cell_height_px);
    let preview_w = u32::try_from(scaled_w).ok()?.max(1);
    if preview_w == width {
        return Some(image.clone());
    }

    Some(image::imageops::resize(
        image,
        preview_w,
        height,
        FilterType::Triangle,
    ))
}

fn detect_terminal_cell_geometry() -> (u32, u32, String) {
    if let Some((w, h)) = parse_cell_geometry_override() {
        return (w, h, format!("env:{DEBUG_CELL_ENV}"));
    }

    if let Ok(size) = terminal::window_size()
        && size.columns > 0
        && size.rows > 0
        && size.width > 0
        && size.height > 0
    {
        let cell_w = u32::from(size.width).checked_div(u32::from(size.columns));
        let cell_h = u32::from(size.height).checked_div(u32::from(size.rows));
        if let (Some(cell_w), Some(cell_h)) = (cell_w, cell_h)
            && cell_w > 0
            && cell_h > 0
        {
            return (
                cell_w,
                cell_h,
                format!(
                    "terminal-window-size {}x{} px over {}x{} cells",
                    size.width, size.height, size.columns, size.rows
                ),
            );
        }
    }

    // Conservative fallback: most terminal cells are notably taller than wide.
    (1, 2, "fallback:1x2".to_string())
}

fn parse_cell_geometry_override() -> Option<(u32, u32)> {
    let raw = std::env::var(DEBUG_CELL_ENV).ok()?;
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let (w, h) = raw.split_once('x')?;
    let w = w.trim().parse::<u32>().ok()?;
    let h = h.trim().parse::<u32>().ok()?;
    if w == 0 || h == 0 {
        return None;
    }
    Some((w, h))
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
