use crossterm::event::KeyCode;
use image::{Rgba, RgbaImage};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::build::{
    GlyphBitmap, MappingEntry, PreprocessedGlyph, bitmap_to_bdf_rows, build_outputs,
    collect_source_files, coverage_map, glyph_sample_string, is_supported_source,
};
use crate::install::{FontInstallNameMode, effective_font_name};
use crate::project::{Manifest, RuntimeConfig, parse_codepoint, read_manifest, write_manifest};
use crate::tui::{App, AppView, InteractiveGlyph, handle_key, persist_threshold_override};

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
fn glyph_sample_string_skips_non_scalar_values() {
    let sample = glyph_sample_string(0xDFFF, 2);
    let expected = char::from_u32(0xE000).expect("valid").to_string();
    assert_eq!(sample, expected);
}

#[test]
fn coverage_map_uses_alpha_for_transparent_sources() {
    let mut image = RgbaImage::from_pixel(2, 2, Rgba([255, 255, 255, 0]));
    image.put_pixel(0, 0, Rgba([255, 255, 255, 255]));

    let coverage = coverage_map(&image, 2).expect("coverage map succeeds");

    assert_eq!(coverage[0], 255);
    assert_eq!(coverage[1], 0);
    assert_eq!(coverage[2], 0);
    assert_eq!(coverage[3], 0);
}

#[test]
fn coverage_map_detects_foreground_on_opaque_background() {
    let mut image = RgbaImage::from_pixel(3, 3, Rgba([0, 0, 0, 255]));
    image.put_pixel(1, 1, Rgba([255, 255, 255, 255]));

    let coverage = coverage_map(&image, 3).expect("coverage map succeeds");

    assert_eq!(coverage[0], 0);
    assert_eq!(coverage[4], 255);
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
    assert_eq!(app.view, AppView::Home);
    handle_key(&mut app, KeyCode::Right).expect("key handling should succeed");
    assert_eq!(app.view, AppView::Home);
    assert_eq!(app.glyphs[0].working_threshold, 64);

    // Tab is now the primary panel navigation key.
    handle_key(&mut app, KeyCode::Tab).expect("key handling should succeed");
    assert_eq!(app.view, AppView::Glyphs);
    handle_key(&mut app, KeyCode::Tab).expect("key handling should succeed");
    assert_eq!(app.view, AppView::Font);
    handle_key(&mut app, KeyCode::Tab).expect("key handling should succeed");
    assert_eq!(app.view, AppView::Home);

    // Threshold arrows still provide granular (+/-1) changes in Glyphs.
    app.view = AppView::Glyphs;
    handle_key(&mut app, KeyCode::Right).expect("key handling should succeed");
    assert_eq!(app.glyphs[0].working_threshold, 65);
    handle_key(&mut app, KeyCode::Left).expect("key handling should succeed");
    assert_eq!(app.glyphs[0].working_threshold, 64);

    fs::remove_dir_all(project_dir).expect("temp project dir is removed");
}
