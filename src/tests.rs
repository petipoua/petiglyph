use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers, KeyboardEnhancementFlags,
};
use image::imageops::FilterType;
use image::{Rgba, RgbaImage};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::artifact_warning::{INCOMPATIBLE_ARTIFACT_PREFIX, incompatible_artifact_warning};
use crate::build::{
    BuildOptions, GlyphBitmap, MappingEntry, PreprocessedGlyph, bitmap_to_bdf_rows, build_outputs,
    build_outputs_with_options, collect_source_files, coverage_map, glyph_sample_string,
    is_supported_source, preprocess_sources_with_compositions,
};
use crate::cli::{
    DefaultTuiTarget, is_private_use_codepoint, resolve_default_tui_target_for,
    sample_terminal_rendering_hints,
};
use crate::image_pipeline::terminal_cell_width_for_height;
use crate::install::{
    DEFAULT_INSTALL_NAME_MODE, FontInstallNameMode, effective_font_name,
    expected_install_ttf_path_for_mode, reserve_project_unicode_range,
};
use crate::project::{
    CompositionDef, Manifest, RuntimeConfig, auto_detect_manifest_path, discover_project_manifests,
    format_codepoint, load_runtime_config, parse_codepoint, read_manifest, write_manifest,
};
use crate::tui::{
    App, AppView, GlyphsFocus, InstalledFontSample, InteractiveGlyph, TuiLaunchOverrides,
    WelcomeFocus, build_action_name, format_projects_card_hint, format_welcome_input_field,
    handle_key, handle_key_event_for_test, handle_paste_event_for_test, install_action_name,
    installed_font_block_display_lines, installed_font_block_display_lines_with_reference,
    persist_threshold_override, regroup_installed_sample_blocks, render_ui_for_test,
    requested_keyboard_enhancement_flags, resolve_installed_font_path_with,
    sample_glyphs_from_ttf_bytes, should_dispatch_key_kind, switch_notice_visible,
    wrap_sample_for_display,
};

fn make_temp_dir(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time is valid")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("petiglyph-{name}-{nonce}"));
    fs::create_dir_all(&dir).expect("temp dir is created");
    dir
}

fn wait_for_background_tasks(app: &mut App) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while app.background_task_in_progress() && Instant::now() < deadline {
        app.poll_background_tasks_for_test();
        std::thread::sleep(Duration::from_millis(10));
    }
    app.poll_background_tasks_for_test();
    assert!(
        !app.background_task_in_progress(),
        "background tasks did not finish before timeout; status={:?}",
        app.status
    );
}

fn write_test_png(path: &Path) {
    let mut img = RgbaImage::from_pixel(8, 8, Rgba([255, 255, 255, 0]));
    img.put_pixel(2, 2, Rgba([0, 0, 0, 255]));
    img.put_pixel(5, 5, Rgba([0, 0, 0, 255]));
    img.save(path).expect("test image is written");
}

fn write_transparent_rect_png(path: &Path, rect_width: u32, rect_height: u32) {
    let mut img = RgbaImage::from_pixel(rect_width + 2, rect_height + 2, Rgba([255, 255, 255, 0]));
    for y in 1..=rect_height {
        for x in 1..=rect_width {
            img.put_pixel(x, y, Rgba([0, 0, 0, 255]));
        }
    }

    img.save(path).expect("test image is written");
}

fn write_quadrant_png(path: &Path, size: u32) {
    let mut img = RgbaImage::from_pixel(size, size, Rgba([255, 255, 255, 0]));
    let half = size / 2;
    for y in 0..size {
        for x in 0..size {
            let on = (x < half && y < half) || (x >= half && y >= half);
            if on {
                img.put_pixel(x, y, Rgba([0, 0, 0, 255]));
            }
        }
    }
    img.save(path).expect("quadrant test image is written");
}

fn write_split_edge_dots_png(path: &Path, tile_size: u32) {
    let width = tile_size.saturating_mul(2).max(2);
    let height = tile_size.max(2);
    let mid_y = height / 2;

    let mut img = RgbaImage::from_pixel(width, height, Rgba([255, 255, 255, 0]));
    img.put_pixel(0, mid_y, Rgba([0, 0, 0, 255]));
    img.put_pixel(width.saturating_sub(1), mid_y, Rgba([0, 0, 0, 255]));
    img.save(path).expect("split-edge test image is written");
}

fn nonzero_coverage_bounds(coverage: &[u8], size: u32) -> Option<(u32, u32, u32, u32)> {
    nonzero_coverage_bounds_rect(coverage, size, size)
}

fn nonzero_coverage_bounds_rect(
    coverage: &[u8],
    width: u32,
    height: u32,
) -> Option<(u32, u32, u32, u32)> {
    if width == 0 || height == 0 {
        return None;
    }

    let mut min_x = width;
    let mut min_y = height;
    let mut max_x = 0u32;
    let mut max_y = 0u32;
    let mut found = false;

    for (idx, value) in coverage.iter().enumerate() {
        if *value == 0 {
            continue;
        }

        found = true;
        let x = (idx as u32) % width;
        let y = (idx as u32) / width;
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }

    found.then_some((min_x, min_y, max_x, max_y))
}

fn crop_expected_coverage_tile(
    coverage: &[u8],
    grid_width: u32,
    x0: u32,
    y0: u32,
    tile_width: u32,
    tile_height: u32,
) -> Vec<u8> {
    let grid_width = grid_width as usize;
    let x0 = x0 as usize;
    let y0 = y0 as usize;
    let tile_width = tile_width as usize;
    let tile_height = tile_height as usize;
    let mut out = Vec::with_capacity(tile_width * tile_height);

    for y in 0..tile_height {
        let start = (y0 + y) * grid_width + x0;
        out.extend_from_slice(&coverage[start..start + tile_width]);
    }

    out
}

fn fit_alpha_to_canvas(source: &RgbaImage, target_w: u32, target_h: u32) -> Vec<u8> {
    let mut min_x = source.width();
    let mut min_y = source.height();
    let mut max_x = 0u32;
    let mut max_y = 0u32;
    let mut found = false;

    for y in 0..source.height() {
        for x in 0..source.width() {
            if source.get_pixel(x, y)[3] == 0 {
                continue;
            }
            found = true;
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    }

    if !found {
        return vec![0; (target_w as usize) * (target_h as usize)];
    }

    let crop_w = max_x - min_x + 1;
    let crop_h = max_y - min_y + 1;
    let cropped = image::imageops::crop_imm(source, min_x, min_y, crop_w, crop_h).to_image();
    let cropped_alpha = image::GrayImage::from_fn(crop_w, crop_h, |x, y| {
        image::Luma([cropped.get_pixel(x, y)[3]])
    });

    let scale = (target_w as f64 / crop_w as f64).min(target_h as f64 / crop_h as f64);
    let scaled_w = ((crop_w as f64 * scale).round() as u32).clamp(1, target_w);
    let scaled_h = ((crop_h as f64 * scale).round() as u32).clamp(1, target_h);
    let resized = image::imageops::resize(&cropped_alpha, scaled_w, scaled_h, FilterType::Lanczos3);

    let mut canvas = image::GrayImage::from_pixel(target_w, target_h, image::Luma([0]));
    let offset_x = ((target_w - scaled_w) / 2) as i64;
    let offset_y = ((target_h - scaled_h) / 2) as i64;
    image::imageops::overlay(&mut canvas, &resized, offset_x, offset_y);
    canvas.into_raw()
}

#[test]
fn parse_codepoint_accepts_common_formats() {
    assert_eq!(parse_codepoint("U+E000").expect("parse U+"), 0xE000);
    assert_eq!(
        parse_codepoint("U+100000").expect("parse supplementary plane"),
        0x10_0000
    );
    assert_eq!(parse_codepoint("0x41").expect("parse hex"), 0x41);
    assert_eq!(
        parse_codepoint("10ffff").expect("parse bare hex"),
        0x10_FFFF
    );
    assert!(parse_codepoint("D800").is_err());
}

#[test]
fn parse_codepoint_rejects_empty_and_out_of_range_values() {
    assert!(parse_codepoint("").is_err());
    assert!(parse_codepoint(" ").is_err());
    assert!(parse_codepoint("110000").is_err());
}

#[test]
fn incompatible_artifact_warning_flags_legacy_lock_mismatch() {
    let warning = incompatible_artifact_warning(
        "glyph lock project_id mismatch in /tmp/x/petiglyph.lock",
        Some(Path::new("/tmp/x/petiglyph.toml")),
    )
    .expect("legacy lock mismatch should be flagged");
    assert!(
        warning.starts_with(INCOMPATIBLE_ARTIFACT_PREFIX),
        "warning prefix should be explicit: {warning}"
    );
    assert!(
        warning.contains("doctor --repair"),
        "warning should include repair command: {warning}"
    );
}

#[test]
fn incompatible_artifact_warning_ignores_standard_runtime_errors() {
    let warning = incompatible_artifact_warning(
        "no source images found in /tmp/x/icons",
        Some(Path::new("/tmp/x/petiglyph.toml")),
    );
    assert!(
        warning.is_none(),
        "normal runtime errors should not be reclassified as incompatible artifacts"
    );
}

#[test]
fn effective_font_name_uses_project_prefix_in_project_mode() {
    let project_dir = make_temp_dir("effective-font-name");
    let manifest_path = project_dir.join("petiglyph.toml");

    let plain = effective_font_name(&manifest_path, "My Font", FontInstallNameMode::Plain)
        .expect("plain naming resolves");
    let prefixed = effective_font_name(
        &manifest_path,
        "My Font",
        FontInstallNameMode::ProjectPrefixed,
    )
    .expect("prefixed naming resolves");

    assert_eq!(plain, "My Font");
    assert!(
        prefixed.ends_with("-My Font"),
        "project prefix should be prepended to user font name"
    );
    assert_ne!(prefixed, plain);

    fs::remove_dir_all(project_dir).expect("temp project dir is removed");
}

#[test]
fn effective_font_name_avoids_duplicate_project_slug_prefix() {
    let project_dir = make_temp_dir("dup-font");
    let manifest_path = project_dir.join("petiglyph.toml");
    let project_slug = project_dir
        .file_name()
        .and_then(|name| name.to_str())
        .expect("temp project dir name");

    let prefixed = effective_font_name(
        &manifest_path,
        project_slug,
        FontInstallNameMode::ProjectPrefixed,
    )
    .expect("project naming resolves");

    assert_eq!(
        prefixed, project_slug,
        "project-prefixed mode should collapse redundant <project>-<font> names when slugs match"
    );

    fs::remove_dir_all(project_dir).expect("temp project dir is removed");
}

#[test]
fn resolve_installed_font_path_detects_cli_project_prefixed_name() {
    let project_dir = make_temp_dir("installed-path-fallback");
    let manifest_path = project_dir.join("petiglyph.toml");

    let plain_path =
        expected_install_ttf_path_for_mode(&manifest_path, "My Font", FontInstallNameMode::Plain)
            .expect("plain install path resolves");
    let prefixed_path = expected_install_ttf_path_for_mode(
        &manifest_path,
        "My Font",
        FontInstallNameMode::ProjectPrefixed,
    )
    .expect("project-prefixed install path resolves");

    let resolved = resolve_installed_font_path_with(&manifest_path, "My Font", None, |path| {
        path == prefixed_path.as_path()
    });

    assert_ne!(plain_path, prefixed_path);
    assert_eq!(resolved.as_deref(), Some(prefixed_path.as_path()));

    fs::remove_dir_all(project_dir).expect("temp project dir is removed");
}

#[test]
fn resolve_installed_font_path_prefers_project_scoped_candidate_over_legacy_plain() {
    let project_dir = make_temp_dir("installed-path-prefers-default");
    let manifest_path = project_dir.join("petiglyph.toml");

    let plain_path =
        expected_install_ttf_path_for_mode(&manifest_path, "My Font", FontInstallNameMode::Plain)
            .expect("plain install path resolves");
    let project_scoped_path =
        expected_install_ttf_path_for_mode(&manifest_path, "My Font", DEFAULT_INSTALL_NAME_MODE)
            .expect("project-scoped install path resolves");

    let resolved = resolve_installed_font_path_with(&manifest_path, "My Font", None, |path| {
        path == plain_path.as_path() || path == project_scoped_path.as_path()
    });

    assert_eq!(resolved.as_deref(), Some(project_scoped_path.as_path()));

    fs::remove_dir_all(project_dir).expect("temp project dir is removed");
}

#[test]
fn auto_detect_manifest_path_finds_single_child_manifest() {
    let root_dir = make_temp_dir("manifest-child-detect");
    let child_dir = root_dir.join("project-a");
    fs::create_dir_all(&child_dir).expect("child dir is created");
    let child_manifest = child_dir.join("petiglyph.toml");
    write_manifest(&child_manifest, &Manifest::default()).expect("manifest is written");

    let detected = auto_detect_manifest_path(&root_dir).expect("manifest should auto-detect");
    assert_eq!(detected, child_manifest);

    fs::remove_dir_all(root_dir).expect("temp root dir is removed");
}

#[test]
fn auto_detect_manifest_path_ignores_deep_manifests_beyond_one_level() {
    let root_dir = make_temp_dir("manifest-depth-limit");
    let deep_dir = root_dir.join("a").join("b");
    fs::create_dir_all(&deep_dir).expect("deep dir is created");
    let deep_manifest = deep_dir.join("petiglyph.toml");
    write_manifest(&deep_manifest, &Manifest::default()).expect("manifest is written");

    let error =
        auto_detect_manifest_path(&root_dir).expect_err("deep manifest should not auto-detect");
    let message = format!("{error:#}");
    assert!(
        message.contains("no petiglyph project detected"),
        "unexpected error: {message}"
    );

    fs::remove_dir_all(root_dir).expect("temp root dir is removed");
}

#[test]
fn discover_project_manifests_scans_current_and_one_level_only() {
    let root_dir = make_temp_dir("discover-manifests");
    let root_manifest = root_dir.join("petiglyph.toml");
    write_manifest(&root_manifest, &Manifest::default()).expect("root manifest is written");

    let child_dir = root_dir.join("child-project");
    fs::create_dir_all(&child_dir).expect("child dir is created");
    let child_manifest = child_dir.join("petiglyph.toml");
    write_manifest(&child_manifest, &Manifest::default()).expect("child manifest is written");

    let deep_dir = root_dir.join("nested").join("too-deep");
    fs::create_dir_all(&deep_dir).expect("deep dir is created");
    let deep_manifest = deep_dir.join("petiglyph.toml");
    write_manifest(&deep_manifest, &Manifest::default()).expect("deep manifest is written");

    let manifests = discover_project_manifests(&root_dir).expect("manifest discovery succeeds");
    assert_eq!(manifests, vec![root_manifest, child_manifest]);

    fs::remove_dir_all(root_dir).expect("temp root dir is removed");
}

#[test]
fn resolve_default_tui_target_prefers_single_project_direct_open() {
    let root_dir = make_temp_dir("default-target-single");
    let project_dir = root_dir.join("demo");
    fs::create_dir_all(&project_dir).expect("project dir is created");
    let manifest_path = project_dir.join("petiglyph.toml");
    write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");

    let target = resolve_default_tui_target_for(&root_dir).expect("target resolves");
    assert_eq!(
        target,
        DefaultTuiTarget {
            workspace_root: root_dir.clone(),
            initial_project: Some(manifest_path),
        }
    );

    fs::remove_dir_all(root_dir).expect("temp root dir is removed");
}

#[test]
fn resolve_default_tui_target_uses_welcome_for_none_or_multiple_projects() {
    let empty_root = make_temp_dir("default-target-none");
    let target = resolve_default_tui_target_for(&empty_root).expect("target resolves");
    assert_eq!(
        target,
        DefaultTuiTarget {
            workspace_root: empty_root.clone(),
            initial_project: None,
        }
    );
    fs::remove_dir_all(&empty_root).expect("temp empty root dir is removed");

    let multi_root = make_temp_dir("default-target-multiple");
    for name in ["p1", "p2"] {
        let project_dir = multi_root.join(name);
        fs::create_dir_all(&project_dir).expect("project dir is created");
        write_manifest(&project_dir.join("petiglyph.toml"), &Manifest::default())
            .expect("manifest is written");
    }
    let multi_target = resolve_default_tui_target_for(&multi_root).expect("target resolves");
    assert_eq!(
        multi_target,
        DefaultTuiTarget {
            workspace_root: multi_root.clone(),
            initial_project: None,
        }
    );

    fs::remove_dir_all(multi_root).expect("temp multi root dir is removed");
}

#[test]
fn unified_tui_zero_projects_starts_without_active_project() {
    let workspace = make_temp_dir("unified-zero-projects");
    let mut app = App::new_workspace(workspace.clone(), None, TuiLaunchOverrides::default())
        .expect("workspace TUI app initializes");
    app.installed_fonts = Vec::new();

    assert_eq!(app.view, AppView::Welcome);
    assert_eq!(app.active_project, None);
    assert_eq!(app.welcome_focus, WelcomeFocus::CreateInput);

    handle_key(&mut app, KeyCode::Down).expect("down should stay on create input");
    assert_eq!(app.welcome_focus, WelcomeFocus::CreateInput);
    handle_key(&mut app, KeyCode::Right)
        .expect("right moves to verbose toggle when no active project");
    assert_eq!(app.welcome_focus, WelcomeFocus::VerbosePathsToggle);
    handle_key(&mut app, KeyCode::Left).expect("left stays on verbose toggle");
    assert_eq!(app.welcome_focus, WelcomeFocus::VerbosePathsToggle);
    handle_key(&mut app, KeyCode::Down)
        .expect("down from verbose toggle goes to create input when no projects");
    // With no projects and no active project, down from VerbosePathsToggle goes to CreateInput
    handle_key(&mut app, KeyCode::Down).expect("down stays on create input");
    assert_eq!(app.welcome_focus, WelcomeFocus::CreateInput);

    fs::remove_dir_all(workspace).expect("temp workspace is removed");
}

#[test]
fn unified_tui_single_project_can_start_active() {
    let workspace = make_temp_dir("unified-single-project");
    let project_dir = workspace.join("demo");
    let icons_dir = project_dir.join("icons");
    fs::create_dir_all(&icons_dir).expect("icons dir is created");
    write_test_png(&icons_dir.join("icon.png"));
    let manifest_path = project_dir.join("petiglyph.toml");
    write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");

    let app = App::new_workspace(
        workspace.clone(),
        Some(manifest_path.clone()),
        TuiLaunchOverrides::default(),
    )
    .expect("workspace TUI app initializes");

    assert_eq!(app.view, AppView::Welcome);
    assert_eq!(app.active_project.as_deref(), Some(manifest_path.as_path()));
    assert_eq!(app.glyphs.len(), 1);

    fs::remove_dir_all(workspace).expect("temp workspace is removed");
}

#[test]
fn unified_tui_multiple_projects_can_be_selected_from_home() {
    let workspace = make_temp_dir("unified-multi-project");
    for name in ["project-a", "project-b"] {
        let project_dir = workspace.join(name);
        let icons_dir = project_dir.join("icons");
        fs::create_dir_all(&icons_dir).expect("icons dir is created");
        write_test_png(&icons_dir.join(format!("{name}.png")));
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");
    }

    let mut app = App::new_workspace(workspace.clone(), None, TuiLaunchOverrides::default())
        .expect("workspace TUI app initializes");
    assert_eq!(app.active_project, None);
    assert_eq!(app.welcome_focus, WelcomeFocus::ProjectList);
    assert_eq!(app.selected_project, 0);

    handle_key(&mut app, KeyCode::Enter).expect("enter opens selected project");
    assert_eq!(
        app.active_project.as_deref(),
        Some(workspace.join("project-a/petiglyph.toml").as_path())
    );
    assert_eq!(app.glyphs.len(), 1);
    assert!(app.switch_notice.is_some());
    let first_notice_debug = format!("{:?}", app.switch_notice);
    assert!(
        first_notice_debug.contains("from_label: \"none\"")
            && first_notice_debug.contains("to_label: \"project-a\""),
        "first project switch notice should use project labels: {first_notice_debug}"
    );

    handle_key(&mut app, KeyCode::Down).expect("down selects next project");
    assert_eq!(app.welcome_focus, WelcomeFocus::ProjectList);
    assert_eq!(app.selected_project, 1);
    handle_key(&mut app, KeyCode::Enter).expect("enter opens next project");
    assert_eq!(
        app.active_project.as_deref(),
        Some(workspace.join("project-b/petiglyph.toml").as_path())
    );
    let second_notice_debug = format!("{:?}", app.switch_notice);
    assert!(
        second_notice_debug.contains("from_label: \"project-a\"")
            && second_notice_debug.contains("to_label: \"project-b\""),
        "second switch notice should keep project-name labels: {second_notice_debug}"
    );

    handle_key(&mut app, KeyCode::Down).expect("down leaves project list after last project");
    assert_eq!(app.welcome_focus, WelcomeFocus::CreateInput);
    handle_key(&mut app, KeyCode::Right).expect("right moves to build button");
    assert_eq!(app.welcome_focus, WelcomeFocus::BuildButton);
    handle_key(&mut app, KeyCode::Right).expect("right moves to install button");
    assert_eq!(app.welcome_focus, WelcomeFocus::InstallButton);
    handle_key(&mut app, KeyCode::Right).expect("right moves to delete project button");
    assert_eq!(app.welcome_focus, WelcomeFocus::DeleteProjectButton);
    handle_key(&mut app, KeyCode::Left).expect("left returns to install button");
    assert_eq!(app.welcome_focus, WelcomeFocus::InstallButton);
    handle_key(&mut app, KeyCode::Up).expect("up from install reaches verbose toggle");
    assert_eq!(app.welcome_focus, WelcomeFocus::VerbosePathsToggle);
    handle_key(&mut app, KeyCode::Down).expect("down from verbose returns to install button");
    assert_eq!(app.welcome_focus, WelcomeFocus::InstallButton);
    handle_key(&mut app, KeyCode::Left).expect("left returns to build");
    assert_eq!(app.welcome_focus, WelcomeFocus::BuildButton);
    handle_key(&mut app, KeyCode::Left).expect("left from build returns to create button");
    assert_eq!(app.welcome_focus, WelcomeFocus::CreateInput);
    handle_key(&mut app, KeyCode::Up).expect("up returns to project list");
    assert_eq!(app.welcome_focus, WelcomeFocus::ProjectList);
    assert_eq!(app.selected_project, 1);
    handle_key(&mut app, KeyCode::Right).expect("right jumps from project list to build");
    assert_eq!(app.welcome_focus, WelcomeFocus::BuildButton);
    handle_key(&mut app, KeyCode::Up).expect("up from build reaches verbose toggle");
    assert_eq!(app.welcome_focus, WelcomeFocus::VerbosePathsToggle);
    handle_key(&mut app, KeyCode::Down).expect("down from verbose returns to install button");
    assert_eq!(app.welcome_focus, WelcomeFocus::InstallButton);
    handle_key(&mut app, KeyCode::Left).expect("left returns to build button");
    assert_eq!(app.welcome_focus, WelcomeFocus::BuildButton);
    handle_key(&mut app, KeyCode::Left).expect("left from build returns to create button");
    assert_eq!(app.welcome_focus, WelcomeFocus::CreateInput);
    handle_key(&mut app, KeyCode::Up).expect("up returns to project list");
    assert_eq!(app.welcome_focus, WelcomeFocus::ProjectList);
    handle_key(&mut app, KeyCode::Down).expect("down from project list reaches create input");
    assert_eq!(app.welcome_focus, WelcomeFocus::CreateInput);
    handle_key(&mut app, KeyCode::Right).expect("right jumps from create input to build");
    assert_eq!(app.welcome_focus, WelcomeFocus::BuildButton);
    handle_key(&mut app, KeyCode::Right).expect("right moves to install");
    assert_eq!(app.welcome_focus, WelcomeFocus::InstallButton);
    handle_key(&mut app, KeyCode::Left).expect("left from install returns to build");
    assert_eq!(app.welcome_focus, WelcomeFocus::BuildButton);
    handle_key(&mut app, KeyCode::Left).expect("left from build returns to create input");
    assert_eq!(app.welcome_focus, WelcomeFocus::CreateInput);
    handle_key(&mut app, KeyCode::Left).expect("left stays on create input");
    assert_eq!(app.welcome_focus, WelcomeFocus::CreateInput);
    handle_key(&mut app, KeyCode::Right).expect("right moves to build");
    assert_eq!(app.welcome_focus, WelcomeFocus::BuildButton);
    handle_key(&mut app, KeyCode::Left).expect("left from build returns to create input");
    assert_eq!(app.welcome_focus, WelcomeFocus::CreateInput);
    handle_key(&mut app, KeyCode::Right).expect("right moves to build button");
    assert_eq!(app.welcome_focus, WelcomeFocus::BuildButton);
    handle_key(&mut app, KeyCode::Left).expect("left from build returns to create input");
    assert_eq!(app.welcome_focus, WelcomeFocus::CreateInput);

    fs::remove_dir_all(workspace).expect("temp workspace is removed");
}

#[test]
fn welcome_input_edit_mode_types_hjkl_without_navigation() {
    let workspace = make_temp_dir("welcome-input-edit-mode");
    let mut app = App::new_workspace(workspace.clone(), None, TuiLaunchOverrides::default())
        .expect("workspace TUI app initializes");
    app.installed_fonts.clear();
    app.selected_installed_font = 0;

    assert_eq!(app.view, AppView::Welcome);
    assert_eq!(app.welcome_focus, WelcomeFocus::CreateInput);
    assert!(!app.welcome_input_editing);

    handle_key(&mut app, KeyCode::Enter).expect("enter starts editing on create input");
    assert!(app.welcome_input_editing);
    assert_eq!(app.welcome_focus, WelcomeFocus::CreateInput);

    for ch in ['l', 'h', 'j', 'k', '2', '3'] {
        handle_key(&mut app, KeyCode::Char(ch)).expect("typing should append character");
    }
    assert_eq!(app.create_input.value(), "lhjk23");
    assert_eq!(app.welcome_focus, WelcomeFocus::CreateInput);

    // clear the input so Enter won't submit_create, just cancels editing
    app.create_input = app.create_input.clone().with_value(String::new());
    handle_key(&mut app, KeyCode::Enter)
        .expect("enter exits typing mode without creating when input is empty");
    assert!(!app.welcome_input_editing);
    assert_eq!(app.welcome_focus, WelcomeFocus::CreateInput);

    fs::remove_dir_all(workspace).expect("temp workspace is removed");
}

#[test]
fn welcome_input_edit_mode_esc_exits_typing_without_quit() {
    let workspace = make_temp_dir("welcome-input-edit-esc");
    let mut app = App::new_workspace(workspace.clone(), None, TuiLaunchOverrides::default())
        .expect("workspace TUI app initializes");

    handle_key(&mut app, KeyCode::Enter).expect("enter starts typing mode");
    assert!(app.welcome_input_editing);

    handle_key(&mut app, KeyCode::Esc).expect("esc exits typing mode");
    assert!(!app.welcome_input_editing);
    assert!(!app.quit);
    assert_eq!(app.welcome_focus, WelcomeFocus::CreateInput);

    fs::remove_dir_all(workspace).expect("temp workspace is removed");
}

#[test]
fn project_actions_without_active_project_set_status_and_do_not_crash() {
    let workspace = make_temp_dir("no-active-actions");
    let mut app = App::new_workspace(workspace.clone(), None, TuiLaunchOverrides::default())
        .expect("workspace TUI app initializes");

    handle_key(&mut app, KeyCode::Char('b')).expect("build guard succeeds");
    assert!(
        app.status
            .as_deref()
            .is_some_and(|status| status.contains("before building"))
    );

    handle_key(&mut app, KeyCode::Char('i')).expect("install guard succeeds");
    assert!(
        app.status
            .as_deref()
            .is_some_and(|status| status.contains("before installing"))
    );

    app.view = AppView::Glyphs;
    handle_key(&mut app, KeyCode::Right).expect("threshold guard succeeds");
    assert!(
        app.status
            .as_deref()
            .is_some_and(|status| status.contains("before tuning glyphs"))
    );

    fs::remove_dir_all(workspace).expect("temp workspace is removed");
}

#[test]
fn switch_notice_visibility_expires_deterministically() {
    let started_at = Instant::now();
    assert!(switch_notice_visible(
        started_at,
        started_at + Duration::from_millis(100)
    ));
    assert!(!switch_notice_visible(
        started_at,
        started_at + Duration::from_millis(2500)
    ));
}

#[test]
fn glyph_sample_string_skips_non_scalar_values() {
    let sample = glyph_sample_string(0xDFFF, 2);
    let expected = char::from_u32(0xE000).expect("valid").to_string();
    assert_eq!(sample, expected);
}

#[test]
fn wrap_sample_for_display_respects_chunk_size() {
    let wrapped = wrap_sample_for_display("ABCDE", 2);
    assert_eq!(wrapped, vec!["AB", "CD", "E"]);
}

#[test]
fn wrap_sample_for_display_preserves_multiline_grid_layout() {
    let wrapped = wrap_sample_for_display("ABCD\nEF\n\nGH", 2);
    assert_eq!(wrapped, vec!["AB", "CD", "EF", "", "GH"]);
}

#[test]
fn private_use_codepoint_detection_matches_unicode_pua_ranges() {
    assert!(is_private_use_codepoint(0xE000));
    assert!(is_private_use_codepoint(0xF8FF));
    assert!(is_private_use_codepoint(0xF0000));
    assert!(is_private_use_codepoint(0x10FFFD));
    assert!(!is_private_use_codepoint(0xD7FF));
    assert!(!is_private_use_codepoint(0x110000));
}

#[test]
fn sample_terminal_rendering_hints_warn_for_pua_and_multiline_grids() {
    let sample = "\u{100000}\u{100001}\n\u{100002}\u{100003}";
    let hints = sample_terminal_rendering_hints(sample);

    assert!(
        hints
            .iter()
            .any(|hint| hint.contains("Private Use codepoints")),
        "expected a private-use width warning: {hints:?}"
    );
    assert!(
        hints
            .iter()
            .any(|hint| hint.contains("line-height/cell-height")),
        "expected a multiline grid spacing warning: {hints:?}"
    );
}

#[test]
fn installed_font_card_keeps_grid_tiles_adjacent() {
    let lines = installed_font_block_display_lines("ABC\nDEF\nGHI", 80);
    assert_eq!(
        lines,
        vec!["ABC", "DEF", "GHI"],
        "installed font card must not inject spaces between composition tiles"
    );
}

#[test]
fn installed_font_card_adds_block_reference_for_multiline_grids() {
    let lines = installed_font_block_display_lines_with_reference("AB\n C", 20);
    assert_eq!(
        lines,
        vec!["AB       │  ██", " C       │   █"],
        "multiline grids should include a full-block reference column"
    );
}

#[test]
fn installed_font_card_reference_helper_keeps_single_line_blocks_unchanged() {
    let lines = installed_font_block_display_lines_with_reference("ABCD", 20);
    assert_eq!(lines, vec!["ABCD"]);
}

#[test]
fn regroup_installed_sample_blocks_merges_unitary_and_keeps_grids_ordered() {
    let grouped = regroup_installed_sample_blocks(vec![
        "A B".to_string(),
        "C D".to_string(),
        "XY\nZW".to_string(),
        "E F".to_string(),
        "12\n34".to_string(),
    ]);
    assert_eq!(grouped, vec!["A B C D E F", "XY\nZW", "12\n34"]);
}

#[test]
fn regroup_installed_sample_blocks_filters_empty_and_whitespace_blocks() {
    let grouped = regroup_installed_sample_blocks(vec![
        "   ".to_string(),
        "\n\n".to_string(),
        "A B".to_string(),
        " \nC\nD\n ".to_string(),
    ]);
    assert_eq!(grouped, vec!["A B", "C\nD"]);
}

#[test]
fn welcome_input_field_keeps_fixed_width_in_all_focus_states() {
    let empty_blur = format_welcome_input_field("", false, 15);
    let empty_focus = format_welcome_input_field("", true, 15);
    let typed_blur = format_welcome_input_field("demo-font", false, 15);
    let typed_focus = format_welcome_input_field("demo-font", true, 15);

    assert_eq!(empty_blur.chars().count(), empty_focus.chars().count());
    assert_eq!(empty_blur.chars().count(), typed_blur.chars().count());
    assert_eq!(typed_blur.chars().count(), typed_focus.chars().count());
    assert_eq!(empty_blur.chars().count(), 17);
}

#[test]
fn projects_card_hint_keeps_fixed_width_and_stays_local() {
    let input_hint = format_projects_card_hint(WelcomeFocus::CreateInput, false);
    let typing_hint = format_projects_card_hint(WelcomeFocus::CreateInput, true);
    let button_hint = format_projects_card_hint(WelcomeFocus::CreateInput, false);
    let build_hint = format_projects_card_hint(WelcomeFocus::BuildButton, false);
    let install_hint = format_projects_card_hint(WelcomeFocus::InstallButton, false);
    let delete_hint = format_projects_card_hint(WelcomeFocus::DeleteProjectButton, false);
    let uninstall_hint = format_projects_card_hint(WelcomeFocus::InstalledFontList, false);
    let list_hint = format_projects_card_hint(WelcomeFocus::ProjectList, false);

    assert_eq!(input_hint.chars().count(), typing_hint.chars().count());
    assert_eq!(input_hint.chars().count(), button_hint.chars().count());
    assert_eq!(input_hint.chars().count(), build_hint.chars().count());
    assert_eq!(input_hint.chars().count(), install_hint.chars().count());
    assert_eq!(input_hint.chars().count(), delete_hint.chars().count());
    assert_eq!(input_hint.chars().count(), uninstall_hint.chars().count());
    assert_eq!(input_hint.chars().count(), list_hint.chars().count());
    assert!(input_hint.contains("press Enter to create"));
    assert!(typing_hint.contains("typing (Enter/Esc to stop)"));
    assert!(button_hint.contains("press Enter to create"));
    assert!(list_hint.trim().is_empty());

    assert!(install_hint.trim().is_empty());
    assert!(delete_hint.trim().is_empty());
    assert!(uninstall_hint.trim().is_empty());
}

#[test]
fn delete_project_flow_requires_arrow_hops_to_reach_delete() {
    let workspace = make_temp_dir("home-delete-confirm");
    let project_dir = workspace.join("delete-me");
    let icons_dir = project_dir.join("icons");
    fs::create_dir_all(&icons_dir).expect("icons dir is created");
    write_test_png(&icons_dir.join("icon.png"));
    let manifest_path = project_dir.join("petiglyph.toml");
    write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");

    let mut app = App::new_workspace(workspace.clone(), None, TuiLaunchOverrides::default())
        .expect("workspace TUI app initializes");
    handle_key(&mut app, KeyCode::Enter).expect("open project");
    assert_eq!(app.active_project.as_deref(), Some(manifest_path.as_path()));

    app.welcome_focus = WelcomeFocus::DeleteProjectButton;
    handle_key(&mut app, KeyCode::Enter).expect("open delete confirmation");
    assert!(
        app.status.is_none(),
        "opening confirm should not set status, got {:?}",
        app.status
    );

    handle_key(&mut app, KeyCode::Enter).expect("enter on cancel should cancel");
    assert!(project_dir.exists());
    assert!(
        app.status
            .as_deref()
            .is_some_and(|status| status.contains("canceled"))
    );

    app.welcome_focus = WelcomeFocus::DeleteProjectButton;
    handle_key(&mut app, KeyCode::Enter).expect("reopen delete confirmation for hop validation");
    assert!(app.status.is_none());
    handle_key(&mut app, KeyCode::Right).expect("move to first hop");
    handle_key(&mut app, KeyCode::Right).expect("right should not bypass turn");
    handle_key(&mut app, KeyCode::Enter).expect("enter on first hop should be blocked");
    assert!(project_dir.exists());
    assert!(
        app.status
            .as_deref()
            .is_some_and(|status| status.contains("path with turns"))
    );

    let hop_path = [KeyCode::Down, KeyCode::Right, KeyCode::Right];
    for key in hop_path {
        handle_key(&mut app, key).expect("hop movement should work");
    }

    handle_key(&mut app, KeyCode::Enter).expect("confirm deletion at delete button");
    assert!(!project_dir.exists());
    assert_eq!(app.active_project, None);
    assert_eq!(app.welcome_focus, WelcomeFocus::CreateInput);
    assert!(
        app.status
            .as_deref()
            .is_some_and(|status| status.contains("deleted project"))
    );

    fs::remove_dir_all(workspace).expect("temp workspace is removed");
}

#[test]
fn delete_project_confirmation_can_be_canceled() {
    let workspace = make_temp_dir("home-delete-cancel");
    let project_dir = workspace.join("keep-me");
    let icons_dir = project_dir.join("icons");
    fs::create_dir_all(&icons_dir).expect("icons dir is created");
    write_test_png(&icons_dir.join("icon.png"));
    let manifest_path = project_dir.join("petiglyph.toml");
    write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");

    let mut app = App::new_workspace(workspace.clone(), None, TuiLaunchOverrides::default())
        .expect("workspace TUI app initializes");
    handle_key(&mut app, KeyCode::Enter).expect("open project");
    app.welcome_focus = WelcomeFocus::DeleteProjectButton;
    handle_key(&mut app, KeyCode::Enter).expect("open confirmation");
    handle_key(&mut app, KeyCode::Esc).expect("cancel confirmation");

    assert!(project_dir.exists());
    assert_eq!(app.active_project.as_deref(), Some(manifest_path.as_path()));
    assert!(
        app.status
            .as_deref()
            .is_some_and(|status| status.contains("canceled"))
    );

    fs::remove_dir_all(workspace).expect("temp workspace is removed");
}

#[test]
fn project_action_labels_switch_with_current_project_state() {
    assert_eq!(build_action_name(false), "Build");
    assert_eq!(build_action_name(true), "Rebuild");
    assert_eq!(install_action_name(false), "Install");
    assert_eq!(install_action_name(true), "Reinstall");
}

#[test]
fn glyph_view_animate_button_runs_placeholder_action() {
    let project_dir = make_temp_dir("animate-glyph-enter");
    let manifest_path = project_dir.join("petiglyph.toml");
    let icons_dir = project_dir.join("icons");
    fs::create_dir_all(&icons_dir).expect("icons dir is created");
    write_test_png(&icons_dir.join("icon.png"));

    let config = RuntimeConfig {
        project_dir: project_dir.clone(),
        project_id: "test-animate-glyph-enter".to_string(),
        input_dir: icons_dir,
        out_dir: project_dir.join("build"),
        font_name: "Petiglyph".to_string(),
        glyph_size: 8,
        base_threshold: 64,
        threshold_overrides: BTreeMap::new(),
        compositions: BTreeMap::new(),
        codepoint_start: 0x10_0000,
    };

    let mut app = App::new(manifest_path, config);
    app.view = AppView::Glyphs;
    app.glyphs_focus = GlyphsFocus::AnimateButton;

    handle_key(&mut app, KeyCode::Enter).expect("enter on animate glyph should succeed");
    assert!(
        app.status
            .as_deref()
            .is_some_and(|status| status.contains("Animate Glyph is planned for Glyphs tools"))
    );
    assert_eq!(app.view, AppView::Glyphs);

    fs::remove_dir_all(project_dir).expect("temp project dir is removed");
}

#[test]
fn glyph_view_c_shortcuts_create_and_remove_composition_in_manifest() {
    let project_dir = make_temp_dir("glyph-compose-shortcuts");
    let manifest_path = project_dir.join("petiglyph.toml");
    write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");

    let icons_dir = project_dir.join("icons");
    fs::create_dir_all(&icons_dir).expect("icons dir is created");
    write_test_png(&icons_dir.join("icon.png"));

    let mut app = App::new_workspace(
        project_dir.clone(),
        Some(manifest_path.clone()),
        TuiLaunchOverrides::default(),
    )
    .expect("workspace app initializes");
    app.view = AppView::Glyphs;

    handle_key(&mut app, KeyCode::Char('c')).expect("create composition shortcut should work");
    let manifest = read_manifest(&manifest_path).expect("manifest reloads");
    assert_eq!(
        manifest.compositions.get("icon.png"),
        Some(&CompositionDef { rows: 2, cols: 2 })
    );
    assert!(
        app.status
            .as_deref()
            .is_some_and(|status| status.contains("created composition"))
    );

    handle_key(&mut app, KeyCode::Char('C')).expect("remove composition shortcut should work");
    let manifest = read_manifest(&manifest_path).expect("manifest reloads");
    assert!(
        !manifest.compositions.contains_key("icon.png"),
        "composition should be removed from manifest"
    );
    assert!(
        app.status
            .as_deref()
            .is_some_and(|status| status.contains("removed composition"))
    );

    fs::remove_dir_all(project_dir).expect("temp project dir is removed");
}

#[test]
fn home_installed_font_buttons_can_be_navigated() {
    let project_dir = make_temp_dir("home-installed-font-nav");
    let manifest_path = project_dir.join("petiglyph.toml");
    write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");

    let icons_dir = project_dir.join("icons");
    fs::create_dir_all(&icons_dir).expect("icons dir is created");
    write_test_png(&icons_dir.join("icon.png"));

    let config = RuntimeConfig {
        project_dir: project_dir.clone(),
        project_id: "test-home-installed-font-nav".to_string(),
        input_dir: icons_dir,
        out_dir: project_dir.join("build"),
        font_name: "Petiglyph".to_string(),
        glyph_size: 8,
        base_threshold: 64,
        threshold_overrides: BTreeMap::new(),
        compositions: BTreeMap::new(),
        codepoint_start: 0x10_0000,
    };

    let mut app = App::new(manifest_path, config);
    app.installed_fonts = vec![
        InstalledFontSample {
            file_name: "alpha.ttf".to_string(),
            path: PathBuf::from("/tmp/alpha.ttf"),
            blocks: vec!["AB".to_string()],
            truncated: false,
        },
        InstalledFontSample {
            file_name: "beta.ttf".to_string(),
            path: PathBuf::from("/tmp/beta.ttf"),
            blocks: vec!["CD".to_string()],
            truncated: false,
        },
    ];

    app.welcome_focus = WelcomeFocus::BuildButton;
    handle_key(&mut app, KeyCode::Down).expect("down should move to installed fonts");
    assert_eq!(app.welcome_focus, WelcomeFocus::InstalledFontList);
    assert_eq!(app.selected_installed_font, 0);
    assert_eq!(app.selected_installed_font_sub_index, 0);

    handle_key(&mut app, KeyCode::Down).expect("down should move to sample line");
    assert_eq!(app.welcome_focus, WelcomeFocus::InstalledFontList);
    assert_eq!(app.selected_installed_font, 0);
    assert_eq!(app.selected_installed_font_sub_index, 1);

    handle_key(&mut app, KeyCode::Down).expect("down should move to next installed font");
    assert_eq!(app.welcome_focus, WelcomeFocus::InstalledFontList);
    assert_eq!(app.selected_installed_font, 1);
    assert_eq!(app.selected_installed_font_sub_index, 0);

    handle_key(&mut app, KeyCode::Up).expect("up should move to previous font's sample");
    assert_eq!(app.welcome_focus, WelcomeFocus::InstalledFontList);
    assert_eq!(app.selected_installed_font, 0);
    assert_eq!(app.selected_installed_font_sub_index, 1);

    handle_key(&mut app, KeyCode::Up).expect("up should move to previous font's title");
    assert_eq!(app.welcome_focus, WelcomeFocus::InstalledFontList);
    assert_eq!(app.selected_installed_font, 0);
    assert_eq!(app.selected_installed_font_sub_index, 0);

    handle_key(&mut app, KeyCode::Up).expect("up from first installed font returns to build");
    assert_eq!(app.welcome_focus, WelcomeFocus::BuildButton);

    handle_key(&mut app, KeyCode::Right).expect("right moves to install");
    assert_eq!(app.welcome_focus, WelcomeFocus::InstallButton);
    handle_key(&mut app, KeyCode::Right)
        .expect("right from install stays put when delete is unavailable");
    assert_eq!(app.welcome_focus, WelcomeFocus::InstallButton);

    fs::remove_dir_all(project_dir).expect("temp project dir is removed");
}

#[test]
fn home_view_renders_without_panicking() {
    let project_dir = make_temp_dir("home-render");
    let manifest_path = project_dir.join("petiglyph.toml");
    write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");

    let icons_dir = project_dir.join("icons");
    fs::create_dir_all(&icons_dir).expect("icons dir is created");
    write_test_png(&icons_dir.join("icon.png"));

    let config = RuntimeConfig {
        project_dir: project_dir.clone(),
        project_id: "test-render-home".to_string(),
        input_dir: icons_dir,
        out_dir: project_dir.join("build"),
        font_name: "Petiglyph".to_string(),
        glyph_size: 8,
        base_threshold: 64,
        threshold_overrides: BTreeMap::new(),
        compositions: BTreeMap::new(),
        codepoint_start: 0x10_0000,
    };

    let app = App::new(manifest_path, config);
    render_ui_for_test(&app, 148, 46).expect("home view should render");

    fs::remove_dir_all(project_dir).expect("temp project dir is removed");
}

#[test]
fn coverage_map_uses_alpha_for_transparent_sources() {
    let mut image = RgbaImage::from_pixel(2, 2, Rgba([255, 255, 255, 0]));
    image.put_pixel(0, 0, Rgba([255, 255, 255, 255]));
    image.put_pixel(1, 1, Rgba([255, 255, 255, 128]));

    let coverage = coverage_map(&image, 2).expect("coverage map succeeds");

    assert_eq!(coverage[0], 255);
    assert_eq!(coverage[1], 0);
    assert_eq!(coverage[2], 0);
    assert_eq!(coverage[3], 128);
}

#[test]
fn coverage_map_detects_foreground_on_opaque_background() {
    let mut image = RgbaImage::from_pixel(3, 3, Rgba([255, 255, 255, 255]));
    for i in 0..3 {
        image.put_pixel(1, i, Rgba([0, 0, 0, 255]));
        image.put_pixel(i, 1, Rgba([0, 0, 0, 255]));
    }

    let coverage = coverage_map(&image, 3).expect("coverage map succeeds");

    assert_eq!(coverage[0], 0);
    assert_eq!(coverage[4], 255);
    assert_eq!(coverage[8], 0);
}

#[test]
fn coverage_map_recenters_transparent_content_after_trimming() {
    let mut image = RgbaImage::from_pixel(10, 10, Rgba([255, 255, 255, 0]));
    for y in 1..=2 {
        for x in 1..=7 {
            image.put_pixel(x, y, Rgba([0, 0, 0, 255]));
        }
    }

    let coverage = coverage_map(&image, 10).expect("coverage map succeeds");
    let (min_x, min_y, max_x, max_y) =
        nonzero_coverage_bounds(&coverage, 10).expect("coverage should contain non-zero pixels");
    let top = min_y;
    let bottom = 9 - max_y;
    let width = max_x - min_x + 1;

    assert!(
        width >= 9,
        "trimmed content should nearly fill width, got {width}"
    );
    assert!(
        (top as i32 - bottom as i32).abs() <= 1,
        "vertical margins should be centered: top={top}, bottom={bottom}"
    );
}

#[test]
fn coverage_map_recenters_opaque_content_after_trimming() {
    let mut image = RgbaImage::from_pixel(10, 10, Rgba([255, 255, 255, 255]));
    for y in 2..=9 {
        for x in 7..=8 {
            image.put_pixel(x, y, Rgba([0, 0, 0, 255]));
        }
    }

    let coverage = coverage_map(&image, 10).expect("coverage map succeeds");
    let (min_x, min_y, max_x, max_y) =
        nonzero_coverage_bounds(&coverage, 10).expect("coverage should contain non-zero pixels");
    let left = min_x;
    let right = 9 - max_x;
    let height = max_y - min_y + 1;

    assert!(
        height >= 9,
        "trimmed content should nearly fill height, got {height}"
    );
    assert!(
        (left as i32 - right as i32).abs() <= 1,
        "horizontal margins should be centered: left={left}, right={right}"
    );
}

#[test]
fn bitmap_to_bdf_rows_packs_pixels_into_hex_rows() {
    let bitmap = GlyphBitmap {
        width: 8,
        height: 8,
        pixels: vec![
            true, false, true, false, false, false, false, true, false, true, false, true, false,
            false, false, false, false, false, false, false, true, true, false, false, false,
            false, false, false, false, false, false, false, true, true, true, true, true, true,
            true, true, false, false, false, false, false, false, false, false, false, false,
            false, false, false, false, false, false, true, false, false, false, false, false,
            false, false,
        ],
    };

    assert_eq!(
        bitmap_to_bdf_rows(&bitmap),
        "A1\n50\n0C\n00\nFF\n00\n00\n80\n"
    );
}

#[test]
fn supported_source_extensions_include_avif() {
    assert!(is_supported_source(Path::new("icon.avif")));
    assert!(is_supported_source(Path::new("ICON.AVIF")));
    assert!(!is_supported_source(Path::new("icon.tiff")));
}

#[test]
fn build_outputs_generates_non_empty_repo_icon_font() {
    let out_dir = make_temp_dir("icons-e2e");
    let config = RuntimeConfig {
        project_dir: out_dir.clone(),
        project_id: "test-icons-e2e".to_string(),
        input_dir: PathBuf::from("icons"),
        out_dir: out_dir.clone(),
        font_name: "Petiglyph".to_string(),
        glyph_size: 64,
        base_threshold: 64,
        threshold_overrides: BTreeMap::new(),
        compositions: BTreeMap::new(),
        codepoint_start: 0x10_0000,
    };

    let summary = build_outputs(&config).expect("build succeeds");
    let mapping_json = fs::read_to_string(&summary.mapping_path).expect("glyph map is written");
    let mapping: Vec<MappingEntry> = serde_json::from_str(&mapping_json).expect("glyph map parses");
    let bdf = fs::read_to_string(&summary.bdf_path).expect("bdf is written");
    let ttf = fs::read(&summary.ttf_path).expect("ttf is written");
    let sample = fs::read_to_string(&summary.sample_path).expect("sample is written");
    let sources = collect_source_files(Path::new("icons")).expect("icons are readable");
    let face = ttf_parser::Face::parse(&ttf, 0).expect("ttf parses");

    assert_eq!(summary.glyph_count, sources.len());
    assert_eq!(mapping.len(), sources.len());
    assert!(bdf.contains(&format!("CHARS {}", sources.len())));
    assert_eq!(face.number_of_glyphs(), sources.len() as u16 + 2);
    assert_eq!(
        sample.trim_end(),
        glyph_sample_string(config.codepoint_start, sources.len())
    );
    let space_id = face.glyph_index(' ').expect("space glyph should exist");
    assert_eq!(
        face.glyph_hor_advance(space_id),
        Some(face.units_per_em() / 2),
        "space must keep the same one-cell advance as generated glyphs"
    );

    for entry in &mapping {
        assert!(bdf.contains(&format!("STARTCHAR {}", entry.glyph_name)));
        let codepoint = parse_codepoint(&entry.codepoint).expect("codepoint parses");
        assert!(bdf.contains(&format!("ENCODING {}", codepoint)));
        assert!(
            face.glyph_index(char::from_u32(codepoint).expect("bmp codepoint"))
                .is_some(),
            "ttf should map {}",
            entry.glyph_name
        );

        let preview_path = summary
            .previews_dir
            .join(format!("{}.png", entry.glyph_name));
        let preview = image::open(&preview_path)
            .expect("preview opens")
            .to_rgba8();
        assert!(
            preview.pixels().any(|pixel| pixel[3] > 0),
            "preview should contain visible pixels: {}",
            preview_path.display()
        );
    }

    fs::remove_dir_all(out_dir).expect("temp output dir is removed");
}

#[test]
fn build_outputs_composition_writes_grid_sample_and_contiguous_codepoints() {
    let project_dir = make_temp_dir("composition-grid-sample");
    let input_dir = project_dir.join("icons");
    fs::create_dir_all(&input_dir).expect("icons dir is created");
    write_quadrant_png(&input_dir.join("logo.png"), 80);

    let manifest_path = project_dir.join("petiglyph.toml");
    let mut manifest = Manifest::default();
    manifest.project_id = Some("test-composition-grid-sample".to_string());
    manifest
        .compositions
        .insert("logo.png".to_string(), CompositionDef { rows: 2, cols: 2 });
    write_manifest(&manifest_path, &manifest).expect("manifest is written");

    let config = load_runtime_config(&manifest_path, None, None, None, None, None)
        .expect("runtime config loads");
    let summary = build_outputs(&config).expect("build succeeds");

    let mapping: Vec<MappingEntry> = serde_json::from_str(
        &fs::read_to_string(&summary.mapping_path).expect("mapping is written"),
    )
    .expect("mapping parses");
    assert_eq!(
        mapping.len(),
        8,
        "2x2 composition should emit 8 one-cell glyph tiles (L/R split)"
    );

    let codepoints = mapping
        .iter()
        .map(|entry| parse_codepoint(&entry.codepoint).expect("codepoint parses"))
        .collect::<Vec<_>>();
    for pair in codepoints.windows(2) {
        assert_eq!(pair[1], pair[0] + 1, "composition tiles must be contiguous");
    }

    let sample = fs::read_to_string(summary.sample_path).expect("sample is written");
    let sample = sample.trim_end();
    let lines = sample.lines().collect::<Vec<_>>();
    assert_eq!(
        lines.len(),
        2,
        "sample should preserve composition row layout"
    );
    assert_eq!(lines[0].chars().count(), 4);
    assert_eq!(lines[1].chars().count(), 4);
    assert!(
        !sample.contains(' '),
        "composed sample should be a compact grid without spaces"
    );

    fs::remove_dir_all(project_dir).expect("temp project dir is removed");
}

#[test]
fn preprocess_composition_tiles_keep_corner_alignment_without_recentering() {
    let project_dir = make_temp_dir("composition-corner-alignment");
    let input_dir = project_dir.join("icons");
    fs::create_dir_all(&input_dir).expect("icons dir is created");

    let mut img = RgbaImage::from_pixel(32, 16, Rgba([255, 255, 255, 0]));
    for y in 5..=10 {
        for x in 1..=4 {
            img.put_pixel(x, y, Rgba([0, 0, 0, 255]));
        }
    }
    for y in 5..=10 {
        for x in 27..=30 {
            img.put_pixel(x, y, Rgba([0, 0, 0, 255]));
        }
    }
    img.save(input_dir.join("dot.png"))
        .expect("dot image is written");

    let mut compositions = BTreeMap::new();
    compositions.insert("dot.png".to_string(), CompositionDef { rows: 1, cols: 2 });
    let sources = vec![input_dir.join("dot.png")];
    let glyphs = preprocess_sources_with_compositions(&sources, &input_dir, 16, &compositions)
        .expect("composition preprocess succeeds");
    assert_eq!(glyphs.len(), 4);
    let tile_width = terminal_cell_width_for_height(16);
    let tile_height = 16;

    let top_left = glyphs
        .iter()
        .find(|glyph| {
            glyph
                .composition_tile
                .as_ref()
                .is_some_and(|tile| tile.row == 0 && tile.col == 0)
        })
        .expect("left tile exists");
    let bounds = nonzero_coverage_bounds_rect(&top_left.coverage, tile_width, tile_height)
        .expect("tile should be visible");
    assert!(
        bounds.0 <= 1,
        "left tile content should stay near its left edge after split (got bounds {:?})",
        bounds
    );

    let top_right = glyphs
        .iter()
        .find(|glyph| {
            glyph
                .composition_tile
                .as_ref()
                .is_some_and(|tile| tile.row == 0 && tile.col == 3)
        })
        .expect("right tile exists");
    let right_bounds = nonzero_coverage_bounds_rect(&top_right.coverage, tile_width, tile_height)
        .expect("right tile should be visible");
    assert!(
        right_bounds.2 >= tile_width.saturating_sub(2),
        "right tile content should stay near its right edge after split (got bounds {:?})",
        right_bounds
    );

    fs::remove_dir_all(project_dir).expect("temp project dir is removed");
}

#[test]
fn preprocess_composition_tiles_use_global_grid_scaling() {
    let project_dir = make_temp_dir("composition-global-grid-scaling");
    let input_dir = project_dir.join("icons");
    fs::create_dir_all(&input_dir).expect("icons dir is created");

    let mut img = RgbaImage::from_pixel(28, 28, Rgba([255, 255, 255, 0]));
    for y in 19..27 {
        for x in 23..27 {
            img.put_pixel(x, y, Rgba([0, 0, 0, 255]));
        }
    }
    for y in 23..25 {
        for x in 13..16 {
            img.put_pixel(x, y, Rgba([0, 0, 0, 255]));
        }
    }
    let source_path = input_dir.join("shape.png");
    img.save(&source_path).expect("shape image is written");

    let mut compositions = BTreeMap::new();
    compositions.insert("shape.png".to_string(), CompositionDef { rows: 3, cols: 3 });
    let sources = vec![source_path.clone()];
    let glyph_size = 16u32;
    let glyphs =
        preprocess_sources_with_compositions(&sources, &input_dir, glyph_size, &compositions)
            .expect("composition preprocess succeeds");
    assert_eq!(glyphs.len(), 18);
    let tile_width = terminal_cell_width_for_height(glyph_size);
    let tile_height = glyph_size;
    let emitted_cols = 6u32;

    let source = image::open(&source_path)
        .expect("source image opens")
        .to_rgba8();
    let expected_grid = fit_alpha_to_canvas(&source, emitted_cols * tile_width, 3 * tile_height);

    for glyph in &glyphs {
        let tile = glyph
            .composition_tile
            .as_ref()
            .expect("all glyphs should be composition tiles");
        let expected_coverage = crop_expected_coverage_tile(
            &expected_grid,
            emitted_cols * tile_width,
            (tile.col as u32) * tile_width,
            (tile.row as u32) * tile_height,
            tile_width,
            tile_height,
        );
        assert_eq!(
            glyph.coverage, expected_coverage,
            "tile coverage should match global fit-before-split for row={}, col={}",
            tile.row, tile.col
        );
    }

    fs::remove_dir_all(project_dir).expect("temp project dir is removed");
}

#[test]
fn preprocess_composition_tiles_keep_raw_internal_threshold_gradient() {
    let project_dir = make_temp_dir("composition-sealed-seams");
    let input_dir = project_dir.join("icons");
    fs::create_dir_all(&input_dir).expect("icons dir is created");

    let mut img = RgbaImage::from_pixel(32, 16, Rgba([255, 255, 255, 0]));
    img.put_pixel(15, 8, Rgba([0, 0, 0, 20]));
    img.put_pixel(16, 8, Rgba([0, 0, 0, 160]));
    let source_path = input_dir.join("seam.png");
    img.save(&source_path).expect("seam image is written");

    let mut compositions = BTreeMap::new();
    compositions.insert("seam.png".to_string(), CompositionDef { rows: 1, cols: 2 });
    let glyphs =
        preprocess_sources_with_compositions(&[source_path], &input_dir, 16, &compositions)
            .expect("composition preprocess succeeds");
    let tile_width = terminal_cell_width_for_height(16) as usize;

    let left = glyphs
        .iter()
        .find(|glyph| {
            glyph
                .composition_tile
                .as_ref()
                .is_some_and(|tile| tile.row == 0 && tile.col == 1)
        })
        .expect("left tile exists");
    let right = glyphs
        .iter()
        .find(|glyph| {
            glyph
                .composition_tile
                .as_ref()
                .is_some_and(|tile| tile.row == 0 && tile.col == 2)
        })
        .expect("right tile exists");

    let left_edge = left.coverage[8 * tile_width + tile_width - 1];
    let right_edge = right.coverage[8 * tile_width];
    assert!(
        left_edge < right_edge,
        "fit-before-split should preserve raw seam gradient (left={left_edge}, right={right_edge})"
    );

    fs::remove_dir_all(project_dir).expect("temp project dir is removed");
}

#[test]
fn build_outputs_composition_preserves_tile_offsets_in_ttf() {
    let project_dir = make_temp_dir("composition-ttf-offsets");
    let input_dir = project_dir.join("icons");
    fs::create_dir_all(&input_dir).expect("icons dir is created");
    write_split_edge_dots_png(&input_dir.join("split.png"), 32);

    let manifest_path = project_dir.join("petiglyph.toml");
    let mut manifest = Manifest::default();
    manifest.project_id = Some("test-composition-ttf-offsets".to_string());
    manifest
        .compositions
        .insert("split.png".to_string(), CompositionDef { rows: 1, cols: 2 });
    write_manifest(&manifest_path, &manifest).expect("manifest is written");

    let config = load_runtime_config(&manifest_path, None, None, None, None, None)
        .expect("runtime config loads");
    let summary = build_outputs(&config).expect("build succeeds");
    let mapping: Vec<MappingEntry> = serde_json::from_str(
        &fs::read_to_string(&summary.mapping_path).expect("mapping is written"),
    )
    .expect("mapping parses");

    let left_cp = mapping
        .iter()
        .find(|entry| entry.source_file == "split.png#compose:1x4:0:0")
        .map(|entry| parse_codepoint(&entry.codepoint).expect("left codepoint parses"))
        .expect("left composition tile is mapped");
    let right_cp = mapping
        .iter()
        .find(|entry| entry.source_file == "split.png#compose:1x4:0:3")
        .map(|entry| parse_codepoint(&entry.codepoint).expect("right codepoint parses"))
        .expect("right composition tile is mapped");

    let ttf = fs::read(summary.ttf_path).expect("ttf is written");
    let face = ttf_parser::Face::parse(&ttf, 0).expect("ttf parses");
    let left_bounds = face
        .glyph_bounding_box(
            face.glyph_index(char::from_u32(left_cp).expect("left codepoint is valid"))
                .expect("left glyph maps"),
        )
        .expect("left glyph has bounds");
    let right_bounds = face
        .glyph_bounding_box(
            face.glyph_index(char::from_u32(right_cp).expect("right codepoint is valid"))
                .expect("right glyph maps"),
        )
        .expect("right glyph has bounds");

    let left_sum = i32::from(left_bounds.x_min) + i32::from(left_bounds.x_max);
    let right_sum = i32::from(right_bounds.x_min) + i32::from(right_bounds.x_max);
    assert!(
        right_sum > left_sum,
        "right tile should stay to the right of left tile in TTF metrics (left_sum={left_sum}, right_sum={right_sum})"
    );

    fs::remove_dir_all(project_dir).expect("temp project dir is removed");
}

#[test]
fn build_outputs_composition_bleeds_internal_ttf_edges() {
    let project_dir = make_temp_dir("composition-ttf-overlap");
    let input_dir = project_dir.join("icons");
    fs::create_dir_all(&input_dir).expect("icons dir is created");

    let mut img = RgbaImage::from_pixel(32, 16, Rgba([255, 255, 255, 0]));
    for y in 0..16 {
        img.put_pixel(15, y, Rgba([0, 0, 0, 255]));
        img.put_pixel(16, y, Rgba([0, 0, 0, 255]));
    }
    img.save(input_dir.join("seam.png"))
        .expect("seam image is written");

    let manifest_path = project_dir.join("petiglyph.toml");
    let mut manifest = Manifest::default();
    manifest.project_id = Some("test-composition-ttf-overlap".to_string());
    manifest
        .compositions
        .insert("seam.png".to_string(), CompositionDef { rows: 1, cols: 2 });
    write_manifest(&manifest_path, &manifest).expect("manifest is written");

    let config = load_runtime_config(&manifest_path, None, None, None, Some(16), None)
        .expect("runtime config loads");
    let summary = build_outputs(&config).expect("build succeeds");
    let mapping: Vec<MappingEntry> = serde_json::from_str(
        &fs::read_to_string(&summary.mapping_path).expect("mapping is written"),
    )
    .expect("mapping parses");
    let ttf = fs::read(summary.ttf_path).expect("ttf is written");
    let face = ttf_parser::Face::parse(&ttf, 0).expect("ttf parses");
    let cell_advance = i16::try_from(face.units_per_em() / 2).expect("cell advance fits i16");

    let left_cp = mapping
        .iter()
        .find(|entry| entry.source_file == "seam.png#compose:1x4:0:1")
        .map(|entry| parse_codepoint(&entry.codepoint).expect("left codepoint parses"))
        .expect("left tile is mapped");
    let right_cp = mapping
        .iter()
        .find(|entry| entry.source_file == "seam.png#compose:1x4:0:2")
        .map(|entry| parse_codepoint(&entry.codepoint).expect("right codepoint parses"))
        .expect("right tile is mapped");

    let left_bounds = face
        .glyph_bounding_box(
            face.glyph_index(char::from_u32(left_cp).expect("left codepoint is valid"))
                .expect("left glyph maps"),
        )
        .expect("left glyph has bounds");
    let right_bounds = face
        .glyph_bounding_box(
            face.glyph_index(char::from_u32(right_cp).expect("right codepoint is valid"))
                .expect("right glyph maps"),
        )
        .expect("right glyph has bounds");

    assert_eq!(
        face.glyph_hor_advance(
            face.glyph_index(char::from_u32(left_cp).expect("left codepoint is valid"))
                .expect("left glyph maps")
        ),
        Some(cell_advance as u16),
        "bleed must not change the one-cell advance"
    );
    assert!(
        left_bounds.x_max > cell_advance,
        "left tile should bleed past the right cell edge (x_max={}, cell_advance={cell_advance})",
        left_bounds.x_max
    );
    assert!(
        left_bounds.x_max <= cell_advance + cell_advance / 4,
        "left tile bleed should stay small (x_max={}, units_per_em={})",
        left_bounds.x_max,
        face.units_per_em()
    );
    assert!(
        right_bounds.x_min < 0,
        "right tile should bleed before the left cell edge (x_min={}, cell_advance={cell_advance})",
        right_bounds.x_min
    );
    assert!(
        right_bounds.x_min >= -(cell_advance / 4),
        "right tile bleed should stay small (x_min={}, units_per_em={})",
        right_bounds.x_min,
        face.units_per_em()
    );

    fs::remove_dir_all(project_dir).expect("temp project dir is removed");
}

#[test]
fn build_outputs_composition_overlaps_internal_ttf_edges_vertically() {
    let project_dir = make_temp_dir("composition-ttf-overlap-vertical");
    let input_dir = project_dir.join("icons");
    fs::create_dir_all(&input_dir).expect("icons dir is created");

    let mut img = RgbaImage::from_pixel(16, 32, Rgba([255, 255, 255, 0]));
    for x in 0..16 {
        img.put_pixel(x, 15, Rgba([0, 0, 0, 255]));
        img.put_pixel(x, 16, Rgba([0, 0, 0, 255]));
    }
    img.save(input_dir.join("vseam.png"))
        .expect("vertical seam image is written");

    let manifest_path = project_dir.join("petiglyph.toml");
    let mut manifest = Manifest::default();
    manifest.project_id = Some("test-composition-ttf-overlap-vertical".to_string());
    manifest
        .compositions
        .insert("vseam.png".to_string(), CompositionDef { rows: 2, cols: 1 });
    write_manifest(&manifest_path, &manifest).expect("manifest is written");

    let config = load_runtime_config(&manifest_path, None, None, None, Some(16), None)
        .expect("runtime config loads");
    let summary = build_outputs(&config).expect("build succeeds");
    let mapping: Vec<MappingEntry> = serde_json::from_str(
        &fs::read_to_string(&summary.mapping_path).expect("mapping is written"),
    )
    .expect("mapping parses");
    let ttf = fs::read(summary.ttf_path).expect("ttf is written");
    let face = ttf_parser::Face::parse(&ttf, 0).expect("ttf parses");
    let asc = i32::from(face.ascender());
    let desc = i32::from(face.descender());
    let minimum_expected_overlap = i32::from(face.units_per_em()) / 4;

    let top_cp = mapping
        .iter()
        .find(|entry| entry.source_file == "vseam.png#compose:2x2:0:0")
        .map(|entry| parse_codepoint(&entry.codepoint).expect("top codepoint parses"))
        .expect("top tile is mapped");
    let bottom_cp = mapping
        .iter()
        .find(|entry| entry.source_file == "vseam.png#compose:2x2:1:0")
        .map(|entry| parse_codepoint(&entry.codepoint).expect("bottom codepoint parses"))
        .expect("bottom tile is mapped");

    let top_bounds = face
        .glyph_bounding_box(
            face.glyph_index(char::from_u32(top_cp).expect("top codepoint is valid"))
                .expect("top glyph maps"),
        )
        .expect("top glyph has bounds");
    let bottom_bounds = face
        .glyph_bounding_box(
            face.glyph_index(char::from_u32(bottom_cp).expect("bottom codepoint is valid"))
                .expect("bottom glyph maps"),
        )
        .expect("bottom glyph has bounds");

    // Line N+1 top for bottom glyph in baseline coordinates of line N:
    // top_next = y_max(bottom) - (asc - desc)
    let next_line_top = i32::from(bottom_bounds.y_max) - (asc - desc);
    let overlap = next_line_top - i32::from(top_bounds.y_min);
    assert!(
        overlap >= minimum_expected_overlap,
        "adjacent composition rows should have strong vertical overscan (overlap={overlap}, minimum={minimum_expected_overlap}, top.y_min={}, next_line_top={next_line_top}, asc={asc}, desc={desc})",
        top_bounds.y_min
    );

    fs::remove_dir_all(project_dir).expect("temp project dir is removed");
}

#[test]
fn build_outputs_empty_composition_tile_keeps_ttf_advance() {
    let project_dir = make_temp_dir("composition-empty-advance");
    let input_dir = project_dir.join("icons");
    fs::create_dir_all(&input_dir).expect("icons dir is created");

    let mut img = RgbaImage::from_pixel(48, 16, Rgba([255, 255, 255, 0]));
    img.put_pixel(1, 8, Rgba([0, 0, 0, 255]));
    img.put_pixel(46, 8, Rgba([0, 0, 0, 255]));
    img.save(input_dir.join("empty-middle.png"))
        .expect("empty-middle image is written");

    let manifest_path = project_dir.join("petiglyph.toml");
    let mut manifest = Manifest::default();
    manifest.project_id = Some("test-composition-empty-advance".to_string());
    manifest.compositions.insert(
        "empty-middle.png".to_string(),
        CompositionDef { rows: 1, cols: 3 },
    );
    write_manifest(&manifest_path, &manifest).expect("manifest is written");

    let config = load_runtime_config(&manifest_path, None, None, None, Some(16), None)
        .expect("runtime config loads");
    let summary = build_outputs(&config).expect("build succeeds");
    let mapping: Vec<MappingEntry> = serde_json::from_str(
        &fs::read_to_string(&summary.mapping_path).expect("mapping is written"),
    )
    .expect("mapping parses");
    let middle_cp = mapping
        .iter()
        .find(|entry| entry.source_file == "empty-middle.png#compose:1x6:0:3")
        .map(|entry| parse_codepoint(&entry.codepoint).expect("middle codepoint parses"))
        .expect("middle tile is mapped");

    let ttf = fs::read(summary.ttf_path).expect("ttf is written");
    let face = ttf_parser::Face::parse(&ttf, 0).expect("ttf parses");
    let glyph_id = face
        .glyph_index(char::from_u32(middle_cp).expect("middle codepoint is valid"))
        .expect("middle glyph maps");

    assert!(
        face.glyph_bounding_box(glyph_id).is_none(),
        "empty tile should remain visually empty"
    );
    assert_eq!(
        face.glyph_hor_advance(glyph_id),
        Some(face.units_per_em() / 2),
        "empty tile must still reserve one full glyph cell"
    );

    fs::remove_dir_all(project_dir).expect("temp project dir is removed");
}

#[test]
fn build_outputs_remaps_noncontiguous_composition_lock_into_contiguous_run() {
    let project_dir = make_temp_dir("composition-lock-remap");
    let input_dir = project_dir.join("icons");
    fs::create_dir_all(&input_dir).expect("icons dir is created");
    write_quadrant_png(&input_dir.join("icon.png"), 64);

    let manifest_path = project_dir.join("petiglyph.toml");
    let mut manifest = Manifest::default();
    manifest.project_id = Some("test-composition-lock-remap".to_string());
    manifest
        .compositions
        .insert("icon.png".to_string(), CompositionDef { rows: 2, cols: 2 });
    write_manifest(&manifest_path, &manifest).expect("manifest is written");

    let lock_path = project_dir.join("petiglyph.lock");
    let lock = serde_json::json!({
        "version": 1,
        "project_id": "test-composition-lock-remap",
        "codepoint_start": "U+100000",
        "entries": [
            { "source_file": "icon.png#compose:2x4:0:0", "codepoint": "U+100000", "image_fingerprint": "fnv1a64:a", "active": true },
            { "source_file": "icon.png#compose:2x4:0:1", "codepoint": "U+100002", "image_fingerprint": "fnv1a64:b", "active": true },
            { "source_file": "icon.png#compose:2x4:0:2", "codepoint": "U+100001", "image_fingerprint": "fnv1a64:c", "active": true },
            { "source_file": "icon.png#compose:2x4:0:3", "codepoint": "U+100004", "image_fingerprint": "fnv1a64:d", "active": true },
            { "source_file": "icon.png#compose:2x4:1:0", "codepoint": "U+100003", "image_fingerprint": "fnv1a64:e", "active": true },
            { "source_file": "icon.png#compose:2x4:1:1", "codepoint": "U+100006", "image_fingerprint": "fnv1a64:f", "active": true },
            { "source_file": "icon.png#compose:2x4:1:2", "codepoint": "U+100005", "image_fingerprint": "fnv1a64:g", "active": true },
            { "source_file": "icon.png#compose:2x4:1:3", "codepoint": "U+100007", "image_fingerprint": "fnv1a64:h", "active": true }
        ]
    });
    fs::write(
        &lock_path,
        serde_json::to_string_pretty(&lock).expect("lock serializes"),
    )
    .expect("lock is written");

    let config = load_runtime_config(&manifest_path, None, None, None, None, None)
        .expect("runtime config loads");
    let summary = build_outputs(&config).expect("build succeeds");
    let mapping: Vec<MappingEntry> = serde_json::from_str(
        &fs::read_to_string(summary.mapping_path).expect("mapping is written"),
    )
    .expect("mapping parses");

    let codepoints = mapping
        .iter()
        .map(|entry| parse_codepoint(&entry.codepoint).expect("codepoint parses"))
        .collect::<Vec<_>>();
    for pair in codepoints.windows(2) {
        assert_eq!(pair[1], pair[0] + 1, "composition tiles must be contiguous");
    }
    assert!(
        codepoints[0] > 0x10_0003,
        "remap should allocate a fresh contiguous run when legacy mapping is fragmented"
    );

    let updated_lock: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(lock_path).expect("lock is readable"))
            .expect("updated lock parses");
    let retired_count = updated_lock["entries"]
        .as_array()
        .expect("entries is an array")
        .iter()
        .filter(|entry| {
            entry["source_file"]
                .as_str()
                .is_some_and(|value| value.contains("#retired:"))
        })
        .count();
    assert!(
        retired_count >= 1,
        "remapped composition should keep retired tombstones in lock"
    );

    fs::remove_dir_all(project_dir).expect("temp project dir is removed");
}

#[test]
fn build_outputs_centers_non_square_glyphs_in_ttf_line_box() {
    let project_dir = make_temp_dir("non-square-centering");
    let input_dir = project_dir.join("icons");
    let out_dir = project_dir.join("build");
    fs::create_dir_all(&input_dir).expect("input dir is created");
    write_transparent_rect_png(&input_dir.join("wide.png"), 100, 77);
    write_transparent_rect_png(&input_dir.join("tall.png"), 77, 100);

    let config = RuntimeConfig {
        project_dir: project_dir.clone(),
        project_id: "test-non-square-centering".to_string(),
        input_dir: input_dir.clone(),
        out_dir,
        font_name: "Petiglyph".to_string(),
        glyph_size: 64,
        base_threshold: 64,
        threshold_overrides: BTreeMap::new(),
        compositions: BTreeMap::new(),
        codepoint_start: 0x10_0000,
    };

    let summary = build_outputs(&config).expect("build succeeds");
    let bdf = fs::read_to_string(&summary.bdf_path).expect("bdf is written");
    let ttf = fs::read(&summary.ttf_path).expect("ttf is written");
    let face = ttf_parser::Face::parse(&ttf, 0).expect("ttf parses");
    assert!(
        bdf.contains("FONT_DESCENT 13\n"),
        "BDF export should reserve descender space"
    );
    assert!(
        face.descender() < 0,
        "icon font should reserve descender space"
    );
    let expected_y_min_max_sum = i32::from(face.ascender()) + i32::from(face.descender());

    for idx in 0..summary.glyph_count {
        let codepoint = config.codepoint_start + idx as u32;
        let glyph_id = face
            .glyph_index(char::from_u32(codepoint).expect("codepoint is valid"))
            .expect("glyph is mapped");
        let bounds = face.glyph_bounding_box(glyph_id).expect("glyph has bounds");
        let expected_x_min_max_sum = i32::from(
            face.glyph_hor_advance(glyph_id)
                .expect("glyph has horizontal advance"),
        );

        assert_eq!(
            i32::from(bounds.x_min) + i32::from(bounds.x_max),
            expected_x_min_max_sum,
            "glyph U+{codepoint:04X} should be horizontally centered"
        );
        let vertical_center_delta =
            (i32::from(bounds.y_min) + i32::from(bounds.y_max) - expected_y_min_max_sum).abs();
        assert!(
            vertical_center_delta <= 1,
            "glyph U+{codepoint:04X} should be vertically centered around the line box (delta={vertical_center_delta})"
        );
        let lower_margin = i32::from(bounds.y_min) - i32::from(face.descender());
        let upper_margin = i32::from(face.ascender()) - i32::from(bounds.y_max);
        assert!(
            (lower_margin - upper_margin).abs() <= 1,
            "glyph U+{codepoint:04X} should have balanced vertical line-box margins (lower={lower_margin}, upper={upper_margin})"
        );
    }

    fs::remove_dir_all(project_dir).expect("temp project dir is removed");
}

#[test]
fn build_outputs_embeds_project_identity_in_ttf_unique_name() {
    let project_dir = make_temp_dir("ttf-unique-identity");
    let input_dir = project_dir.join("icons");
    let out_dir = project_dir.join("build");
    fs::create_dir_all(&input_dir).expect("icons dir is created");
    fs::create_dir_all(&out_dir).expect("build dir is created");
    write_test_png(&input_dir.join("alpha.png"));

    let config = RuntimeConfig {
        project_dir: project_dir.clone(),
        project_id: "phase2_project_identity_abc123".to_string(),
        input_dir,
        out_dir,
        font_name: "UniqueName".to_string(),
        glyph_size: 16,
        base_threshold: 64,
        threshold_overrides: BTreeMap::new(),
        compositions: BTreeMap::new(),
        codepoint_start: 0x10_0000,
    };

    let summary = build_outputs(&config).expect("build succeeds");
    let ttf = fs::read(&summary.ttf_path).expect("ttf is readable");
    let face = ttf_parser::Face::parse(&ttf, 0).expect("ttf parses");

    let unique_ids = face
        .names()
        .into_iter()
        .filter(|name| name.name_id == ttf_parser::name_id::UNIQUE_ID && name.is_unicode())
        .filter_map(|name| name.to_string())
        .collect::<Vec<_>>();

    assert!(
        unique_ids
            .iter()
            .any(|value| value.contains(&config.project_id)
                && value.contains(env!("CARGO_PKG_VERSION"))),
        "unique name ID should contain project identity and CLI version; got {unique_ids:?}"
    );

    fs::remove_dir_all(project_dir).expect("temp project dir is removed");
}

#[test]
fn sample_glyphs_from_ttf_bytes_limits_preview_to_requested_count() {
    let project_dir = make_temp_dir("ttf-sample-limit");
    let input_dir = project_dir.join("icons");
    let out_dir = project_dir.join("build");
    fs::create_dir_all(&input_dir).expect("icons dir is created");
    fs::create_dir_all(&out_dir).expect("build dir is created");
    for idx in 0..20 {
        write_test_png(&input_dir.join(format!("icon-{idx}.png")));
    }

    let config = RuntimeConfig {
        project_dir: project_dir.clone(),
        project_id: "test-ttf-sample-limit".to_string(),
        input_dir,
        out_dir,
        font_name: "SampleLimit".to_string(),
        glyph_size: 16,
        base_threshold: 64,
        threshold_overrides: BTreeMap::new(),
        compositions: BTreeMap::new(),
        codepoint_start: 0x10_0000,
    };
    let summary = build_outputs(&config).expect("build succeeds");
    let ttf = fs::read(&summary.ttf_path).expect("ttf is readable");

    let (sample, truncated) =
        sample_glyphs_from_ttf_bytes(&ttf, 15).expect("sample should be extracted");
    assert_eq!(sample.chars().count(), 15);
    assert!(truncated, "sample should be marked as truncated");

    fs::remove_dir_all(project_dir).expect("temp project dir is removed");
}

#[test]
fn build_outputs_supports_upper_unicode_edge() {
    let project_dir = make_temp_dir("unicode-edge");
    let input_dir = project_dir.join("icons");
    let out_dir = project_dir.join("build");
    fs::create_dir_all(&input_dir).expect("icons dir is created");
    fs::create_dir_all(&out_dir).expect("build dir is created");
    write_test_png(&input_dir.join("a.png"));
    write_test_png(&input_dir.join("b.png"));

    let config = RuntimeConfig {
        project_dir: project_dir.clone(),
        project_id: "test-unicode-edge".to_string(),
        input_dir,
        out_dir: out_dir.clone(),
        font_name: "Edge".to_string(),
        glyph_size: 32,
        base_threshold: 64,
        threshold_overrides: BTreeMap::new(),
        compositions: BTreeMap::new(),
        codepoint_start: 0x10_FFFE,
    };

    let summary = build_outputs(&config).expect("build succeeds");
    let mapping_json = fs::read_to_string(&summary.mapping_path).expect("glyph map is written");
    let mapping: Vec<MappingEntry> = serde_json::from_str(&mapping_json).expect("glyph map parses");

    assert_eq!(mapping.len(), 2);
    assert_eq!(mapping[0].codepoint, "U+10FFFE");
    assert_eq!(mapping[1].codepoint, "U+10FFFF");
    assert_eq!(
        fs::read_to_string(summary.sample_path)
            .expect("sample is written")
            .trim()
            .to_string(),
        glyph_sample_string(0x10_FFFE, 2)
    );

    fs::remove_dir_all(project_dir).expect("temp project dir is removed");
}

#[test]
fn build_outputs_rejects_codepoint_range_above_unicode_max() {
    let project_dir = make_temp_dir("unicode-overflow");
    let input_dir = project_dir.join("icons");
    let out_dir = project_dir.join("build");
    fs::create_dir_all(&input_dir).expect("icons dir is created");
    fs::create_dir_all(&out_dir).expect("build dir is created");
    write_test_png(&input_dir.join("a.png"));
    write_test_png(&input_dir.join("b.png"));

    let config = RuntimeConfig {
        project_dir: project_dir.clone(),
        project_id: "test-unicode-overflow".to_string(),
        input_dir,
        out_dir,
        font_name: "Overflow".to_string(),
        glyph_size: 32,
        base_threshold: 64,
        threshold_overrides: BTreeMap::new(),
        compositions: BTreeMap::new(),
        codepoint_start: 0x10_FFFF,
    };

    let error = build_outputs(&config).expect_err("build should reject codepoint overflow");
    let message = format!("{error:#}");
    assert!(
        message.contains("codepoint range exceeds Unicode limit"),
        "unexpected error: {message}"
    );

    fs::remove_dir_all(project_dir).expect("temp project dir is removed");
}

#[test]
fn build_outputs_rejects_codepoint_range_crossing_surrogates() {
    let project_dir = make_temp_dir("unicode-surrogate-crossing");
    let input_dir = project_dir.join("icons");
    let out_dir = project_dir.join("build");
    fs::create_dir_all(&input_dir).expect("icons dir is created");
    fs::create_dir_all(&out_dir).expect("build dir is created");
    write_test_png(&input_dir.join("a.png"));
    write_test_png(&input_dir.join("b.png"));

    let config = RuntimeConfig {
        project_dir: project_dir.clone(),
        project_id: "test-unicode-surrogate".to_string(),
        input_dir,
        out_dir,
        font_name: "Surrogate".to_string(),
        glyph_size: 32,
        base_threshold: 64,
        threshold_overrides: BTreeMap::new(),
        compositions: BTreeMap::new(),
        codepoint_start: 0xD7FF,
    };

    let error = build_outputs(&config).expect_err("build should reject surrogate-range codepoint");
    let message = format!("{error:#}");
    assert!(
        message.contains("surrogate range"),
        "unexpected error: {message}"
    );

    fs::remove_dir_all(project_dir).expect("temp project dir is removed");
}

#[test]
fn load_runtime_config_generates_and_persists_project_id() {
    let project_dir = make_temp_dir("project-id-migration");
    let manifest_path = project_dir.join("petiglyph.toml");
    write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");

    let config = load_runtime_config(&manifest_path, None, None, None, None, None)
        .expect("runtime config should load");
    assert!(
        !config.project_id.trim().is_empty(),
        "project_id should be generated"
    );

    let manifest = read_manifest(&manifest_path).expect("manifest is readable");
    assert_eq!(
        manifest.project_id.as_deref(),
        Some(config.project_id.as_str()),
        "generated project_id should be persisted"
    );

    fs::remove_dir_all(project_dir).expect("temp project dir is removed");
}

#[test]
fn build_outputs_preserves_existing_codepoints_when_new_sorted_source_is_added() {
    let project_dir = make_temp_dir("stable-codepoints-on-insert");
    let input_dir = project_dir.join("icons");
    let out_dir = project_dir.join("build");
    fs::create_dir_all(&input_dir).expect("icons dir is created");
    fs::create_dir_all(&out_dir).expect("build dir is created");

    write_test_png(&input_dir.join("b.png"));
    write_test_png(&input_dir.join("c.png"));

    let config = RuntimeConfig {
        project_dir: project_dir.clone(),
        project_id: "test-stable-insert".to_string(),
        input_dir: input_dir.clone(),
        out_dir: out_dir.clone(),
        font_name: "StableInsert".to_string(),
        glyph_size: 16,
        base_threshold: 64,
        threshold_overrides: BTreeMap::new(),
        compositions: BTreeMap::new(),
        codepoint_start: 0x10_0000,
    };

    let first = build_outputs(&config).expect("first build should succeed");
    let first_map: Vec<MappingEntry> =
        serde_json::from_str(&fs::read_to_string(&first.mapping_path).expect("map is readable"))
            .expect("map parses");
    let first_b = first_map
        .iter()
        .find(|entry| entry.source_file == "b.png")
        .expect("b.png should be mapped")
        .codepoint
        .clone();
    let first_c = first_map
        .iter()
        .find(|entry| entry.source_file == "c.png")
        .expect("c.png should be mapped")
        .codepoint
        .clone();

    write_test_png(&input_dir.join("a.png"));

    let second = build_outputs(&config).expect("second build should succeed");
    let second_map: Vec<MappingEntry> =
        serde_json::from_str(&fs::read_to_string(&second.mapping_path).expect("map is readable"))
            .expect("map parses");
    let second_b = second_map
        .iter()
        .find(|entry| entry.source_file == "b.png")
        .expect("b.png should be mapped")
        .codepoint
        .clone();
    let second_c = second_map
        .iter()
        .find(|entry| entry.source_file == "c.png")
        .expect("c.png should be mapped")
        .codepoint
        .clone();
    let second_a = second_map
        .iter()
        .find(|entry| entry.source_file == "a.png")
        .expect("a.png should be mapped")
        .codepoint
        .clone();

    assert_eq!(
        first_b, second_b,
        "existing b.png codepoint should remain stable"
    );
    assert_eq!(
        first_c, second_c,
        "existing c.png codepoint should remain stable"
    );
    assert!(
        parse_codepoint(&second_a).expect("new codepoint parses")
            > parse_codepoint(&first_c).expect("existing codepoint parses"),
        "new sorted file should get a newly allocated codepoint instead of shifting old mappings"
    );

    fs::remove_dir_all(project_dir).expect("temp project dir is removed");
}

#[test]
fn build_outputs_tombstones_removed_sources_and_does_not_reuse_their_codepoints() {
    let project_dir = make_temp_dir("stable-codepoints-tombstone");
    let input_dir = project_dir.join("icons");
    let out_dir = project_dir.join("build");
    fs::create_dir_all(&input_dir).expect("icons dir is created");
    fs::create_dir_all(&out_dir).expect("build dir is created");

    write_test_png(&input_dir.join("a.png"));
    write_test_png(&input_dir.join("b.png"));

    let config = RuntimeConfig {
        project_dir: project_dir.clone(),
        project_id: "test-tombstone".to_string(),
        input_dir: input_dir.clone(),
        out_dir: out_dir.clone(),
        font_name: "Tombstone".to_string(),
        glyph_size: 16,
        base_threshold: 64,
        threshold_overrides: BTreeMap::new(),
        compositions: BTreeMap::new(),
        codepoint_start: 0x10_0000,
    };

    let first = build_outputs(&config).expect("first build should succeed");
    let first_map: Vec<MappingEntry> =
        serde_json::from_str(&fs::read_to_string(&first.mapping_path).expect("map is readable"))
            .expect("map parses");
    let removed_codepoint = first_map
        .iter()
        .find(|entry| entry.source_file == "b.png")
        .expect("b.png should be mapped")
        .codepoint
        .clone();

    fs::remove_file(input_dir.join("b.png")).expect("old file is removed");
    write_test_png(&input_dir.join("c.png"));

    let second = build_outputs(&config).expect("second build should succeed");
    let second_map: Vec<MappingEntry> =
        serde_json::from_str(&fs::read_to_string(&second.mapping_path).expect("map is readable"))
            .expect("map parses");
    let new_codepoint = second_map
        .iter()
        .find(|entry| entry.source_file == "c.png")
        .expect("c.png should be mapped")
        .codepoint
        .clone();

    assert_ne!(
        new_codepoint, removed_codepoint,
        "removed glyph codepoint should not be reused"
    );

    let lock_path = project_dir.join("petiglyph.lock");
    let lock_raw = fs::read_to_string(lock_path).expect("lock file should be written");
    let lock_json: serde_json::Value = serde_json::from_str(&lock_raw).expect("lock parses");
    let entries = lock_json
        .get("entries")
        .and_then(|value| value.as_array())
        .expect("lock entries should be an array");

    let tombstone_entry = entries
        .iter()
        .find(|entry| entry.get("source_file").and_then(|v| v.as_str()) == Some("b.png"))
        .expect("removed file should remain as tombstone");
    assert_eq!(
        tombstone_entry.get("active").and_then(|v| v.as_bool()),
        Some(false)
    );

    fs::remove_dir_all(project_dir).expect("temp project dir is removed");
}

#[test]
fn unicode_registry_allocates_disjoint_ranges_for_different_projects() {
    let registry_root = make_temp_dir("unicode-registry-disjoint");
    let locked = BTreeSet::new();

    let a = reserve_project_unicode_range(Some(&registry_root), "project-a", 0x10_0000, 4, &locked)
        .expect("project-a reservation should succeed");
    let b = reserve_project_unicode_range(Some(&registry_root), "project-b", 0x10_0000, 4, &locked)
        .expect("project-b reservation should succeed");

    assert_eq!(a.range_start, 0x10_0000);
    assert!(
        b.range_start > a.range_end || a.range_start > b.range_end,
        "project ranges must not overlap: a=U+{:X}..U+{:X}, b=U+{:X}..U+{:X}",
        a.range_start,
        a.range_end,
        b.range_start,
        b.range_end
    );

    let lock_path = registry_root.join("petiglyph/.unicode-registry.lock");
    assert!(
        !lock_path.exists(),
        "registry lock should be released after reservation"
    );

    fs::remove_dir_all(registry_root).expect("temp registry root is removed");
}

#[test]
fn unicode_registry_rejects_locked_codepoint_conflicts_with_foreign_project() {
    let registry_root = make_temp_dir("unicode-registry-conflict");
    let locked = BTreeSet::new();

    let _a =
        reserve_project_unicode_range(Some(&registry_root), "project-a", 0x10_0000, 8, &locked)
            .expect("project-a reservation should succeed");

    let mut conflicting_locked = BTreeSet::new();
    conflicting_locked.insert(0x10_0002);
    let error = reserve_project_unicode_range(
        Some(&registry_root),
        "project-b",
        0x10_0000,
        1,
        &conflicting_locked,
    )
    .expect_err("reservation should fail on owned codepoint conflict");

    let message = format!("{error:#}");
    assert!(
        message.contains("conflict") && message.contains("project-a"),
        "unexpected conflict error: {message}"
    );

    fs::remove_dir_all(registry_root).expect("temp registry root is removed");
}

#[test]
fn unicode_registry_can_relocate_project_range_while_preserving_locked_codepoints() {
    let registry_root = make_temp_dir("unicode-registry-relocate");

    let mut project_a_locked = BTreeSet::new();
    project_a_locked.insert(0x10_0025);
    let first = reserve_project_unicode_range(
        Some(&registry_root),
        "project-a",
        0x10_0000,
        1,
        &project_a_locked,
    )
    .expect("initial project-a reservation should succeed");
    assert_eq!(first.range_start, 0x10_0025);
    assert_eq!(first.range_end, 0x10_0025);

    let mut project_b_locked = BTreeSet::new();
    project_b_locked.insert(0x10_0026);
    let second = reserve_project_unicode_range(
        Some(&registry_root),
        "project-b",
        0x10_0000,
        1,
        &project_b_locked,
    )
    .expect("project-b reservation should succeed");
    assert_eq!(second.range_start, 0x10_0026);
    assert_eq!(second.range_end, 0x10_0026);

    let expanded = reserve_project_unicode_range(
        Some(&registry_root),
        "project-a",
        0x10_0000,
        3,
        &project_a_locked,
    )
    .expect("project-a should relocate range to keep lock and avoid collision");

    assert!(
        expanded.range_start <= 0x10_0025 && expanded.range_end >= 0x10_0025,
        "expanded range must still contain locked codepoint: U+100025, got U+{:X}..U+{:X}",
        expanded.range_start,
        expanded.range_end
    );
    assert_eq!(
        expanded.range_end - expanded.range_start + 1,
        3,
        "expanded range should match requested span"
    );
    assert!(
        expanded.range_end < second.range_start || expanded.range_start > second.range_end,
        "expanded range must remain disjoint from project-b range"
    );

    fs::remove_dir_all(registry_root).expect("temp registry root is removed");
}

#[test]
fn build_force_remap_recovers_from_foreign_codepoint_conflict() {
    let project_dir = make_temp_dir("force-remap-conflict");
    let input_dir = project_dir.join("icons");
    let out_dir = project_dir.join("build");
    fs::create_dir_all(&input_dir).expect("icons dir is created");
    fs::create_dir_all(&out_dir).expect("build dir is created");
    write_test_png(&input_dir.join("icon.png"));

    let foreign_locked = BTreeSet::new();
    let foreign = reserve_project_unicode_range(
        Some(&project_dir),
        "project-a",
        0x10_0000,
        8,
        &foreign_locked,
    )
    .expect("project-a reservation should succeed");

    let conflicting_codepoint = format_codepoint(foreign.range_start);
    let lock_json = serde_json::json!({
        "version": 1,
        "project_id": "project-b",
        "codepoint_start": "U+100000",
        "entries": [
            {
                "source_file": "icon.png",
                "codepoint": conflicting_codepoint,
                "image_fingerprint": "fnv1a64:0000000000000000",
                "active": true
            }
        ]
    });
    fs::write(
        project_dir.join("petiglyph.lock"),
        serde_json::to_string_pretty(&lock_json).expect("lock serializes"),
    )
    .expect("lock is written");

    let config = RuntimeConfig {
        project_dir: project_dir.clone(),
        project_id: "project-b".to_string(),
        input_dir,
        out_dir,
        font_name: "ForceRemap".to_string(),
        glyph_size: 16,
        base_threshold: 64,
        threshold_overrides: BTreeMap::new(),
        compositions: BTreeMap::new(),
        codepoint_start: 0x10_0000,
    };

    let regular_error = build_outputs(&config).expect_err("regular build should fail on conflict");
    let regular_message = format!("{regular_error:#}");
    assert!(
        regular_message.contains("conflict"),
        "unexpected regular conflict error: {regular_message}"
    );

    build_outputs_with_options(&config, BuildOptions { force_remap: true })
        .expect("force remap build should recover and succeed");

    fs::remove_dir_all(project_dir).expect("temp project dir is removed");
}

#[test]
fn persist_threshold_override_roundtrip() {
    let project_dir = make_temp_dir("override-roundtrip");
    let manifest_path = project_dir.join("petiglyph.toml");
    write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");

    persist_threshold_override(&manifest_path, "icon.png", Some(77))
        .expect("override should persist");
    let manifest = read_manifest(&manifest_path).expect("manifest reads");
    assert_eq!(manifest.threshold_overrides.get("icon.png"), Some(&77));

    persist_threshold_override(&manifest_path, "icon.png", None).expect("override should clear");
    let manifest = read_manifest(&manifest_path).expect("manifest reads");
    assert!(!manifest.threshold_overrides.contains_key("icon.png"));

    fs::remove_dir_all(project_dir).expect("temp project dir is removed");
}

#[test]
fn app_new_hydrates_previous_build_outputs_from_disk() {
    let project_dir = make_temp_dir("hydrate-build-state");
    let manifest_path = project_dir.join("petiglyph.toml");
    let build_dir = project_dir.join("build");
    fs::create_dir_all(&build_dir).expect("build dir is created");

    fs::write(build_dir.join("petiglyph.ttf"), b"not-a-real-ttf").expect("ttf is written");
    fs::write(build_dir.join("petiglyph.bdf"), b"not-a-real-bdf").expect("bdf is written");
    fs::write(
        build_dir.join("glyph-map.json"),
        serde_json::to_string(&vec![MappingEntry {
            glyph_name: "icon".to_string(),
            source_file: "icon.png".to_string(),
            codepoint: "U+100000".to_string(),
        }])
        .expect("mapping serializes"),
    )
    .expect("mapping is written");
    fs::write(build_dir.join("glyph-sample.txt"), "sample\n").expect("sample is written");

    let config = RuntimeConfig {
        project_dir: project_dir.clone(),
        project_id: "test-hydrate-last-build".to_string(),
        input_dir: project_dir.join("icons"),
        out_dir: build_dir.clone(),
        font_name: "Petiglyph".to_string(),
        glyph_size: 8,
        base_threshold: 64,
        threshold_overrides: BTreeMap::new(),
        compositions: BTreeMap::new(),
        codepoint_start: 0x10_0000,
    };

    let app = App::new(manifest_path, config);
    let summary = app
        .last_build
        .as_ref()
        .expect("existing build should hydrate");
    assert_eq!(summary.glyph_count, 1);
    assert_eq!(summary.ttf_path, build_dir.join("petiglyph.ttf"));
    assert_eq!(summary.bdf_path, build_dir.join("petiglyph.bdf"));
    assert_eq!(app.last_sample.as_deref(), Some("sample"));

    fs::remove_dir_all(project_dir).expect("temp project dir is removed");
}

#[test]
fn handle_key_updates_and_clears_selected_threshold_override() {
    let project_dir = make_temp_dir("handle-key-threshold");
    let manifest_path = project_dir.join("petiglyph.toml");
    write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");

    let config = RuntimeConfig {
        project_dir: project_dir.clone(),
        project_id: "test-threshold-override".to_string(),
        input_dir: project_dir.join("icons"),
        out_dir: project_dir.join("build"),
        font_name: "Petiglyph".to_string(),
        glyph_size: 8,
        base_threshold: 64,
        threshold_overrides: BTreeMap::new(),
        compositions: BTreeMap::new(),
        codepoint_start: 0x10_0000,
    };

    let mut app = App::new(manifest_path.clone(), config);
    app.glyphs.push(InteractiveGlyph {
        glyph: PreprocessedGlyph {
            source_path: project_dir.join("icons/icon.png"),
            source_key: "icon.png".to_string(),
            source_parent_key: "icon.png".to_string(),
            glyph_name: "icon".to_string(),
            width: 4,
            height: 8,
            coverage: vec![0; 32],
            image_fingerprint: "fnv1a64:test".to_string(),
            composition_tile: None,
        },
        saved_threshold: None,
        working_threshold: 64,
    });
    app.selected = 0;
    app.view = AppView::Glyphs;

    handle_key(&mut app, KeyCode::Char('+')).expect("key handling should succeed");
    assert_eq!(app.glyphs[0].working_threshold, 65);
    assert_eq!(app.glyphs[0].saved_threshold, Some(65));
    assert_eq!(app.view, AppView::Glyphs);
    let manifest = read_manifest(&manifest_path).expect("manifest reads");
    assert_eq!(manifest.threshold_overrides.get("icon.png"), Some(&65));

    handle_key(&mut app, KeyCode::Char('r')).expect("key handling should succeed");
    assert_eq!(app.glyphs[0].working_threshold, 64);
    assert_eq!(app.glyphs[0].saved_threshold, None);
    assert_eq!(app.view, AppView::Glyphs);
    let manifest = read_manifest(&manifest_path).expect("manifest reads");
    assert!(!manifest.threshold_overrides.contains_key("icon.png"));

    fs::remove_dir_all(project_dir).expect("temp project dir is removed");
}

#[test]
fn dispatches_press_and_repeat_key_kinds() {
    assert!(should_dispatch_key_kind(KeyEventKind::Press));
    assert!(should_dispatch_key_kind(KeyEventKind::Repeat));
    assert!(!should_dispatch_key_kind(KeyEventKind::Release));
}

#[test]
fn tui_requests_only_escape_disambiguation_keyboard_enhancement() {
    assert_eq!(
        requested_keyboard_enhancement_flags(),
        KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
    );
    assert!(
        !requested_keyboard_enhancement_flags()
            .contains(KeyboardEnhancementFlags::REPORT_EVENT_TYPES)
    );
    assert!(
        !requested_keyboard_enhancement_flags()
            .contains(KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES)
    );
}

#[test]
fn handle_key_supports_keypad_plus_minus_aliases_for_threshold() {
    let project_dir = make_temp_dir("handle-key-keypad-plus-minus");
    let manifest_path = project_dir.join("petiglyph.toml");
    write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");

    let config = RuntimeConfig {
        project_dir: project_dir.clone(),
        project_id: "test-keypad-threshold".to_string(),
        input_dir: project_dir.join("icons"),
        out_dir: project_dir.join("build"),
        font_name: "Petiglyph".to_string(),
        glyph_size: 8,
        base_threshold: 64,
        threshold_overrides: BTreeMap::new(),
        compositions: BTreeMap::new(),
        codepoint_start: 0x10_0000,
    };

    let mut app = App::new(manifest_path.clone(), config);
    app.glyphs.push(InteractiveGlyph {
        glyph: PreprocessedGlyph {
            source_path: project_dir.join("icons/icon.png"),
            source_key: "icon.png".to_string(),
            source_parent_key: "icon.png".to_string(),
            glyph_name: "icon".to_string(),
            width: 4,
            height: 8,
            coverage: vec![0; 32],
            image_fingerprint: "fnv1a64:test".to_string(),
            composition_tile: None,
        },
        saved_threshold: None,
        working_threshold: 64,
    });
    app.selected = 0;
    app.view = AppView::Glyphs;

    handle_key_event_for_test(
        &mut app,
        KeyEvent::new_with_kind_and_state(
            KeyCode::Char('k'),
            KeyModifiers::NONE,
            KeyEventKind::Press,
            KeyEventState::KEYPAD,
        ),
    )
    .expect("keypad plus alias should increment threshold");
    assert_eq!(app.glyphs[0].working_threshold, 65);

    handle_key_event_for_test(
        &mut app,
        KeyEvent::new_with_kind_and_state(
            KeyCode::Char('m'),
            KeyModifiers::NONE,
            KeyEventKind::Press,
            KeyEventState::KEYPAD,
        ),
    )
    .expect("keypad minus alias should decrement threshold");
    assert_eq!(app.glyphs[0].working_threshold, 64);

    fs::remove_dir_all(project_dir).expect("temp project dir is removed");
}

#[test]
fn tab_cycles_panels_and_glyph_controls_stay_in_glyph_view() {
    let project_dir = make_temp_dir("handle-key-tab-cycle");
    let manifest_path = project_dir.join("petiglyph.toml");
    write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");

    let config = RuntimeConfig {
        project_dir: project_dir.clone(),
        project_id: "test-tab-cycle".to_string(),
        input_dir: project_dir.join("icons"),
        out_dir: project_dir.join("build"),
        font_name: "Petiglyph".to_string(),
        glyph_size: 8,
        base_threshold: 64,
        threshold_overrides: BTreeMap::new(),
        compositions: BTreeMap::new(),
        codepoint_start: 0x10_0000,
    };

    let mut app = App::new(manifest_path.clone(), config);
    app.glyphs.push(InteractiveGlyph {
        glyph: PreprocessedGlyph {
            source_path: project_dir.join("icons/icon.png"),
            source_key: "icon.png".to_string(),
            source_parent_key: "icon.png".to_string(),
            glyph_name: "icon".to_string(),
            width: 4,
            height: 8,
            coverage: vec![0; 32],
            image_fingerprint: "fnv1a64:test".to_string(),
            composition_tile: None,
        },
        saved_threshold: None,
        working_threshold: 64,
    });

    // Glyph-specific keys do nothing outside Glyphs view.
    assert_eq!(app.view, AppView::Welcome);
    handle_key(&mut app, KeyCode::Right).expect("key handling should succeed");
    assert_eq!(app.view, AppView::Welcome);
    assert_eq!(app.glyphs[0].working_threshold, 64);

    // Tab is now the primary panel navigation key.
    handle_key(&mut app, KeyCode::Tab).expect("key handling should succeed");
    assert_eq!(app.view, AppView::Glyphs);
    handle_key(&mut app, KeyCode::Tab).expect("key handling should succeed");
    assert_eq!(app.view, AppView::Welcome);
    assert_eq!(app.welcome_focus, WelcomeFocus::BuildButton);

    // Numbered panel jump should keep the same quick-rebuild focus behavior.
    handle_key(&mut app, KeyCode::Char('2')).expect("key handling should succeed");
    assert_eq!(app.view, AppView::Glyphs);
    handle_key(&mut app, KeyCode::Char('1')).expect("key handling should succeed");
    assert_eq!(app.view, AppView::Welcome);
    assert_eq!(app.welcome_focus, WelcomeFocus::BuildButton);

    // Shift+Tab should also return to Home and restore Build/Rebuild focus.
    app.welcome_focus = WelcomeFocus::CreateInput;
    handle_key(&mut app, KeyCode::BackTab).expect("key handling should succeed");
    assert_eq!(app.view, AppView::Glyphs);
    handle_key(&mut app, KeyCode::BackTab).expect("key handling should succeed");
    assert_eq!(app.view, AppView::Welcome);
    assert_eq!(app.welcome_focus, WelcomeFocus::InstalledFontList);

    // Threshold arrows still provide granular (+/-1) changes in Glyphs.
    app.view = AppView::Glyphs;
    handle_key(&mut app, KeyCode::Right).expect("key handling should succeed");
    assert_eq!(app.glyphs[0].working_threshold, 65);
    handle_key(&mut app, KeyCode::Left).expect("key handling should succeed");
    assert_eq!(app.glyphs[0].working_threshold, 64);

    fs::remove_dir_all(project_dir).expect("temp project dir is removed");
}

#[test]
fn dropped_image_on_home_imports_and_switches_to_glyphs() {
    let project_dir = make_temp_dir("drop-home-switch");
    let manifest_path = project_dir.join("petiglyph.toml");
    write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");

    let icons_dir = project_dir.join("icons");
    fs::create_dir_all(&icons_dir).expect("icons dir is created");
    let dropped_image = project_dir.join("drop source.png");
    write_test_png(&dropped_image);

    let config = RuntimeConfig {
        project_dir: project_dir.clone(),
        project_id: "test-drop-home-switch".to_string(),
        input_dir: icons_dir.clone(),
        out_dir: project_dir.join("build"),
        font_name: "Petiglyph".to_string(),
        glyph_size: 8,
        base_threshold: 64,
        threshold_overrides: BTreeMap::new(),
        compositions: BTreeMap::new(),
        codepoint_start: 0x10_0000,
    };

    let mut app = App::new(manifest_path, config);
    app.view = AppView::Welcome;

    handle_paste_event_for_test(&mut app, &format!("'{}'", dropped_image.display()))
        .expect("drop import should succeed");

    assert_eq!(app.view, AppView::Glyphs);
    assert_eq!(app.glyphs.len(), 1);
    assert!(
        icons_dir.join("drop source.png").is_file(),
        "dropped image should be copied into icons directory"
    );
    assert!(
        app.status
            .as_deref()
            .is_some_and(|value| value.contains("drop import: 1 added")),
        "status should report import success, got {:?}",
        app.status
    );

    fs::remove_dir_all(project_dir).expect("temp project dir is removed");
}

#[test]
fn dropped_image_with_conflicting_name_is_renamed_without_overwrite() {
    let project_dir = make_temp_dir("drop-rename-collision");
    let manifest_path = project_dir.join("petiglyph.toml");
    write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");

    let icons_dir = project_dir.join("icons");
    fs::create_dir_all(&icons_dir).expect("icons dir is created");
    let existing = icons_dir.join("icon.png");
    write_test_png(&existing);
    let existing_bytes = fs::read(&existing).expect("existing icon bytes are readable");

    let import_dir = project_dir.join("imports");
    fs::create_dir_all(&import_dir).expect("imports dir is created");
    let dropped = import_dir.join("icon.png");
    write_test_png(&dropped);

    let config = RuntimeConfig {
        project_dir: project_dir.clone(),
        project_id: "test-drop-rename-collision".to_string(),
        input_dir: icons_dir.clone(),
        out_dir: project_dir.join("build"),
        font_name: "Petiglyph".to_string(),
        glyph_size: 8,
        base_threshold: 64,
        threshold_overrides: BTreeMap::new(),
        compositions: BTreeMap::new(),
        codepoint_start: 0x10_0000,
    };

    let mut app = App::new(manifest_path, config);
    app.view = AppView::Glyphs;

    handle_paste_event_for_test(&mut app, &dropped.display().to_string())
        .expect("drop import should succeed");

    assert!(
        icons_dir.join("icon-1.png").is_file(),
        "a renamed copy should be created when filename collides"
    );
    assert_eq!(
        fs::read(&existing).expect("original icon bytes are readable"),
        existing_bytes,
        "existing icon should not be overwritten"
    );
    assert_eq!(app.glyphs.len(), 2);
    assert!(
        app.status
            .as_deref()
            .is_some_and(|value| value.contains("1 renamed")),
        "status should report renamed import, got {:?}",
        app.status
    );

    fs::remove_dir_all(project_dir).expect("temp project dir is removed");
}

#[test]
fn handle_key_w_is_not_a_navigation_path() {
    let workspace = make_temp_dir("handle-key-welcome-nav");
    let project_a = workspace.join("project-a");
    let project_b = workspace.join("project-b");
    fs::create_dir_all(&project_a).expect("project-a dir is created");
    fs::create_dir_all(&project_b).expect("project-b dir is created");

    let manifest_a = project_a.join("petiglyph.toml");
    write_manifest(&manifest_a, &Manifest::default()).expect("manifest-a is written");
    write_manifest(&project_b.join("petiglyph.toml"), &Manifest::default())
        .expect("manifest-b is written");

    let config = RuntimeConfig {
        project_dir: project_a.clone(),
        project_id: "test-workspace-switch".to_string(),
        input_dir: project_a.join("icons"),
        out_dir: project_a.join("build"),
        font_name: "Petiglyph".to_string(),
        glyph_size: 8,
        base_threshold: 64,
        threshold_overrides: BTreeMap::new(),
        compositions: BTreeMap::new(),
        codepoint_start: 0x10_0000,
    };

    let mut app = App::new_with_overrides(
        manifest_a,
        config,
        TuiLaunchOverrides::default(),
        Some(workspace.clone()),
    );
    assert!(!app.quit, "app should start active");

    handle_key(&mut app, KeyCode::Char('w')).expect("key handling should succeed");
    assert!(
        !app.quit,
        "w should not switch out to a separate welcome runtime"
    );
    assert_eq!(app.view, AppView::Welcome);

    fs::remove_dir_all(workspace).expect("temp workspace is removed");
}

#[test]
fn build_shortcut_rebuilds_and_clears_previous_outputs() {
    let project_dir = make_temp_dir("font-action-buttons");
    let manifest_path = project_dir.join("petiglyph.toml");
    write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");

    let icons_dir = project_dir.join("icons");
    fs::create_dir_all(&icons_dir).expect("icons dir is created");
    write_test_png(&icons_dir.join("icon.png"));

    let config = RuntimeConfig {
        project_dir: project_dir.clone(),
        project_id: "test-build-task-visible".to_string(),
        input_dir: icons_dir,
        out_dir: project_dir.join("build"),
        font_name: "Petiglyph".to_string(),
        glyph_size: 8,
        base_threshold: 64,
        threshold_overrides: BTreeMap::new(),
        compositions: BTreeMap::new(),
        codepoint_start: 0x10_0000,
    };

    let mut app = App::new(manifest_path, config);

    handle_key(&mut app, KeyCode::Char('b')).expect("key handling should succeed");
    assert!(
        app.background_task_in_progress(),
        "build should remain visible as a background task briefly"
    );
    wait_for_background_tasks(&mut app);
    let summary = app
        .last_build
        .as_ref()
        .expect("build shortcut should build");
    assert!(summary.ttf_path.is_file(), "ttf output should exist");
    assert!(summary.bdf_path.is_file(), "bdf output should exist");
    let stale_preview = summary.previews_dir.join("stale.png");
    fs::write(&stale_preview, b"stale").expect("stale preview is written");

    handle_key(&mut app, KeyCode::Char('b')).expect("rebuild shortcut should succeed");
    assert!(
        app.background_task_in_progress(),
        "rebuild should remain visible as a background task briefly"
    );
    wait_for_background_tasks(&mut app);
    let rebuilt = app
        .last_build
        .as_ref()
        .expect("rebuild should refresh build state");
    assert!(
        rebuilt.ttf_path.is_file(),
        "rebuilt ttf output should exist"
    );
    assert!(
        rebuilt.bdf_path.is_file(),
        "rebuilt bdf output should exist"
    );
    assert!(
        !stale_preview.exists(),
        "rebuild should clear stale files from the previous build output"
    );
    assert!(
        app.status
            .as_deref()
            .is_some_and(|status| status.contains("rebuild complete")),
        "status should reflect rebuild, got {:?}",
        app.status
    );
    assert_eq!(app.view, AppView::Welcome);

    fs::remove_dir_all(project_dir).expect("temp project dir is removed");
}

#[test]
fn tui_launch_overrides_persist_through_reload_and_build() {
    let project_dir = make_temp_dir("tui-overrides");
    let manifest_path = project_dir.join("petiglyph.toml");
    let default_icons = project_dir.join("icons");
    let override_icons = project_dir.join("icons-override");
    let build_dir = project_dir.join("build");

    fs::create_dir_all(&default_icons).expect("default icons dir is created");
    fs::create_dir_all(&override_icons).expect("override icons dir is created");
    fs::create_dir_all(&build_dir).expect("build dir is created");
    write_test_png(&default_icons.join("default.png"));
    write_test_png(&override_icons.join("override.png"));

    let manifest = Manifest {
        input_dir: "icons".to_string(),
        out_dir: "build".to_string(),
        font_name: "Petiglyph".to_string(),
        glyph_size: 8,
        threshold: 10,
        codepoint_start: "U+100000".to_string(),
        project_id: Some("test-tui-overrides".to_string()),
        threshold_overrides: BTreeMap::new(),
        compositions: BTreeMap::new(),
    };
    write_manifest(&manifest_path, &manifest).expect("manifest is written");

    let overrides = TuiLaunchOverrides {
        input_dir: Some(override_icons.clone()),
        threshold: Some(201),
        glyph_size: Some(16),
        codepoint_start: Some("U+E100".to_string()),
    };

    let config = load_runtime_config(
        &manifest_path,
        overrides.input_dir.clone(),
        None,
        overrides.threshold,
        overrides.glyph_size,
        overrides.codepoint_start.clone(),
    )
    .expect("runtime config with overrides should load");

    let mut app = App::new_with_overrides(manifest_path.clone(), config, overrides.clone(), None);

    let mut modified_manifest = read_manifest(&manifest_path).expect("manifest reads");
    modified_manifest.threshold = 1;
    modified_manifest.glyph_size = 4;
    modified_manifest.codepoint_start = "U+100010".to_string();
    write_manifest(&manifest_path, &modified_manifest).expect("modified manifest is written");

    handle_key(&mut app, KeyCode::Char('R')).expect("rescan should succeed");
    assert_eq!(app.config.input_dir, override_icons);
    assert_eq!(app.config.base_threshold, 201);
    assert_eq!(app.config.glyph_size, 16);
    assert_eq!(
        app.config.codepoint_start,
        parse_codepoint("U+E100").expect("override codepoint parses")
    );
    assert_eq!(app.glyphs.len(), 1);
    assert_eq!(app.glyphs[0].glyph.source_key, "override.png");

    app.view = AppView::Welcome;
    app.welcome_focus = WelcomeFocus::BuildButton;
    handle_key(&mut app, KeyCode::Enter).expect("enter should run build action");
    wait_for_background_tasks(&mut app);
    let summary = app
        .last_build
        .as_ref()
        .expect("build summary should be present");
    let bdf = fs::read_to_string(&summary.bdf_path).expect("bdf is written");
    assert!(
        bdf.contains("SIZE 16 75 75"),
        "build should honor glyph_size override; bdf={bdf}"
    );

    fs::remove_dir_all(project_dir).expect("temp project dir is removed");
}
