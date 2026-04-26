use crossterm::event::KeyCode;
use image::{Rgba, RgbaImage};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::build::{
    GlyphBitmap, MappingEntry, PreprocessedGlyph, bitmap_to_bdf_rows, build_outputs,
    collect_source_files, coverage_map, glyph_sample_string, is_supported_source,
};
use crate::cli::{DefaultTuiTarget, resolve_default_tui_target_for};
use crate::install::{
    FontInstallNameMode, effective_font_name, expected_install_ttf_path_for_mode,
};
use crate::project::{
    Manifest, RuntimeConfig, auto_detect_manifest_path, discover_project_manifests,
    load_runtime_config, parse_codepoint, read_manifest, write_manifest,
};
use crate::tui::{
    App, AppView, FontAction, InteractiveGlyph, TuiLaunchOverrides, WelcomeFocus,
    format_welcome_input_field, handle_key, persist_threshold_override,
    resolve_installed_font_path_with, sample_glyphs_from_ttf_bytes, spaced_sample,
    switch_notice_visible, wrap_sample_for_display,
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

fn write_test_png(path: &Path) {
    let mut img = RgbaImage::from_pixel(8, 8, Rgba([255, 255, 255, 0]));
    img.put_pixel(2, 2, Rgba([0, 0, 0, 255]));
    img.put_pixel(5, 5, Rgba([0, 0, 0, 255]));
    img.save(path).expect("test image is written");
}

fn nonzero_coverage_bounds(coverage: &[u8], size: u32) -> Option<(u32, u32, u32, u32)> {
    if size == 0 {
        return None;
    }

    let mut min_x = size;
    let mut min_y = size;
    let mut max_x = 0u32;
    let mut max_y = 0u32;
    let mut found = false;

    for (idx, value) in coverage.iter().enumerate() {
        if *value == 0 {
            continue;
        }

        found = true;
        let x = (idx as u32) % size;
        let y = (idx as u32) / size;
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }

    found.then_some((min_x, min_y, max_x, max_y))
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

    let resolved = resolve_installed_font_path_with(&manifest_path, "My Font", |path| {
        path == prefixed_path.as_path()
    });

    assert_ne!(plain_path, prefixed_path);
    assert_eq!(resolved.as_deref(), Some(prefixed_path.as_path()));

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
    let app = App::new_workspace(workspace.clone(), None, TuiLaunchOverrides::default())
        .expect("workspace TUI app initializes");

    assert_eq!(app.view, AppView::Welcome);
    assert_eq!(app.active_project, None);
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
fn unified_tui_multiple_projects_keep_project_card_informational() {
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
    assert_eq!(app.welcome_focus, WelcomeFocus::CreateInput);

    handle_key(&mut app, KeyCode::Down).expect("welcome arrows move to create button");
    assert_eq!(app.welcome_focus, WelcomeFocus::CreateButton);

    handle_key(&mut app, KeyCode::Up).expect("welcome arrows move to input");
    assert_eq!(app.welcome_focus, WelcomeFocus::CreateInput);
    handle_key(&mut app, KeyCode::Enter).expect("enter on input moves to create button");

    assert_eq!(app.active_project, None);
    assert!(app.glyphs.is_empty());
    assert!(app.switch_notice.is_none());

    fs::remove_dir_all(workspace).expect("temp workspace is removed");
}

#[test]
fn welcome_input_edit_mode_types_hjkl_without_navigation() {
    let workspace = make_temp_dir("welcome-input-edit-mode");
    let mut app = App::new_workspace(workspace.clone(), None, TuiLaunchOverrides::default())
        .expect("workspace TUI app initializes");

    assert_eq!(app.view, AppView::Welcome);
    assert_eq!(app.welcome_focus, WelcomeFocus::CreateInput);
    assert!(!app.welcome_input_editing);

    handle_key(&mut app, KeyCode::Char('l')).expect("welcome navigation works when not editing");
    assert_eq!(app.welcome_focus, WelcomeFocus::CreateButton);
    assert!(app.create_input.value().is_empty());

    handle_key(&mut app, KeyCode::Left).expect("left arrow returns focus to input");
    assert_eq!(app.welcome_focus, WelcomeFocus::CreateInput);

    handle_key(&mut app, KeyCode::Enter).expect("enter starts typing mode");
    assert!(app.welcome_input_editing);
    assert_eq!(app.welcome_focus, WelcomeFocus::CreateInput);

    for ch in ['l', 'h', 'j', 'k', '2', '3'] {
        handle_key(&mut app, KeyCode::Char(ch)).expect("typing should append character");
    }
    assert_eq!(app.create_input.value(), "lhjk23");
    assert_eq!(app.welcome_focus, WelcomeFocus::CreateInput);

    handle_key(&mut app, KeyCode::Enter).expect("enter exits typing mode");
    assert!(!app.welcome_input_editing);
    assert_eq!(app.welcome_focus, WelcomeFocus::CreateButton);

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

    app.view = AppView::Font;
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
fn spaced_sample_separates_glyphs_for_readability() {
    assert_eq!(spaced_sample("ABC"), "A  B  C");
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
        size: 8,
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
        input_dir: PathBuf::from("icons"),
        out_dir: out_dir.clone(),
        font_name: "Petiglyph".to_string(),
        glyph_size: 64,
        base_threshold: 64,
        threshold_overrides: BTreeMap::new(),
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
    assert!(face.glyph_index(' ').is_some(), "space glyph should exist");

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
        input_dir,
        out_dir,
        font_name: "SampleLimit".to_string(),
        glyph_size: 16,
        base_threshold: 64,
        threshold_overrides: BTreeMap::new(),
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
        input_dir,
        out_dir: out_dir.clone(),
        font_name: "Edge".to_string(),
        glyph_size: 32,
        base_threshold: 64,
        threshold_overrides: BTreeMap::new(),
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
        input_dir,
        out_dir,
        font_name: "Overflow".to_string(),
        glyph_size: 32,
        base_threshold: 64,
        threshold_overrides: BTreeMap::new(),
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
        input_dir,
        out_dir,
        font_name: "Surrogate".to_string(),
        glyph_size: 32,
        base_threshold: 64,
        threshold_overrides: BTreeMap::new(),
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
        input_dir: project_dir.join("icons"),
        out_dir: build_dir.clone(),
        font_name: "Petiglyph".to_string(),
        glyph_size: 8,
        base_threshold: 64,
        threshold_overrides: BTreeMap::new(),
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
        input_dir: project_dir.join("icons"),
        out_dir: project_dir.join("build"),
        font_name: "Petiglyph".to_string(),
        glyph_size: 8,
        base_threshold: 64,
        threshold_overrides: BTreeMap::new(),
        codepoint_start: 0x10_0000,
    };

    let mut app = App::new(manifest_path.clone(), config);
    app.glyphs.push(InteractiveGlyph {
        glyph: PreprocessedGlyph {
            source_path: project_dir.join("icons/icon.png"),
            source_key: "icon.png".to_string(),
            glyph_name: "icon".to_string(),
            size: 8,
            coverage: vec![0; 64],
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
fn tab_cycles_panels_and_glyph_controls_stay_in_glyph_view() {
    let project_dir = make_temp_dir("handle-key-tab-cycle");
    let manifest_path = project_dir.join("petiglyph.toml");
    write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");

    let config = RuntimeConfig {
        input_dir: project_dir.join("icons"),
        out_dir: project_dir.join("build"),
        font_name: "Petiglyph".to_string(),
        glyph_size: 8,
        base_threshold: 64,
        threshold_overrides: BTreeMap::new(),
        codepoint_start: 0x10_0000,
    };

    let mut app = App::new(manifest_path.clone(), config);
    app.glyphs.push(InteractiveGlyph {
        glyph: PreprocessedGlyph {
            source_path: project_dir.join("icons/icon.png"),
            source_key: "icon.png".to_string(),
            glyph_name: "icon".to_string(),
            size: 8,
            coverage: vec![0; 64],
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
    assert_eq!(app.view, AppView::Font);
    handle_key(&mut app, KeyCode::Tab).expect("key handling should succeed");
    assert_eq!(app.view, AppView::Welcome);

    // Threshold arrows still provide granular (+/-1) changes in Glyphs.
    app.view = AppView::Glyphs;
    handle_key(&mut app, KeyCode::Right).expect("key handling should succeed");
    assert_eq!(app.glyphs[0].working_threshold, 65);
    handle_key(&mut app, KeyCode::Left).expect("key handling should succeed");
    assert_eq!(app.glyphs[0].working_threshold, 64);

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
        input_dir: project_a.join("icons"),
        out_dir: project_a.join("build"),
        font_name: "Petiglyph".to_string(),
        glyph_size: 8,
        base_threshold: 64,
        threshold_overrides: BTreeMap::new(),
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
fn font_action_buttons_switch_with_arrows_and_enter_builds() {
    let project_dir = make_temp_dir("font-action-buttons");
    let manifest_path = project_dir.join("petiglyph.toml");
    write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");

    let icons_dir = project_dir.join("icons");
    fs::create_dir_all(&icons_dir).expect("icons dir is created");
    write_test_png(&icons_dir.join("icon.png"));

    let config = RuntimeConfig {
        input_dir: icons_dir,
        out_dir: project_dir.join("build"),
        font_name: "Petiglyph".to_string(),
        glyph_size: 8,
        base_threshold: 64,
        threshold_overrides: BTreeMap::new(),
        codepoint_start: 0x10_0000,
    };

    let mut app = App::new(manifest_path, config);
    app.view = AppView::Font;

    assert_eq!(app.selected_font_action, FontAction::Build);
    handle_key(&mut app, KeyCode::Right).expect("key handling should succeed");
    assert_eq!(app.selected_font_action, FontAction::Install);
    handle_key(&mut app, KeyCode::Left).expect("key handling should succeed");
    assert_eq!(app.selected_font_action, FontAction::Build);

    handle_key(&mut app, KeyCode::Enter).expect("key handling should succeed");
    let summary = app
        .last_build
        .as_ref()
        .expect("enter on build action should build");
    assert!(summary.ttf_path.is_file(), "ttf output should exist");
    assert!(summary.bdf_path.is_file(), "bdf output should exist");
    assert_eq!(app.view, AppView::Font);

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
        threshold_overrides: BTreeMap::new(),
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

    app.view = AppView::Font;
    handle_key(&mut app, KeyCode::Enter).expect("enter should run build action");
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
