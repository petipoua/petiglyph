    use super::{
        AnimationConfig, AnimationConfigFocus, AnimationImportSettingsFocus,
        AnimationImportSettingsState, AnimationImportTaskOutput, AnimationPreview, AnimationType,
        App, AppView, BleedLevel, DropImportResult, ExistingImportPolicy, GlyphPreviewControl,
        GlyphToolMode, HomeCreationKind, HomeWorkflow, InteractiveGlyph, KeyCode, KeyEvent,
        RuntimeConfig, TuiLaunchOverrides, VisibleGlyphRow, animation_frame_source_for_preview,
        animation_has_non_uniform_frame_invert, animation_has_non_uniform_frame_thresholds,
        collect_dropped_paths, composition_preview_lines_stable_frame,
        continue_home_workflow_after_tweaking, default_animation_name_from_frames,
        drag_images_here_lines, emitted_composition_cols, glyph_matches_animation_frame_source,
        glyph_tweak_continue_label, glyph_tweak_progress_label, grayscale_options_are_default,
        handle_key, handle_key_event_for_test, handle_paste_event_for_test,
        home_workflow_preview_lines, import_image_files_to_input, import_settings_visible_focuses,
        installed_animation_blocks_for_definition, installed_animation_frame_index,
        installed_animation_source_block, move_import_settings_focus,
        persist_composition_definition, preview_leftmost_control, preview_lines,
        prune_static_sample_blocks, resolve_command_path, scrollbar_thumb_geometry,
        selected_threshold_sources, split_shell_like_tokens, step_animation_preview,
        visible_window_bounds,
    };
    use crate::animation_media;
    use crate::build::{CompositionTileInfo, PreprocessedGlyph};
    use crate::image_pipeline::{SourceFitMode, coverage_map_from_image_with_fit};
    use crate::project::{AnimationDef, CompositionDef, Manifest, read_manifest, write_manifest};
    use anyhow::anyhow;
    use image::{Rgb, RgbImage, Rgba, RgbaImage};
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    fn make_temp_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time is valid")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("petiglyph-tui-{name}-{nonce}"));
        fs::create_dir_all(&dir).expect("temp dir is created");
        dir
    }

    fn drain_background_tasks(app: &mut App) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while app.background_task_in_progress() && Instant::now() < deadline {
            app.poll_background_tasks_for_test();
            std::thread::sleep(Duration::from_millis(10));
        }
        app.poll_background_tasks_for_test();
        assert!(
            !app.background_task_in_progress(),
            "background task should complete before test continues; status={:?}",
            app.status
        );
    }

    fn write_test_png(path: &std::path::Path) {
        let mut img = RgbaImage::from_pixel(8, 8, Rgba([255, 255, 255, 0]));
        for y in 2..6 {
            for x in 2..6 {
                img.put_pixel(x, y, Rgba([0, 0, 0, 255]));
            }
        }
        img.save(path).expect("test png is written");
    }

    fn write_test_jpg(path: &std::path::Path) {
        let mut img = RgbImage::from_pixel(8, 8, Rgb([255, 255, 255]));
        for y in 2..6 {
            for x in 2..6 {
                img.put_pixel(x, y, Rgb([0, 0, 0]));
            }
        }
        img.save(path).expect("test jpg is written");
    }

    fn write_test_avif_with_ffmpeg(path: &std::path::Path) -> bool {
        let source_png = path.with_file_name(".petiglyph-test-avif-source.png");
        write_test_png(&source_png);

        let Some(ffmpeg) = resolve_command_path("ffmpeg") else {
            let _ = fs::remove_file(&source_png);
            return false;
        };

        let output = Command::new(ffmpeg)
            .arg("-v")
            .arg("error")
            .arg("-y")
            .arg("-i")
            .arg(&source_png)
            .arg("-frames:v")
            .arg("1")
            .arg(path)
            .output();
        let _ = fs::remove_file(&source_png);

        output.is_ok_and(|output| output.status.success())
    }

    fn write_test_svg(path: &std::path::Path) {
        fs::write(
            path,
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8" viewBox="0 0 8 8"><rect width="8" height="8" fill="none"/><rect x="2" y="2" width="4" height="4" fill="black"/></svg>"#,
        )
        .expect("test svg is written");
    }

    #[test]
    fn first_install_notice_must_be_dismissed_before_global_shortcuts_resume() {
        let project_dir = make_temp_dir("first-install-notice");
        let manifest_path = project_dir.join("petiglyph.toml");
        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-first-install-popup".to_string(),
            input_dir: project_dir.join("icons"),
            out_dir: project_dir.join("build"),
            font_name: "Petiglyph".to_string(),
            glyph_size: 8,
            base_threshold: 64,
            threshold_overrides: BTreeMap::new(),
            invert_overrides: BTreeMap::new(),
            compositions: BTreeMap::new(),
            animations: Vec::new(),
            codepoint_start: 0x10_0000,
        };

        let mut app = App::new(manifest_path, config);
        app.view = AppView::Welcome;
        app.first_install_notice_open = true;

        handle_key(&mut app, KeyCode::Char('q')).expect("popup should intercept quit");
        assert!(!app.quit, "quit should not fire while popup is open");
        assert!(
            app.first_install_notice_open,
            "non-dismiss keys should keep popup open"
        );

        handle_key(&mut app, KeyCode::Char(' ')).expect("space should dismiss popup");
        assert!(
            !app.first_install_notice_open,
            "space should close first-install popup"
        );

        handle_key(&mut app, KeyCode::Char('q')).expect("quit should work once popup is closed");
        assert!(app.quit, "quit should resume after popup dismissal");

        fs::remove_dir_all(project_dir).expect("temp dir is removed");
    }

    #[test]
    fn verbose_paths_toggle_switches_with_v_shortcut() {
        let project_dir = make_temp_dir("verbose-toggle");
        let manifest_path = project_dir.join("petiglyph.toml");
        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-verbose-toggle".to_string(),
            input_dir: project_dir.join("icons"),
            out_dir: project_dir.join("build"),
            font_name: "Petiglyph".to_string(),
            glyph_size: 8,
            base_threshold: 64,
            threshold_overrides: BTreeMap::new(),
            invert_overrides: BTreeMap::new(),
            compositions: BTreeMap::new(),
            animations: Vec::new(),
            codepoint_start: 0x10_0000,
        };

        let mut app = App::new(manifest_path, config);
        assert!(!app.verbose_paths, "verbose paths should default to off");

        handle_key(&mut app, KeyCode::Char('v')).expect("v should toggle verbose paths on");
        assert!(app.verbose_paths, "verbose paths should toggle on");

        handle_key(&mut app, KeyCode::Char('V')).expect("V should toggle verbose paths off");
        assert!(!app.verbose_paths, "verbose paths should toggle off");

        fs::remove_dir_all(project_dir).expect("temp dir is removed");
    }

    #[test]
    fn installed_animation_preview_advances_one_frame_at_a_time() {
        let started_at = Instant::now();

        assert_eq!(
            installed_animation_frame_index(4, 3, started_at, started_at),
            0
        );
        assert_eq!(
            installed_animation_frame_index(
                4,
                3,
                started_at,
                started_at + Duration::from_millis(250),
            ),
            1
        );
        assert_eq!(
            installed_animation_frame_index(
                4,
                3,
                started_at,
                started_at + Duration::from_millis(500),
            ),
            2
        );
        assert_eq!(
            installed_animation_frame_index(
                4,
                3,
                started_at,
                started_at + Duration::from_millis(750),
            ),
            0
        );
    }

    #[test]
    fn animation_preview_step_uses_accumulated_timing_without_threshold_jump() {
        let animation = AnimationDef {
            name: "run".to_string(),
            animation_type: AnimationType::Standard,
            fps: 20,
            frames: vec![
                "a.png".to_string(),
                "b.png".to_string(),
                "c.png".to_string(),
            ],
            rows: None,
            cols: None,
            horizontal_bleed: None,
            vertical_bleed: None,
            grayscale_processing: None,
        };
        let started_at = Instant::now();
        let mut preview = AnimationPreview {
            animation_name: animation.name.clone(),
            frame_index: 0,
            last_frame_at: started_at,
        };

        // Simulate a render cadence (~48ms) where old logic could alias into abrupt speed shifts.
        for tick in [48u64, 96, 144, 192, 240, 288, 336, 384, 432, 480] {
            step_animation_preview(
                &mut preview,
                &animation,
                started_at + Duration::from_millis(tick),
            );
        }
        assert_eq!(
            preview.frame_index, 0,
            "20fps over 480ms should advance exactly 9 frames (mod 3 -> 0) with accumulator timing"
        );

        let mut faster = animation.clone();
        faster.fps = 21;
        let mut faster_preview = AnimationPreview {
            animation_name: faster.name.clone(),
            frame_index: 0,
            last_frame_at: started_at,
        };
        for tick in [48u64, 96, 144, 192, 240, 288, 336, 384, 432, 480] {
            step_animation_preview(
                &mut faster_preview,
                &faster,
                started_at + Duration::from_millis(tick),
            );
        }
        assert_eq!(
            faster_preview.frame_index, 1,
            "21fps over 480ms should only be one frame ahead of 20fps in this window"
        );
    }

    #[test]
    fn installed_animation_source_block_remaps_unambiguous_compose_row_col() {
        let by_source = BTreeMap::from([
            (
                "strip.png#compose:1x4:0:0".to_string(),
                "U+100000".to_string(),
            ),
            (
                "strip.png#compose:1x4:0:1".to_string(),
                "U+100001".to_string(),
            ),
        ]);
        let block0 = installed_animation_source_block(&by_source, "strip.png#compose:1x4:0:0");
        let block1 = installed_animation_source_block(&by_source, "strip.png#compose:1x4:0:1");

        assert_eq!(block0, Some(char::from_u32(0x100000).unwrap().to_string()));
        assert_eq!(block1, Some(char::from_u32(0x100001).unwrap().to_string()));
    }

    #[test]
    fn installed_grid_animation_blocks_use_emitted_composition_columns() {
        let by_source = BTreeMap::from([
            (
                "strip.png#compose:2x4:0:0".to_string(),
                "U+100000".to_string(),
            ),
            (
                "strip.png#compose:2x4:0:1".to_string(),
                "U+100001".to_string(),
            ),
            (
                "strip.png#compose:2x4:0:2".to_string(),
                "U+100002".to_string(),
            ),
            (
                "strip.png#compose:2x4:0:3".to_string(),
                "U+100003".to_string(),
            ),
            (
                "strip.png#compose:2x4:1:0".to_string(),
                "U+100004".to_string(),
            ),
            (
                "strip.png#compose:2x4:1:1".to_string(),
                "U+100005".to_string(),
            ),
            (
                "strip.png#compose:2x4:1:2".to_string(),
                "U+100006".to_string(),
            ),
            (
                "strip.png#compose:2x4:1:3".to_string(),
                "U+100007".to_string(),
            ),
        ]);
        let animation = AnimationDef {
            name: "walk".to_string(),
            animation_type: AnimationType::Grid,
            fps: 8,
            frames: vec!["strip.png".to_string()],
            rows: Some(2),
            cols: Some(2),
            horizontal_bleed: Some(BleedLevel::Weak),
            vertical_bleed: Some(BleedLevel::Off),
            grayscale_processing: None,
        };

        let blocks = installed_animation_blocks_for_definition(&animation, &by_source);

        assert_eq!(blocks.len(), 1);
        assert_eq!(
            blocks[0],
            format!(
                "{}{}{}{}\n{}{}{}{}",
                char::from_u32(0x100000).unwrap(),
                char::from_u32(0x100001).unwrap(),
                char::from_u32(0x100002).unwrap(),
                char::from_u32(0x100003).unwrap(),
                char::from_u32(0x100004).unwrap(),
                char::from_u32(0x100005).unwrap(),
                char::from_u32(0x100006).unwrap(),
                char::from_u32(0x100007).unwrap()
            )
        );
    }

    #[test]
    fn glyph_matches_animation_frame_source_requires_matching_grid_dims() {
        let glyph = InteractiveGlyph {
            glyph: PreprocessedGlyph {
                source_path: PathBuf::from("icons/strip.png"),
                source_key: "strip.png#compose:1x4:0:1".to_string(),
                source_parent_key: "strip.png".to_string(),
                glyph_name: "strip_r1_c2".to_string(),
                width: 8,
                height: 8,
                coverage: vec![0; 64],
                image_fingerprint: "fnv1a64:test".to_string(),
                composition_tile: None,
            },
            working_threshold: 64,
            saved_threshold: None,
            saved_invert: false,
            working_invert: false,
        };

        assert!(!glyph_matches_animation_frame_source(
            &glyph,
            "strip.png#compose:1x2:0:1"
        ));
        assert!(glyph_matches_animation_frame_source(
            &glyph,
            "strip.png#compose:1x4:0:1"
        ));
    }

    #[test]
    fn installed_animation_source_block_does_not_remap_ambiguous_compose_row_col() {
        let by_source = BTreeMap::from([
            (
                "strip.png#compose:1x4:0:1".to_string(),
                "U+100001".to_string(),
            ),
            (
                "strip.png#compose:1x8:0:1".to_string(),
                "U+1000AA".to_string(),
            ),
        ]);
        let block = installed_animation_source_block(&by_source, "strip.png#compose:1x2:0:1");
        assert_eq!(block, None);
    }

    #[test]
    fn prune_static_sample_blocks_removes_animation_chars_from_dense_blocks() {
        let a = char::from_u32(0x100000).expect("valid char");
        let b = char::from_u32(0x100001).expect("valid char");
        let c = char::from_u32(0x100002).expect("valid char");
        let sample_blocks = vec![format!("{a}{b}{c}")];
        let animation_frames =
            std::iter::once(format!("{b}")).collect::<std::collections::HashSet<_>>();

        let filtered = prune_static_sample_blocks(sample_blocks, &animation_frames);

        assert_eq!(filtered, vec![format!("{a}{c}")]);
    }

    #[test]
    fn prune_static_sample_blocks_drops_exact_animation_frame_blocks() {
        let a = char::from_u32(0x100000).expect("valid char");
        let b = char::from_u32(0x100001).expect("valid char");
        let sample_blocks = vec![format!("{a}"), format!("{b}")];
        let animation_frames =
            std::iter::once(format!("{b}")).collect::<std::collections::HashSet<_>>();

        let filtered = prune_static_sample_blocks(sample_blocks, &animation_frames);

        assert_eq!(filtered, vec![format!("{a}")]);
    }

    #[test]
    fn verbose_paths_toggle_does_not_fire_while_typing_project_name() {
        let project_dir = make_temp_dir("verbose-toggle-input");
        let manifest_path = project_dir.join("petiglyph.toml");
        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-verbose-toggle-input".to_string(),
            input_dir: project_dir.join("icons"),
            out_dir: project_dir.join("build"),
            font_name: "Petiglyph".to_string(),
            glyph_size: 8,
            base_threshold: 64,
            threshold_overrides: BTreeMap::new(),
            invert_overrides: BTreeMap::new(),
            compositions: BTreeMap::new(),
            animations: Vec::new(),
            codepoint_start: 0x10_0000,
        };

        let mut app = App::new(manifest_path, config);
        app.view = AppView::Welcome;
        app.welcome_focus = super::WelcomeFocus::CreateInput;
        app.welcome_input_editing = true;

        handle_key(&mut app, KeyCode::Char('v')).expect("typing should accept v");
        assert!(
            !app.verbose_paths,
            "verbose paths should not toggle during project-name typing"
        );
        assert_eq!(
            app.create_input.value(),
            "v",
            "v should be inserted into input"
        );

        fs::remove_dir_all(project_dir).expect("temp dir is removed");
    }

    #[test]
    fn verbose_paths_toggle_does_not_bypass_delete_confirmation_popup() {
        let project_dir = make_temp_dir("verbose-delete-confirm");
        let manifest_path = project_dir.join("petiglyph.toml");
        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-verbose-delete-confirm".to_string(),
            input_dir: project_dir.join("icons"),
            out_dir: project_dir.join("build"),
            font_name: "Petiglyph".to_string(),
            glyph_size: 8,
            base_threshold: 64,
            threshold_overrides: BTreeMap::new(),
            invert_overrides: BTreeMap::new(),
            compositions: BTreeMap::new(),
            animations: Vec::new(),
            codepoint_start: 0x10_0000,
        };

        let mut app = App::new(manifest_path, config);
        app.view = AppView::Welcome;
        app.delete_project_confirm_selection = Some(0);
        assert!(!app.verbose_paths, "verbose paths should start disabled");

        handle_key(&mut app, KeyCode::Char('v'))
            .expect("delete confirmation should intercept verbose toggle");

        assert!(
            app.delete_project_confirm_selection.is_some(),
            "delete confirmation should stay open"
        );
        assert!(
            !app.verbose_paths,
            "verbose paths should not toggle through delete confirmation"
        );

        fs::remove_dir_all(project_dir).expect("temp dir is removed");
    }

    #[test]
    fn verbose_paths_toggle_is_focusable_with_arrows_and_enter() {
        let project_dir = make_temp_dir("verbose-toggle-focus");
        let manifest_path = project_dir.join("petiglyph.toml");
        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-verbose-toggle-focus".to_string(),
            input_dir: project_dir.join("icons"),
            out_dir: project_dir.join("build"),
            font_name: "Petiglyph".to_string(),
            glyph_size: 8,
            base_threshold: 64,
            threshold_overrides: BTreeMap::new(),
            invert_overrides: BTreeMap::new(),
            compositions: BTreeMap::new(),
            animations: Vec::new(),
            codepoint_start: 0x10_0000,
        };

        let mut app = App::new(manifest_path, config);
        app.view = AppView::Welcome;
        assert!(!app.verbose_paths, "verbose paths should start off");

        for focus in [
            super::WelcomeFocus::BuildButton,
            super::WelcomeFocus::InstallButton,
            super::WelcomeFocus::DeleteProjectButton,
        ] {
            app.welcome_focus = focus;
            handle_key(&mut app, KeyCode::Up)
                .expect("up should move from current-project actions to settings");
            assert_eq!(
                app.welcome_focus,
                super::WelcomeFocus::VerbosePathsToggle,
                "settings toggle should be focusable from current-project actions"
            );
            handle_key(&mut app, KeyCode::Down)
                .expect("down from settings should return to install action");
            assert_eq!(
                app.welcome_focus,
                super::WelcomeFocus::InstallButton,
                "down from settings should land on install (not delete)"
            );
        }

        app.welcome_focus = super::WelcomeFocus::VerbosePathsToggle;
        handle_key(&mut app, KeyCode::Enter).expect("enter should toggle settings row");
        assert!(
            app.verbose_paths,
            "enter on settings should toggle verbose paths"
        );

        fs::remove_dir_all(project_dir).expect("temp dir is removed");
    }

    #[test]
    fn up_from_create_glyph_jumps_to_install_button() {
        let project_dir = make_temp_dir("home-nav-create-glyph-up");
        let manifest_path = project_dir.join("petiglyph.toml");
        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-home-nav-create-glyph-up".to_string(),
            input_dir: project_dir.join("icons"),
            out_dir: project_dir.join("build"),
            font_name: "Petiglyph".to_string(),
            glyph_size: 8,
            base_threshold: 64,
            threshold_overrides: BTreeMap::new(),
            invert_overrides: BTreeMap::new(),
            compositions: BTreeMap::new(),
            animations: Vec::new(),
            codepoint_start: 0x10_0000,
        };

        let mut app = App::new(manifest_path, config);
        app.view = AppView::Welcome;
        app.welcome_focus = super::WelcomeFocus::HomeCreateButtons;
        app.home_launcher_focus = super::HomeLauncherFocus::CreateGlyph;

        handle_key(&mut app, KeyCode::Up).expect("up should navigate to install/reinstall");
        assert_eq!(
            app.welcome_focus,
            super::WelcomeFocus::InstallButton,
            "up from create glyph should jump to install/reinstall button"
        );

        fs::remove_dir_all(project_dir).expect("temp dir is removed");
    }

    #[test]
    fn glyphs_view_install_shortcut_routes_to_install_action() {
        let workspace_root = make_temp_dir("glyphs-shortcut-install");
        let mut app = App::new_inactive(workspace_root.clone(), TuiLaunchOverrides::default());
        app.view = AppView::Glyphs;

        handle_key(&mut app, KeyCode::Char('i'))
            .expect("glyphs install shortcut should route to install action");

        assert_eq!(
            app.status.as_deref(),
            Some("create a project in Home or relaunch with --manifest before installing")
        );

        fs::remove_dir_all(workspace_root).expect("temp dir is removed");
    }

    #[test]
    fn glyphs_view_rescan_shortcut_reloads_project_sources() {
        let project_dir = make_temp_dir("glyphs-shortcut-rescan");
        let manifest_path = project_dir.join("petiglyph.toml");
        let icons_dir = project_dir.join("icons");
        fs::create_dir_all(&icons_dir).expect("icons dir is created");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");
        write_test_png(&icons_dir.join("one.png"));

        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-glyphs-shortcut-rescan".to_string(),
            input_dir: icons_dir.clone(),
            out_dir: project_dir.join("build"),
            font_name: "Petiglyph".to_string(),
            glyph_size: 8,
            base_threshold: 64,
            threshold_overrides: BTreeMap::new(),
            invert_overrides: BTreeMap::new(),
            compositions: BTreeMap::new(),
            animations: Vec::new(),
            codepoint_start: 0x10_0000,
        };

        let mut app = App::new(manifest_path, config);
        app.view = AppView::Glyphs;
        app.reload_glyphs().expect("initial glyphs load");
        assert_eq!(
            app.glyphs.len(),
            1,
            "initial glyph load should see one source"
        );

        write_test_png(&icons_dir.join("two.png"));
        assert_eq!(
            app.glyphs.len(),
            1,
            "glyph cache should stay stale until the rescan shortcut runs"
        );

        handle_key(&mut app, KeyCode::Char('R'))
            .expect("glyphs rescan shortcut should reload project sources");

        assert_eq!(app.glyphs.len(), 2, "rescan should reload both sources");

        fs::remove_dir_all(project_dir).expect("temp dir is removed");
    }

    #[test]
    fn visible_window_bounds_center_and_clamp_selection() {
        assert_eq!(visible_window_bounds(0, 0, 5), (0, 0));
        assert_eq!(visible_window_bounds(5, 0, 10), (0, 5));
        assert_eq!(visible_window_bounds(20, 0, 5), (0, 5));
        assert_eq!(visible_window_bounds(20, 10, 5), (8, 13));
        assert_eq!(visible_window_bounds(20, 99, 5), (15, 20));
    }

    #[test]
    fn scrollbar_thumb_geometry_tracks_start_and_end_positions() {
        assert_eq!(scrollbar_thumb_geometry(0, 10, 0), (0, 0));
        assert_eq!(scrollbar_thumb_geometry(5, 10, 0), (0, 0));
        assert_eq!(scrollbar_thumb_geometry(100, 10, 0), (0, 1));
        assert_eq!(scrollbar_thumb_geometry(100, 10, 90), (9, 1));
    }

    #[test]
    fn standard_preview_preserves_aspect_ratio_in_square_viewport() {
        let mut coverage = vec![0; 32 * 64];
        for y in 15..49 {
            for x in 0..32 {
                coverage[y * 32 + x] = 255;
            }
        }
        let glyph = PreprocessedGlyph {
            source_path: PathBuf::from("codex-1.png"),
            source_key: "codex-1.png".to_string(),
            source_parent_key: "codex-1.png".to_string(),
            glyph_name: "codex_1".to_string(),
            width: 32,
            height: 64,
            coverage,
            image_fingerprint: "test".to_string(),
            composition_tile: None,
        };

        let lines = preview_lines(&glyph, 64, false, 37, 37);
        let rendered = lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert_eq!(rendered.len(), 35);
        assert!(
            rendered.iter().all(|line| line.chars().count() == 70),
            "aspect-fit preview should preserve width while reducing height to avoid stretching"
        );
    }

    #[test]
    fn stable_grid_animation_preview_uses_emitted_composition_columns() {
        let glyphs = (0..2)
            .flat_map(|row| {
                (0..4).map(move |col| InteractiveGlyph {
                    glyph: PreprocessedGlyph {
                        source_path: PathBuf::from("icons/strip.png"),
                        source_key: format!("strip.png#compose:2x4:{row}:{col}"),
                        source_parent_key: "strip.png".to_string(),
                        glyph_name: format!("strip_r{}_c{}", row + 1, col + 1),
                        width: 1,
                        height: 1,
                        coverage: vec![255],
                        image_fingerprint: "fnv1a64:test".to_string(),
                        composition_tile: Some(CompositionTileInfo {
                            rows: 2,
                            cols: emitted_composition_cols(2),
                            row,
                            col,
                            horizontal_bleed: BleedLevel::Weak,
                            vertical_bleed: BleedLevel::Off,
                        }),
                    },
                    working_threshold: 64,
                    saved_threshold: None,
                    saved_invert: false,
                    working_invert: false,
                })
            })
            .collect::<Vec<_>>();
        let tile_refs = glyphs.iter().collect::<Vec<_>>();

        let lines = composition_preview_lines_stable_frame(
            &tile_refs,
            64,
            false,
            2,
            emitted_composition_cols(2),
            12,
            4,
        );
        let rendered = lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert!(!rendered.contains("unavailable"));
        assert!(
            rendered.chars().any(|ch| !ch.is_whitespace()),
            "grid animation preview should render visible cells"
        );
    }

    #[test]
    fn visible_glyph_rows_groups_animation_frames_under_animation_parent() {
        let project_dir = make_temp_dir("animation-row-grouping");
        let manifest_path = project_dir.join("petiglyph.toml");
        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-animation-row-grouping".to_string(),
            input_dir: project_dir.join("icons"),
            out_dir: project_dir.join("build"),
            font_name: "Petiglyph".to_string(),
            glyph_size: 8,
            base_threshold: 64,
            threshold_overrides: BTreeMap::new(),
            invert_overrides: BTreeMap::new(),
            compositions: BTreeMap::new(),
            animations: vec![AnimationDef {
                name: "walk".to_string(),
                animation_type: AnimationType::Standard,
                fps: 8,
                frames: vec!["f_01.png".to_string(), "f_02.png".to_string()],
                rows: None,
                cols: None,
                horizontal_bleed: None,
                vertical_bleed: None,
                grayscale_processing: None,
            }],
            codepoint_start: 0x10_0000,
        };
        let mut app = App::new(manifest_path, config);
        app.glyphs = ["f_01.png", "f_02.png", "loose.png"]
            .into_iter()
            .map(|source| InteractiveGlyph {
                glyph: PreprocessedGlyph {
                    source_path: project_dir.join("icons").join(source),
                    source_key: source.to_string(),
                    source_parent_key: source.to_string(),
                    glyph_name: source.replace(".png", ""),
                    width: 1,
                    height: 1,
                    coverage: vec![255],
                    image_fingerprint: "fnv1a64:test".to_string(),
                    composition_tile: None,
                },
                working_threshold: 64,
                saved_threshold: None,
                saved_invert: false,
                working_invert: false,
            })
            .collect();

        let rows = app.visible_glyph_rows();
        assert_eq!(
            rows.len(),
            2,
            "animation parents should be collapsed by default"
        );
        app.expanded_animations.insert("walk".to_string());
        let rows = app.visible_glyph_rows();

        assert!(matches!(
            rows.first(),
            Some(VisibleGlyphRow::AnimationParent { animation_idx: 0 })
        ));
        assert!(matches!(
            rows.get(1),
            Some(VisibleGlyphRow::AnimationFrame {
                source_key,
                frame_idx: 0,
                ..
            }) if source_key == "f_01.png"
        ));
        assert!(matches!(
            rows.get(2),
            Some(VisibleGlyphRow::AnimationFrame {
                source_key,
                frame_idx: 1,
                ..
            }) if source_key == "f_02.png"
        ));
        assert_eq!(
            rows.iter()
                .filter(|row| matches!(row, VisibleGlyphRow::Single { .. }))
                .count(),
            1,
            "animation frames should not also appear as loose glyph rows"
        );
    }

    #[test]
    fn standard_animation_row_uses_whole_glyph_when_source_is_also_grid() {
        let project_dir = make_temp_dir("standard-animation-reused-grid-source");
        let manifest_path = project_dir.join("petiglyph.toml");
        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-standard-animation-reused-grid-source".to_string(),
            input_dir: project_dir.join("icons"),
            out_dir: project_dir.join("build"),
            font_name: "Petiglyph".to_string(),
            glyph_size: 8,
            base_threshold: 64,
            threshold_overrides: BTreeMap::new(),
            invert_overrides: BTreeMap::new(),
            compositions: BTreeMap::new(),
            animations: vec![
                AnimationDef {
                    name: "run-grid".to_string(),
                    animation_type: AnimationType::Grid,
                    fps: 8,
                    frames: vec!["runner_01.png".to_string()],
                    rows: Some(1),
                    cols: Some(1),
                    horizontal_bleed: Some(BleedLevel::Weak),
                    vertical_bleed: Some(BleedLevel::Off),
                    grayscale_processing: None,
                },
                AnimationDef {
                    name: "run-standard".to_string(),
                    animation_type: AnimationType::Standard,
                    fps: 8,
                    frames: vec!["runner_01.png".to_string()],
                    rows: None,
                    cols: None,
                    horizontal_bleed: None,
                    vertical_bleed: None,
                    grayscale_processing: None,
                },
            ],
            codepoint_start: 0x10_0000,
        };
        let mut app = App::new(manifest_path, config);
        app.glyphs = vec![
            InteractiveGlyph {
                glyph: PreprocessedGlyph {
                    source_path: project_dir.join("icons/runner_01.png"),
                    source_key: "runner_01.png#compose:1x2:0:0".to_string(),
                    source_parent_key: "runner_01.png".to_string(),
                    glyph_name: "runner_01_r1_c1".to_string(),
                    width: 1,
                    height: 1,
                    coverage: vec![255],
                    image_fingerprint: "fnv1a64:tile".to_string(),
                    composition_tile: Some(CompositionTileInfo {
                        rows: 1,
                        cols: 2,
                        row: 0,
                        col: 0,
                        horizontal_bleed: BleedLevel::Weak,
                        vertical_bleed: BleedLevel::Off,
                    }),
                },
                working_threshold: 64,
                saved_threshold: None,
                saved_invert: false,
                working_invert: false,
            },
            InteractiveGlyph {
                glyph: PreprocessedGlyph {
                    source_path: project_dir.join("icons/runner_01.png"),
                    source_key: "runner_01.png".to_string(),
                    source_parent_key: "runner_01.png".to_string(),
                    glyph_name: "runner_01_standard".to_string(),
                    width: 2,
                    height: 1,
                    coverage: vec![255, 255],
                    image_fingerprint: "fnv1a64:standard".to_string(),
                    composition_tile: None,
                },
                working_threshold: 64,
                saved_threshold: None,
                saved_invert: false,
                working_invert: false,
            },
        ];

        let rows = app.visible_glyph_rows();
        assert_eq!(
            rows.len(),
            2,
            "animation parents should be collapsed by default"
        );
        app.expanded_animations.insert("run-grid".to_string());
        app.expanded_animations.insert("run-standard".to_string());
        let rows = app.visible_glyph_rows();

        assert!(matches!(
            rows.get(3),
            Some(VisibleGlyphRow::AnimationFrame {
                animation_idx: 1,
                glyph_idx: Some(1),
                ..
            })
        ));

        fs::remove_dir_all(project_dir).expect("temp dir is removed");
    }

    #[test]
    fn animation_frame_row_preview_is_pinned_to_selected_frame() {
        let animation = AnimationDef {
            name: "walk".to_string(),
            animation_type: AnimationType::Standard,
            fps: 8,
            frames: vec![
                "f_01.png".to_string(),
                "f_02.png".to_string(),
                "f_03.png".to_string(),
            ],
            rows: None,
            cols: None,
            horizontal_bleed: None,
            vertical_bleed: None,
            grayscale_processing: None,
        };
        let preview = AnimationPreview {
            animation_name: "walk".to_string(),
            frame_index: 2,
            last_frame_at: Instant::now(),
        };
        let parent_row = VisibleGlyphRow::AnimationParent { animation_idx: 0 };
        let frame_row = VisibleGlyphRow::AnimationFrame {
            animation_idx: 0,
            frame_idx: 1,
            source_key: "f_02.png".to_string(),
            glyph_idx: Some(1),
        };

        assert_eq!(
            animation_frame_source_for_preview(Some(&parent_row), &animation, Some(&preview)),
            Some("f_03.png".to_string()),
            "animation parent rows should use the animated frame index"
        );
        assert_eq!(
            animation_frame_source_for_preview(Some(&frame_row), &animation, Some(&preview)),
            Some("f_02.png".to_string()),
            "animation frame rows should preview the selected frame only"
        );
    }

    #[test]
    fn animation_import_reuses_identical_existing_input_file() {
        let project_dir = make_temp_dir("animation-import-reuse-existing");
        let input_dir = project_dir.join("icons");
        let external_dir = project_dir.join("external");
        fs::create_dir_all(&input_dir).expect("input dir is created");
        fs::create_dir_all(&external_dir).expect("external dir is created");
        fs::write(input_dir.join("frame.png"), b"same bytes").expect("input frame is written");
        fs::write(external_dir.join("frame.png"), b"same bytes")
            .expect("external frame is written");

        let result = import_image_files_to_input(
            &input_dir,
            &external_dir.join("frame.png").display().to_string(),
            ExistingImportPolicy::ReuseIdentical,
            animation_media::AnimationImportProcessingOptions {
                grayscale_enabled: false,
                ..Default::default()
            },
        )
        .expect("import should succeed");

        assert_eq!(result.imported, 0);
        assert_eq!(result.skipped_existing, 1);
        assert_eq!(result.imported_source_keys, vec!["frame.png".to_string()]);
        assert!(
            !input_dir.join("frame-1.png").exists(),
            "identical animation frames should reuse the existing input file"
        );
    }

    #[test]
    fn animated_home_workflow_drop_starts_animation_frame_import_task() {
        let project_dir = make_temp_dir("animated-home-drop-import");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");

        let icons_dir = project_dir.join("icons");
        fs::create_dir_all(&icons_dir).expect("icons dir is created");

        let external_dir = project_dir.join("external");
        fs::create_dir_all(&external_dir).expect("external dir is created");
        let frame_path = external_dir.join("frame_01.png");
        fs::write(&frame_path, b"frame bytes").expect("frame file is written");

        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-animated-home-drop-import".to_string(),
            input_dir: icons_dir,
            out_dir: project_dir.join("build"),
            font_name: "Petiglyph".to_string(),
            glyph_size: 8,
            base_threshold: 64,
            threshold_overrides: BTreeMap::new(),
            invert_overrides: BTreeMap::new(),
            compositions: BTreeMap::new(),
            animations: Vec::new(),
            codepoint_start: 0x10_0000,
        };
        let mut app = App::new(manifest_path, config);
        app.start_home_workflow(HomeCreationKind::AnimatedGlyph);
        assert!(matches!(
            app.home_workflow,
            HomeWorkflow::Import(HomeCreationKind::AnimatedGlyph)
        ));

        handle_paste_event_for_test(&mut app, &frame_path.display().to_string())
            .expect("drop/paste import should succeed");

        assert!(
            app.animation_import_task.is_some(),
            "animated home workflow drop should start animation frame import task"
        );
    }

    #[test]
    fn glyph_home_workflow_drop_starts_background_import_task() {
        let project_dir = make_temp_dir("glyph-home-drop-import-task");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");
        let icons_dir = project_dir.join("icons");
        fs::create_dir_all(&icons_dir).expect("icons dir is created");
        let external_dir = project_dir.join("external");
        fs::create_dir_all(&external_dir).expect("external dir is created");
        let image_path = external_dir.join("source.png");
        write_test_png(&image_path);

        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-glyph-home-drop-import-task".to_string(),
            input_dir: icons_dir,
            out_dir: project_dir.join("build"),
            font_name: "Petiglyph".to_string(),
            glyph_size: 8,
            base_threshold: 64,
            threshold_overrides: BTreeMap::new(),
            invert_overrides: BTreeMap::new(),
            compositions: BTreeMap::new(),
            animations: Vec::new(),
            codepoint_start: 0x10_0000,
        };
        let mut app = App::new(manifest_path, config);
        app.start_home_workflow(HomeCreationKind::Glyph);

        handle_paste_event_for_test(&mut app, &image_path.display().to_string())
            .expect("drop/paste import should succeed");
        assert!(
            app.home_import_task.is_some(),
            "glyph home workflow drop should start background image import task"
        );
    }

    #[test]
    fn glyph_home_workflow_queues_additional_drop_payload_while_import_is_running() {
        let project_dir = make_temp_dir("glyph-home-drop-queue-payload");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");
        let icons_dir = project_dir.join("icons");
        fs::create_dir_all(&icons_dir).expect("icons dir is created");
        let external_dir = project_dir.join("external");
        fs::create_dir_all(&external_dir).expect("external dir is created");
        let first_path = external_dir.join("first.png");
        let second_path = external_dir.join("second.png");
        write_test_png(&first_path);
        write_test_png(&second_path);

        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-glyph-home-drop-queue-payload".to_string(),
            input_dir: icons_dir,
            out_dir: project_dir.join("build"),
            font_name: "Petiglyph".to_string(),
            glyph_size: 8,
            base_threshold: 64,
            threshold_overrides: BTreeMap::new(),
            invert_overrides: BTreeMap::new(),
            compositions: BTreeMap::new(),
            animations: Vec::new(),
            codepoint_start: 0x10_0000,
        };
        let mut app = App::new(manifest_path, config);
        app.start_home_workflow(HomeCreationKind::Glyph);

        handle_paste_event_for_test(&mut app, &first_path.display().to_string())
            .expect("first drop should start import task");
        assert!(app.home_import_task.is_some());

        handle_paste_event_for_test(&mut app, &second_path.display().to_string())
            .expect("second drop should queue");

        let expected_queued = second_path.display().to_string();
        assert_eq!(
            app.queued_drop_payload.as_deref(),
            Some(expected_queued.as_str()),
            "second payload should be queued while the first import task is running"
        );
        assert!(
            app.status
                .as_ref()
                .is_some_and(|msg| msg.contains("queued additional dropped files")),
            "queued drop should be acknowledged in status"
        );
    }

    #[test]
    fn glyph_home_workflow_runs_queued_drop_after_current_import_finishes() {
        let project_dir = make_temp_dir("glyph-home-drop-queue-drain");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");
        let icons_dir = project_dir.join("icons");
        fs::create_dir_all(&icons_dir).expect("icons dir is created");
        let external_dir = project_dir.join("external");
        fs::create_dir_all(&external_dir).expect("external dir is created");
        let first_path = external_dir.join("first.png");
        let second_path = external_dir.join("second.png");
        write_test_png(&first_path);
        write_test_png(&second_path);

        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-glyph-home-drop-queue-drain".to_string(),
            input_dir: icons_dir.clone(),
            out_dir: project_dir.join("build"),
            font_name: "Petiglyph".to_string(),
            glyph_size: 8,
            base_threshold: 64,
            threshold_overrides: BTreeMap::new(),
            invert_overrides: BTreeMap::new(),
            compositions: BTreeMap::new(),
            animations: Vec::new(),
            codepoint_start: 0x10_0000,
        };
        let mut app = App::new(manifest_path, config);
        app.start_home_workflow(HomeCreationKind::Glyph);

        handle_paste_event_for_test(&mut app, &first_path.display().to_string())
            .expect("first drop should start import task");
        handle_paste_event_for_test(&mut app, &second_path.display().to_string())
            .expect("second drop should queue");

        drain_background_tasks(&mut app);

        assert!(
            icons_dir.join("first.png").is_file(),
            "first dropped image should be imported"
        );
        assert!(
            icons_dir.join("second.png").is_file(),
            "queued second dropped image should be imported after the first task finishes"
        );
        assert_eq!(
            app.home_workflow_import_count, 2,
            "workflow import count should include imports from queued payloads"
        );
        assert_eq!(
            app.queued_drop_payload, None,
            "queued payload should be cleared after it is processed"
        );
    }

    #[test]
    fn grid_home_workflow_second_drop_replaces_selected_source() {
        let project_dir = make_temp_dir("grid-home-drop-replace");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");

        let icons_dir = project_dir.join("icons");
        fs::create_dir_all(&icons_dir).expect("icons dir is created");

        let external_dir = project_dir.join("external");
        fs::create_dir_all(&external_dir).expect("external dir is created");
        let first_path = external_dir.join("grid_1.png");
        let second_path = external_dir.join("grid_2.png");
        write_test_png(&first_path);
        write_test_png(&second_path);

        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-grid-home-drop-replace".to_string(),
            input_dir: icons_dir,
            out_dir: project_dir.join("build"),
            font_name: "Petiglyph".to_string(),
            glyph_size: 8,
            base_threshold: 64,
            threshold_overrides: BTreeMap::new(),
            invert_overrides: BTreeMap::new(),
            compositions: BTreeMap::new(),
            animations: Vec::new(),
            codepoint_start: 0x10_0000,
        };
        let mut app = App::new(manifest_path, config);
        app.start_home_workflow(HomeCreationKind::Grid);

        handle_paste_event_for_test(&mut app, &first_path.display().to_string())
            .expect("first drop should succeed");
        for _ in 0..100 {
            app.poll_background_tasks_for_test();
            if !app.background_task_in_progress() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            app.home_workflow_grid_source_key.as_deref(),
            Some("grid_1.png"),
            "first drop should set selected grid source"
        );
        assert_eq!(app.home_workflow_import_count, 1);

        handle_paste_event_for_test(&mut app, &second_path.display().to_string())
            .expect("second drop should succeed");
        for _ in 0..100 {
            app.poll_background_tasks_for_test();
            if !app.background_task_in_progress() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            app.home_workflow_grid_source_key.as_deref(),
            Some("grid_2.png"),
            "second drop should replace selected grid source"
        );
        assert_eq!(
            app.home_workflow_import_count, 1,
            "grid workflow should keep a single selected source after replacement"
        );
        assert!(
            app.status
                .as_ref()
                .is_some_and(|msg| !msg.contains("replaced selected image")),
            "replacement detail should not be emitted in footer status"
        );
        assert!(
            app.home_workflow_grid_inline_notice
                .as_ref()
                .is_some_and(|msg| msg.contains("Replaced image:")),
            "grid workflow should surface inline replacement notice in drop area"
        );
    }

    #[test]
    fn grid_home_workflow_refuses_multi_image_drop_without_importing() {
        let project_dir = make_temp_dir("grid-home-refuse-multi-drop");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");

        let icons_dir = project_dir.join("icons");
        fs::create_dir_all(&icons_dir).expect("icons dir is created");

        let external_dir = project_dir.join("external");
        fs::create_dir_all(&external_dir).expect("external dir is created");
        let first_path = external_dir.join("grid_1.png");
        let second_path = external_dir.join("grid_2.png");
        write_test_png(&first_path);
        write_test_png(&second_path);

        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-grid-home-refuse-multi-drop".to_string(),
            input_dir: icons_dir.clone(),
            out_dir: project_dir.join("build"),
            font_name: "Petiglyph".to_string(),
            glyph_size: 8,
            base_threshold: 64,
            threshold_overrides: BTreeMap::new(),
            invert_overrides: BTreeMap::new(),
            compositions: BTreeMap::new(),
            animations: Vec::new(),
            codepoint_start: 0x10_0000,
        };
        let mut app = App::new(manifest_path, config);
        app.start_home_workflow(HomeCreationKind::Grid);

        handle_paste_event_for_test(
            &mut app,
            &format!("{}\n{}", first_path.display(), second_path.display()),
        )
        .expect("multi-image drop should be handled");

        assert!(
            app.home_import_task.is_none(),
            "refused grid multi-drop should not start background import task"
        );
        assert_eq!(
            app.home_workflow_grid_source_key, None,
            "refused grid multi-drop should not update selected grid source"
        );
        assert_eq!(
            app.home_workflow_import_count, 0,
            "refused grid multi-drop should not advance workflow import count"
        );
        let icon_entries = fs::read_dir(&icons_dir)
            .expect("icons dir should remain readable")
            .count();
        assert_eq!(
            icon_entries, 0,
            "refused grid multi-drop should not import files into icons"
        );
        assert!(
            app.status
                .as_ref()
                .is_some_and(|msg| msg.contains("drop only one image at a time")),
            "refused grid multi-drop should explain refusal in status"
        );
    }

    #[test]
    fn animated_home_workflow_reimport_identical_frames_keeps_progress_and_selection() {
        let project_dir = make_temp_dir("animated-home-reimport-identical");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");

        let input_dir = project_dir.join("icons");
        let external_dir = project_dir.join("external");
        fs::create_dir_all(&input_dir).expect("input dir is created");
        fs::create_dir_all(&external_dir).expect("external dir is created");
        fs::write(input_dir.join("frame.png"), b"same bytes").expect("input frame is written");
        fs::write(external_dir.join("frame.png"), b"same bytes")
            .expect("external frame is written");

        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-animated-home-reimport-identical".to_string(),
            input_dir,
            out_dir: project_dir.join("build"),
            font_name: "Petiglyph".to_string(),
            glyph_size: 8,
            base_threshold: 64,
            threshold_overrides: BTreeMap::new(),
            invert_overrides: BTreeMap::new(),
            compositions: BTreeMap::new(),
            animations: Vec::new(),
            codepoint_start: 0x10_0000,
        };
        let mut app = App::new(manifest_path, config);
        app.start_home_workflow(HomeCreationKind::AnimatedGlyph);
        app.home_workflow = HomeWorkflow::Tweaking(HomeCreationKind::AnimatedGlyph);
        app.home_workflow = HomeWorkflow::Tweaking(HomeCreationKind::AnimatedGlyph);
        app.home_workflow = HomeWorkflow::Tweaking(HomeCreationKind::AnimatedGlyph);
        let output = AnimationImportTaskOutput {
            import: DropImportResult {
                imported: 0,
                renamed: 0,
                skipped_existing: 1,
                skipped_unsupported: 0,
                skipped_missing: 0,
                imported_source_keys: vec!["frame.png".to_string()],
                created_source_keys: Vec::new(),
            },
            loaded: None,
            detail_status: None,
        };

        app.finish_animation_import(output);

        assert_eq!(
            app.home_workflow_import_count, 1,
            "reimporting identical existing frames should still count as selected in workflow progress"
        );
        assert_eq!(app.animation_selection_order, vec!["frame.png".to_string()]);
        assert!(
            app.status
                .as_ref()
                .is_some_and(|msg| msg.contains("1 frame selected")),
            "status should confirm selected frames, even for reused existing files"
        );
    }

    #[test]
    fn home_workflow_enter_advances_import_to_tweaking_when_sources_exist() {
        let project_dir = make_temp_dir("home-workflow-enter-to-tweaking");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");
        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-home-workflow-enter-to-tweaking".to_string(),
            input_dir: project_dir.join("icons"),
            out_dir: project_dir.join("build"),
            font_name: "Petiglyph".to_string(),
            glyph_size: 8,
            base_threshold: 64,
            threshold_overrides: BTreeMap::new(),
            invert_overrides: BTreeMap::new(),
            compositions: BTreeMap::new(),
            animations: Vec::new(),
            codepoint_start: 0x10_0000,
        };
        let mut app = App::new(manifest_path, config);
        app.start_home_workflow(HomeCreationKind::AnimatedGlyph);
        app.animation_selection_order = vec!["frame.png".to_string()];

        handle_key(&mut app, KeyCode::Enter).expect("enter should advance to tweaking");
        assert!(matches!(
            app.home_workflow,
            HomeWorkflow::Tweaking(HomeCreationKind::AnimatedGlyph)
        ));
    }

    #[test]
    fn animated_home_workflow_threshold_knob_adjusts_in_tweaking_step() {
        let project_dir = make_temp_dir("animated-home-threshold-knob");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");
        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-animated-home-threshold-knob".to_string(),
            input_dir: project_dir.join("icons"),
            out_dir: project_dir.join("build"),
            font_name: "Petiglyph".to_string(),
            glyph_size: 8,
            base_threshold: 64,
            threshold_overrides: BTreeMap::new(),
            invert_overrides: BTreeMap::new(),
            compositions: BTreeMap::new(),
            animations: Vec::new(),
            codepoint_start: 0x10_0000,
        };
        let mut app = App::new(manifest_path, config);
        app.start_home_workflow(HomeCreationKind::AnimatedGlyph);
        app.home_workflow = HomeWorkflow::Tweaking(HomeCreationKind::AnimatedGlyph);
        app.home_workflow = HomeWorkflow::Tweaking(HomeCreationKind::AnimatedGlyph);

        assert_eq!(app.animation_import_settings.threshold, 64);
        handle_key(&mut app, KeyCode::Left).expect("left focuses export from continue");
        handle_key(&mut app, KeyCode::Left).expect("left focuses frames from export");
        handle_key(&mut app, KeyCode::Left).expect("left focuses threshold from frames");
        assert_eq!(
            app.animation_import_settings.focus,
            super::AnimationImportSettingsFocus::Threshold
        );
        handle_key(&mut app, KeyCode::Up).expect("up increases threshold");
        assert_eq!(app.animation_import_settings.threshold, 65);
        handle_key(&mut app, KeyCode::Down).expect("down decreases threshold");
        assert_eq!(app.animation_import_settings.threshold, 64);
    }

    #[test]
    fn glyph_creation_tweak_threshold_becomes_glyph_panel_threshold() {
        let project_dir = make_temp_dir("glyph-creation-threshold-persists");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");
        let icons_dir = project_dir.join("icons");
        fs::create_dir_all(&icons_dir).expect("icons dir is created");
        write_test_png(&icons_dir.join("source.png"));

        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-glyph-creation-threshold-persists".to_string(),
            input_dir: icons_dir,
            out_dir: project_dir.join("build"),
            font_name: "Petiglyph".to_string(),
            glyph_size: 8,
            base_threshold: 64,
            threshold_overrides: BTreeMap::new(),
            invert_overrides: BTreeMap::new(),
            compositions: BTreeMap::new(),
            animations: Vec::new(),
            codepoint_start: 0x10_0000,
        };
        let mut app = App::new(manifest_path.clone(), config);
        app.start_home_workflow(HomeCreationKind::Glyph);
        app.home_workflow = HomeWorkflow::Tweaking(HomeCreationKind::Glyph);
        app.home_workflow_recent_imported_source_keys = vec!["source.png".to_string()];
        app.home_workflow_import_count = 1;
        app.animation_import_settings.threshold = 91;

        continue_home_workflow_after_tweaking(&mut app, HomeCreationKind::Glyph)
            .expect("continue should persist threshold and load glyphs");

        let manifest = read_manifest(&manifest_path).expect("manifest reload succeeds");
        assert_eq!(manifest.threshold_overrides.get("source.png"), Some(&91));
        let glyph = app
            .glyphs
            .iter()
            .find(|glyph| glyph.glyph.source_parent_key == "source.png")
            .expect("source glyph is loaded");
        assert_eq!(glyph.working_threshold, 91);
        assert_eq!(glyph.saved_threshold, Some(91));
    }
