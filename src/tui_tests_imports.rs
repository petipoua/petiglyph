    #[test]
    fn animated_home_workflow_grayscale_toggle_is_on_by_default_and_toggles_with_keys() {
        let project_dir = make_temp_dir("animated-home-grayscale-toggle");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");

        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-animated-home-grayscale-toggle".to_string(),
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

        assert!(
            app.animation_import_settings.grayscale_enabled,
            "grayscale should default to enabled for animated imports"
        );

        handle_key(&mut app, KeyCode::Left).expect("left moves focus from Continue to export");
        handle_key(&mut app, KeyCode::Left).expect("left moves focus from export to frames");
        handle_key(&mut app, KeyCode::Left).expect("left moves focus from frames to threshold");
        handle_key(&mut app, KeyCode::Left).expect("left moves focus from threshold to options");
        handle_key(&mut app, KeyCode::Left).expect("left moves focus from options to toggle");
        handle_key(&mut app, KeyCode::Enter).expect("enter toggles grayscale");
        assert!(
            !app.animation_import_settings.grayscale_enabled,
            "enter on grayscale toggle should disable grayscale"
        );
        assert!(
            matches!(
                app.home_workflow,
                HomeWorkflow::Tweaking(HomeCreationKind::AnimatedGlyph)
            ),
            "toggling grayscale should not leave the tweaking step"
        );

        handle_key(&mut app, KeyCode::Down).expect("up/down also toggles grayscale");
        assert!(
            app.animation_import_settings.grayscale_enabled,
            "down on grayscale toggle should re-enable grayscale"
        );
    }

    #[test]
    fn animated_home_workflow_grayscale_options_editor_commits_and_cancels() {
        let project_dir = make_temp_dir("animated-home-grayscale-options");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");

        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-animated-home-grayscale-options".to_string(),
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

        handle_key(&mut app, KeyCode::Left).expect("left moves focus from Continue to export");
        handle_key(&mut app, KeyCode::Left).expect("left moves focus from export to frames");
        handle_key(&mut app, KeyCode::Left).expect("left moves focus from frames to threshold");
        handle_key(&mut app, KeyCode::Left).expect("left moves focus from threshold to options");
        handle_key(&mut app, KeyCode::Enter).expect("enter opens grayscale options editor");
        assert!(
            app.animation_import_settings.grayscale_editor.is_some(),
            "options editor should be opened"
        );

        handle_key(&mut app, KeyCode::Up).expect("up adjusts brightness");
        handle_key(&mut app, KeyCode::Right).expect("right focuses contrast");
        handle_key(&mut app, KeyCode::Up).expect("up adjusts contrast");
        handle_key(&mut app, KeyCode::Esc).expect("esc cancels options edit");
        assert!(
            app.animation_import_settings.grayscale_editor.is_none(),
            "editor should close on esc"
        );
        assert_eq!(
            app.animation_import_settings.grayscale_options,
            animation_media::AnimationGrayscaleOptions::default(),
            "esc should restore prior grayscale options"
        );
        assert!(
            matches!(
                app.home_workflow,
                HomeWorkflow::Tweaking(HomeCreationKind::AnimatedGlyph)
            ),
            "esc in editor should not cancel the workflow"
        );

        handle_key(&mut app, KeyCode::Enter).expect("enter reopens grayscale options editor");
        handle_key(&mut app, KeyCode::Up).expect("up adjusts brightness");
        handle_key(&mut app, KeyCode::Right).expect("right focuses contrast");
        handle_key(&mut app, KeyCode::Up).expect("up adjusts contrast");
        handle_key(&mut app, KeyCode::Right).expect("right focuses gamma");
        handle_key(&mut app, KeyCode::Up).expect("up adjusts gamma");
        handle_key(&mut app, KeyCode::Enter).expect("enter commits edited grayscale options");

        let options = app.animation_import_settings.grayscale_options;
        assert_eq!(options.brightness, 1);
        assert_eq!(options.contrast, 1);
        assert_eq!(options.gamma_percent, 105);
        assert!(
            !grayscale_options_are_default(options),
            "committed knobs should mark options as non-default"
        );
    }

    #[test]
    fn animated_home_workflow_can_export_test_image_into_project_test_images() {
        let project_dir = make_temp_dir("animated-home-export-test-image");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");

        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-animated-home-export-test-image".to_string(),
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
        app.animation_selection_order = vec!["runner_01.png".to_string()];
        app.glyphs = vec![InteractiveGlyph {
            glyph: PreprocessedGlyph {
                source_path: PathBuf::from("icons/runner_01.png"),
                source_key: "runner_01.png".to_string(),
                source_parent_key: "runner_01.png".to_string(),
                glyph_name: "runner_01".to_string(),
                width: 2,
                height: 2,
                coverage: vec![255, 0, 0, 255],
                image_fingerprint: "fnv1a64:test-runner".to_string(),
                composition_tile: None,
            },
            working_threshold: 64,
            saved_threshold: None,
            saved_invert: false,
            working_invert: false,
        }];
        app.animation_import_settings.focus =
            super::AnimationImportSettingsFocus::ExportTestImageButton;

        handle_key(&mut app, KeyCode::Enter).expect("enter exports test image from import step");

        let test_images_dir = project_dir.join("test-images");
        let entries = fs::read_dir(&test_images_dir)
            .expect("test-images dir exists")
            .collect::<Result<Vec<_>, _>>()
            .expect("test-images entries are readable");
        assert_eq!(entries.len(), 1, "one preview file should be exported");
        let file_name = entries[0]
            .file_name()
            .into_string()
            .expect("filename should be valid unicode");
        assert!(
            file_name.contains("import_test_source_runner_01_png"),
            "filename should include source key slug"
        );
        assert!(
            file_name.contains("gray_on_bp0_cp0_g100_th064"),
            "filename should include grayscale and threshold parameters"
        );
        assert!(
            app.animation_import_settings
                .last_exported_test_image
                .as_ref()
                .is_some_and(|path| path.starts_with(&test_images_dir)),
            "import settings should retain last exported test image path"
        );
    }

    #[test]
    fn animated_home_workflow_export_frame_count_knob_adjusts_on_export_focus() {
        let project_dir = make_temp_dir("animated-home-export-frame-count");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");
        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-animated-home-export-frame-count".to_string(),
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

        assert_eq!(app.animation_import_settings.export_frame_count, 5);
        handle_key(&mut app, KeyCode::Left).expect("left focuses export from continue");
        assert_eq!(
            app.animation_import_settings.focus,
            super::AnimationImportSettingsFocus::ExportTestImageButton
        );
        handle_key(&mut app, KeyCode::Up).expect("up increases frame export count");
        assert_eq!(app.animation_import_settings.export_frame_count, 6);
        handle_key(&mut app, KeyCode::Down).expect("down decreases frame export count");
        assert_eq!(app.animation_import_settings.export_frame_count, 5);
    }

    #[test]
    fn animated_home_workflow_frames_focus_cycles_preview_frame_with_up_down() {
        let project_dir = make_temp_dir("animated-home-frames-focus");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");
        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-animated-home-frames-focus".to_string(),
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
        app.animation_selection_order = vec![
            "f1.png".to_string(),
            "f2.png".to_string(),
            "f3.png".to_string(),
        ];

        handle_key(&mut app, KeyCode::Left).expect("left focuses export from continue");
        handle_key(&mut app, KeyCode::Left).expect("left focuses frames from export");
        assert_eq!(
            app.animation_import_settings.focus,
            super::AnimationImportSettingsFocus::FramesButton
        );

        handle_key(&mut app, KeyCode::Up).expect("up advances preview frame");
        assert_eq!(app.animation_import_settings.preview_frame_index, 1);
        handle_key(&mut app, KeyCode::Down).expect("down rewinds preview frame");
        assert_eq!(app.animation_import_settings.preview_frame_index, 0);
    }

    #[test]
    fn animated_home_workflow_live_preview_preserves_full_frame_fit() {
        let project_dir = make_temp_dir("animated-home-full-frame-preview");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");
        let icons_dir = project_dir.join("icons");
        fs::create_dir_all(&icons_dir).expect("icons dir is created");

        let source_path = icons_dir.join("frame.png");
        let mut image = RgbaImage::from_pixel(32, 16, Rgba([255, 255, 255, 0]));
        for y in 4..12 {
            for x in 24..32 {
                image.put_pixel(x, y, Rgba([0, 0, 0, 255]));
            }
        }
        image.save(&source_path).expect("frame image is written");

        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-animated-home-full-frame-preview".to_string(),
            input_dir: icons_dir.clone(),
            out_dir: project_dir.join("build"),
            font_name: "Petiglyph".to_string(),
            glyph_size: 16,
            base_threshold: 64,
            threshold_overrides: BTreeMap::new(),
            invert_overrides: BTreeMap::new(),
            compositions: BTreeMap::new(),
            animations: Vec::new(),
            codepoint_start: 0x10_0000,
        };
        let mut app = App::new(manifest_path, config);
        app.animation_import_settings.grayscale_enabled = false;

        let trimmed = app
            .live_import_source_coverage(&source_path)
            .expect("static live preview coverage exists");

        app.start_home_workflow(HomeCreationKind::AnimatedGlyph);
        app.home_workflow = HomeWorkflow::Tweaking(HomeCreationKind::AnimatedGlyph);
        app.animation_selection_order = vec!["frame.png".to_string()];
        app.animation_selection_set.insert("frame.png".to_string());

        let animated = app
            .live_import_source_coverage(&source_path)
            .expect("animated live preview coverage exists");
        let expected =
            coverage_map_from_image_with_fit(&image, 16, SourceFitMode::PreserveFrame)
                .expect("expected preserve-frame coverage");

        assert_eq!(
            animated, expected,
            "animated draft preview should preserve the full frame instead of trimming content bounds"
        );
        assert_ne!(
            trimmed, animated,
            "live preview cache must distinguish trim-fit from full-frame animation fit"
        );

        fs::remove_dir_all(project_dir).expect("temp dir is removed");
    }

    #[test]
    fn animated_standard_workflow_popup_preview_uses_whole_frame_scale() {
        assert_animated_workflow_popup_preview_uses_whole_frame_scale(
            HomeCreationKind::AnimatedGlyph,
            "animated-standard-popup-whole-frame-preview",
        );
    }

    #[test]
    fn animated_grid_workflow_popup_preview_uses_whole_frame_scale() {
        assert_animated_workflow_popup_preview_uses_whole_frame_scale(
            HomeCreationKind::AnimatedGridGlyph,
            "animated-grid-popup-whole-frame-preview",
        );
    }

    fn assert_animated_workflow_popup_preview_uses_whole_frame_scale(
        kind: HomeCreationKind,
        temp_name: &str,
    ) {
        let project_dir = make_temp_dir(temp_name);
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");
        let icons_dir = project_dir.join("icons");
        fs::create_dir_all(&icons_dir).expect("icons dir is created");

        let tall_frame = icons_dir.join("frame-tall.png");
        let short_frame = icons_dir.join("frame-short.png");
        let mut tall = RgbaImage::from_pixel(32, 16, Rgba([255, 255, 255, 0]));
        for y in 0..16 {
            for x in 8..24 {
                tall.put_pixel(x, y, Rgba([0, 0, 0, 255]));
            }
        }
        tall.save(&tall_frame).expect("tall frame image is written");

        let mut short = RgbaImage::from_pixel(32, 16, Rgba([255, 255, 255, 0]));
        for y in 7..9 {
            for x in 8..24 {
                short.put_pixel(x, y, Rgba([0, 0, 0, 255]));
            }
        }
        short.save(&short_frame)
            .expect("short frame image is written");

        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: format!("test-{temp_name}"),
            input_dir: icons_dir,
            out_dir: project_dir.join("build"),
            font_name: "Petiglyph".to_string(),
            glyph_size: 16,
            base_threshold: 64,
            threshold_overrides: BTreeMap::new(),
            invert_overrides: BTreeMap::new(),
            compositions: BTreeMap::new(),
            animations: Vec::new(),
            codepoint_start: 0x10_0000,
        };
        let mut app = App::new(manifest_path, config);
        app.start_home_workflow(kind);
        app.home_workflow = HomeWorkflow::Tweaking(kind);
        app.animation_import_settings.grayscale_enabled = false;
        app.animation_selection_order = vec![
            "frame-tall.png".to_string(),
            "frame-short.png".to_string(),
        ];
        app.animation_selection_set
            .insert("frame-tall.png".to_string());
        app.animation_selection_set
            .insert("frame-short.png".to_string());

        app.animation_import_settings.preview_frame_index = 0;
        let (_, tall_lines) = home_workflow_preview_lines(&app, kind, 32, 32);
        app.animation_import_settings.preview_frame_index = 1;
        let (_, short_lines) = home_workflow_preview_lines(&app, kind, 32, 32);

        assert_eq!(
            tall_lines.len(),
            short_lines.len(),
            "animated workflow popup previews should scale from the whole animation frame, not each frame's active bounds"
        );
        assert!(
            tall_lines.len() > 16,
            "whole-frame preview should keep the reserved frame area instead of shrinking to active content"
        );

        fs::remove_dir_all(project_dir).expect("temp dir is removed");
    }

    #[test]
    fn animated_home_workflow_export_uses_first_five_frames_by_default() {
        let project_dir = make_temp_dir("animated-home-export-default-five");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");
        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-animated-home-export-default-five".to_string(),
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
        app.animation_import_settings.focus =
            super::AnimationImportSettingsFocus::ExportTestImageButton;
        app.animation_selection_order = (1..=6).map(|idx| format!("f{idx}.png")).collect();
        app.glyphs = (1..=6)
            .map(|idx| InteractiveGlyph {
                glyph: PreprocessedGlyph {
                    source_path: PathBuf::from(format!("icons/f{idx}.png")),
                    source_key: format!("f{idx}.png"),
                    source_parent_key: format!("f{idx}.png"),
                    glyph_name: format!("f{idx}"),
                    width: 1,
                    height: 1,
                    coverage: vec![255],
                    image_fingerprint: format!("fnv1a64:test-f{idx}"),
                    composition_tile: None,
                },
                working_threshold: 64,
                saved_threshold: None,
                saved_invert: false,
                working_invert: false,
            })
            .collect();

        handle_key(&mut app, KeyCode::Enter).expect("enter exports first default frame batch");

        let test_images_dir = project_dir.join("test-images");
        let entries = fs::read_dir(&test_images_dir)
            .expect("test-images dir exists")
            .collect::<Result<Vec<_>, _>>()
            .expect("test-images entries are readable");
        assert_eq!(
            entries.len(),
            5,
            "default animated export should include first five frames"
        );
    }

    #[test]
    fn grid_home_workflow_can_export_test_image() {
        let project_dir = make_temp_dir("grid-home-export-test-image");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");

        let icons_dir = project_dir.join("icons");
        fs::create_dir_all(&icons_dir).expect("icons dir is created");
        let external_dir = project_dir.join("external");
        fs::create_dir_all(&external_dir).expect("external dir is created");
        let dropped = external_dir.join("grid_source.png");
        write_test_png(&dropped);

        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-grid-home-export-test-image".to_string(),
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
        handle_paste_event_for_test(&mut app, &dropped.display().to_string())
            .expect("grid import should succeed");
        drain_background_tasks(&mut app);
        app.home_workflow = HomeWorkflow::Tweaking(HomeCreationKind::Grid);
        app.animation_import_settings.focus =
            super::AnimationImportSettingsFocus::ExportTestImageButton;

        handle_key(&mut app, KeyCode::Enter).expect("enter exports grid test image");

        let test_images_dir = project_dir.join("test-images");
        let entries = fs::read_dir(&test_images_dir)
            .expect("test-images dir exists")
            .collect::<Result<Vec<_>, _>>()
            .expect("test-images entries are readable");
        assert_eq!(entries.len(), 1, "one grid export file should be produced");
    }

    #[test]
    fn grid_home_workflow_export_falls_back_to_source_file_when_glyph_cache_is_empty() {
        let project_dir = make_temp_dir("grid-home-export-fallback-source");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");

        let icons_dir = project_dir.join("icons");
        fs::create_dir_all(&icons_dir).expect("icons dir is created");
        let source_path = icons_dir.join("grid_source.png");
        write_test_png(&source_path);

        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-grid-home-export-fallback-source".to_string(),
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
        app.home_workflow = HomeWorkflow::Tweaking(HomeCreationKind::Grid);
        app.home_workflow_grid_source_key = Some("grid_source.png".to_string());
        app.home_workflow_import_count = 1;
        app.glyphs.clear();
        app.animation_import_settings.focus =
            super::AnimationImportSettingsFocus::ExportTestImageButton;

        handle_key(&mut app, KeyCode::Enter).expect("enter exports grid test image via fallback");

        let test_images_dir = project_dir.join("test-images");
        let entries = fs::read_dir(&test_images_dir)
            .expect("test-images dir exists")
            .collect::<Result<Vec<_>, _>>()
            .expect("test-images entries are readable");
        assert_eq!(
            entries.len(),
            1,
            "fallback should still produce one grid export file"
        );
    }

    #[test]
    fn create_grid_animation_sorts_frames_naturally_before_persisting() {
        let project_dir = make_temp_dir("animation-frame-natural-sort");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");

        let icons_dir = project_dir.join("icons");
        fs::create_dir_all(&icons_dir).expect("icons dir is created");
        for file_name in ["runner_10.png", "runner_2.png", "runner_1.png"] {
            write_test_png(&icons_dir.join(file_name));
        }

        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-animation-frame-natural-sort".to_string(),
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

        let animation_config = AnimationConfig {
            selected_frames: vec![
                "runner_10.png".to_string(),
                "runner_2.png".to_string(),
                "runner_1.png".to_string(),
            ],
            animation_name: "walk_anim".to_string(),
            animation_type: AnimationType::Grid,
            fps: 8,
            rows: 1,
            cols: 1,
            horizontal_bleed: BleedLevel::Weak,
            vertical_bleed: BleedLevel::Off,
            grayscale_processing: None,
            focus: AnimationConfigFocus::Fps,
        };

        let output = super::create_animation_task(
            manifest_path.clone(),
            config.input_dir.clone(),
            TuiLaunchOverrides::default(),
            false,
            animation_config,
        )
        .expect("animation should persist");
        assert_eq!(output.config.animations.len(), 1);
        assert_eq!(output.loaded.glyphs.len(), 6);

        let manifest = read_manifest(&manifest_path).expect("manifest reloads");
        assert_eq!(manifest.animations.len(), 1);
        assert_eq!(
            manifest.animations[0].frames,
            vec![
                "runner_1.png".to_string(),
                "runner_2.png".to_string(),
                "runner_10.png".to_string()
            ]
        );

        fs::remove_dir_all(project_dir).expect("temp dir is removed");
    }

    #[test]
    fn create_grid_animation_auto_duplicates_conflicting_frame_compositions() {
        let project_dir = make_temp_dir("animation-grid-conflict-auto-duplicate");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");

        let icons_dir = project_dir.join("icons");
        fs::create_dir_all(&icons_dir).expect("icons dir is created");
        write_test_png(&icons_dir.join("frame.png"));

        persist_composition_definition(
            &manifest_path,
            "frame.png",
            Some(CompositionDef {
                rows: 2,
                cols: 2,
                horizontal_bleed: BleedLevel::Weak,
                vertical_bleed: BleedLevel::Off,
            }),
        )
        .expect("initial composition persists");

        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-animation-grid-conflict-auto-duplicate".to_string(),
            input_dir: icons_dir,
            out_dir: project_dir.join("build"),
            font_name: "Petiglyph".to_string(),
            glyph_size: 8,
            base_threshold: 64,
            threshold_overrides: BTreeMap::new(),
            invert_overrides: BTreeMap::new(),
            compositions: BTreeMap::from([(
                "frame.png".to_string(),
                CompositionDef {
                    rows: 2,
                    cols: 2,
                    horizontal_bleed: BleedLevel::Weak,
                    vertical_bleed: BleedLevel::Off,
                },
            )]),
            animations: Vec::new(),
            codepoint_start: 0x10_0000,
        };

        let animation_config = AnimationConfig {
            selected_frames: vec!["frame.png".to_string()],
            animation_name: "frame_anim".to_string(),
            animation_type: AnimationType::Grid,
            fps: 8,
            rows: 1,
            cols: 1,
            horizontal_bleed: BleedLevel::Weak,
            vertical_bleed: BleedLevel::Off,
            grayscale_processing: None,
            focus: AnimationConfigFocus::Fps,
        };

        let output = super::create_animation_task(
            manifest_path.clone(),
            config.input_dir.clone(),
            TuiLaunchOverrides::default(),
            false,
            animation_config,
        )
        .expect("animation should persist with duplicate frame");
        assert_eq!(output.config.animations.len(), 1);
        assert_eq!(output.loaded.glyphs.len(), 10);

        let manifest = read_manifest(&manifest_path).expect("manifest reloads");
        assert_eq!(manifest.animations.len(), 1);
        let created = &manifest.animations[0];
        assert_eq!(created.frames.len(), 1);
        assert_ne!(
            created.frames[0], "frame.png",
            "conflicting frame should be auto-duplicated to a new source key"
        );
        assert!(
            created.frames[0].starts_with("frame-"),
            "auto-duplicated key should use incremental suffix"
        );
        assert!(
            manifest.compositions.contains_key("frame.png"),
            "original composition should be preserved"
        );
        assert!(
            manifest.compositions.contains_key(&created.frames[0]),
            "duplicated frame should receive desired composition"
        );

        fs::remove_dir_all(project_dir).expect("temp dir is removed");
    }

    #[test]
    fn animation_create_task_spinner_advances_while_worker_runs() {
        let project_dir = make_temp_dir("animation-create-spinner-advances");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");
        let icons_dir = project_dir.join("icons");
        fs::create_dir_all(&icons_dir).expect("icons dir is created");
        write_test_png(&icons_dir.join("frame.png"));

        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-animation-create-spinner-advances".to_string(),
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
        let (_sender, receiver) =
            std::sync::mpsc::channel::<Result<super::AnimationCreateTaskOutput, String>>();
        app.animation_create_task = Some(super::AnimationCreateTask {
            receiver,
            spinner_index: 0,
            spinner_last_frame_at: Instant::now()
                - Duration::from_millis(super::FONT_TASK_SPINNER_FRAME_MS * 2),
        });
        app.live_glyph_source_count = Some(0);
        app.live_glyph_source_probe_fingerprint = None;
        app.live_glyph_source_probe_at =
            Some(Instant::now() - Duration::from_millis(super::GLYPH_SOURCE_COUNT_REFRESH_MS));

        assert_eq!(app.animation_create_spinner_frame(), "-");
        app.poll_animation_create_task();
        assert_eq!(app.animation_create_spinner_frame(), "|");
        app.refresh_live_glyph_source_count();
        assert_eq!(
            app.live_glyph_source_count,
            Some(0),
            "source scans must stay off the UI thread while creation is running"
        );
    }

    #[test]
    fn animation_name_is_derived_from_first_frame_and_suffixed_with_anim() {
        let config = RuntimeConfig {
            project_dir: PathBuf::from("/tmp/project"),
            project_id: "test-animation-name-derive".to_string(),
            input_dir: PathBuf::from("/tmp/project/icons"),
            out_dir: PathBuf::from("/tmp/project/build"),
            font_name: "Petiglyph".to_string(),
            glyph_size: 8,
            base_threshold: 64,
            threshold_overrides: BTreeMap::new(),
            invert_overrides: BTreeMap::new(),
            compositions: BTreeMap::new(),
            animations: Vec::new(),
            codepoint_start: 0x10_0000,
        };
        let frames = vec![
            "run-fast_001.png".to_string(),
            "run-fast_002.png".to_string(),
        ];
        let name = default_animation_name_from_frames(&config, &frames);
        assert_eq!(name, "runfast_anim");
    }

    #[test]
    fn animation_name_conflicts_increment_with_numeric_suffix() {
        let mut config = RuntimeConfig {
            project_dir: PathBuf::from("/tmp/project"),
            project_id: "test-animation-name-conflicts".to_string(),
            input_dir: PathBuf::from("/tmp/project/icons"),
            out_dir: PathBuf::from("/tmp/project/build"),
            font_name: "Petiglyph".to_string(),
            glyph_size: 8,
            base_threshold: 64,
            threshold_overrides: BTreeMap::new(),
            invert_overrides: BTreeMap::new(),
            compositions: BTreeMap::new(),
            animations: Vec::new(),
            codepoint_start: 0x10_0000,
        };
        config.animations.push(AnimationDef {
            name: "runner_anim".to_string(),
            animation_type: AnimationType::Standard,
            fps: 8,
            frames: vec!["runner_001.png".to_string()],
            rows: None,
            cols: None,
            horizontal_bleed: None,
            vertical_bleed: None,
            grayscale_processing: None,
        });
        config.animations.push(AnimationDef {
            name: "runner_anim_1".to_string(),
            animation_type: AnimationType::Standard,
            fps: 8,
            frames: vec!["runner_002.png".to_string()],
            rows: None,
            cols: None,
            horizontal_bleed: None,
            vertical_bleed: None,
            grayscale_processing: None,
        });

        let name = default_animation_name_from_frames(&config, &["runner_010.png".to_string()]);
        assert_eq!(name, "runner_anim_2");
    }

    #[test]
    fn preview_leftmost_control_prefers_threshold_then_fps_then_invert() {
        assert_eq!(
            preview_leftmost_control(true, true, false),
            Some(GlyphPreviewControl::Threshold)
        );
        assert_eq!(
            preview_leftmost_control(false, true, true),
            Some(GlyphPreviewControl::Fps)
        );
        assert_eq!(
            preview_leftmost_control(false, true, false),
            Some(GlyphPreviewControl::Fps)
        );
        assert_eq!(preview_leftmost_control(false, false, false), None);
    }

    #[test]
    fn animation_parent_threshold_sources_include_all_frames_but_frame_row_is_specific() {
        let project_dir = make_temp_dir("animation-threshold-sources");
        let manifest_path = project_dir.join("petiglyph.toml");
        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-animation-threshold-sources".to_string(),
            input_dir: project_dir.join("icons"),
            out_dir: project_dir.join("build"),
            font_name: "Petiglyph".to_string(),
            glyph_size: 8,
            base_threshold: 64,
            threshold_overrides: BTreeMap::new(),
            invert_overrides: BTreeMap::new(),
            compositions: BTreeMap::new(),
            animations: vec![AnimationDef {
                name: "run_anim".to_string(),
                animation_type: AnimationType::Standard,
                fps: 8,
                frames: vec!["f1.png".to_string(), "f2.png".to_string()],
                rows: None,
                cols: None,
                horizontal_bleed: None,
                vertical_bleed: None,
                grayscale_processing: None,
            }],
            codepoint_start: 0x10_0000,
        };
        let mut app = App::new(manifest_path, config);
        app.glyphs = vec![
            InteractiveGlyph {
                glyph: PreprocessedGlyph {
                    source_path: PathBuf::from("icons/f1.png"),
                    source_key: "f1.png".to_string(),
                    source_parent_key: "f1.png".to_string(),
                    glyph_name: "f1".to_string(),
                    width: 1,
                    height: 1,
                    coverage: vec![255],
                    image_fingerprint: "fnv1a64:test-f1".to_string(),
                    composition_tile: None,
                },
                working_threshold: 64,
                saved_threshold: None,
                saved_invert: false,
                working_invert: false,
            },
            InteractiveGlyph {
                glyph: PreprocessedGlyph {
                    source_path: PathBuf::from("icons/f2.png"),
                    source_key: "f2.png".to_string(),
                    source_parent_key: "f2.png".to_string(),
                    glyph_name: "f2".to_string(),
                    width: 1,
                    height: 1,
                    coverage: vec![255],
                    image_fingerprint: "fnv1a64:test-f2".to_string(),
                    composition_tile: None,
                },
                working_threshold: 64,
                saved_threshold: None,
                saved_invert: false,
                working_invert: false,
            },
        ];
        app.expanded_animations.insert("run_anim".to_string());

        app.selected_visible = 0;
        assert_eq!(
            selected_threshold_sources(&app),
            Some(vec!["f1.png".to_string(), "f2.png".to_string()])
        );

        app.selected_visible = 1;
        assert_eq!(
            selected_threshold_sources(&app),
            Some(vec!["f1.png".to_string()])
        );

        app.selected_visible = 2;
        assert_eq!(
            selected_threshold_sources(&app),
            Some(vec!["f2.png".to_string()])
        );

        fs::remove_dir_all(project_dir).expect("temp dir is removed");
    }

    #[test]
    fn animation_non_uniform_threshold_detection_tracks_frame_specific_overrides() {
        let project_dir = make_temp_dir("animation-non-uniform-thresholds");
        let manifest_path = project_dir.join("petiglyph.toml");
        let animation = AnimationDef {
            name: "blink_anim".to_string(),
            animation_type: AnimationType::Standard,
            fps: 12,
            frames: vec!["a.png".to_string(), "b.png".to_string()],
            rows: None,
            cols: None,
            horizontal_bleed: None,
            vertical_bleed: None,
            grayscale_processing: None,
        };
        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-animation-non-uniform-thresholds".to_string(),
            input_dir: project_dir.join("icons"),
            out_dir: project_dir.join("build"),
            font_name: "Petiglyph".to_string(),
            glyph_size: 8,
            base_threshold: 64,
            threshold_overrides: BTreeMap::new(),
            invert_overrides: BTreeMap::new(),
            compositions: BTreeMap::new(),
            animations: vec![animation.clone()],
            codepoint_start: 0x10_0000,
        };
        let mut app = App::new(manifest_path, config);
        app.glyphs = vec![
            InteractiveGlyph {
                glyph: PreprocessedGlyph {
                    source_path: PathBuf::from("icons/a.png"),
                    source_key: "a.png".to_string(),
                    source_parent_key: "a.png".to_string(),
                    glyph_name: "a".to_string(),
                    width: 1,
                    height: 1,
                    coverage: vec![255],
                    image_fingerprint: "fnv1a64:test-a".to_string(),
                    composition_tile: None,
                },
                working_threshold: 60,
                saved_threshold: Some(60),
                saved_invert: false,
                working_invert: false,
            },
            InteractiveGlyph {
                glyph: PreprocessedGlyph {
                    source_path: PathBuf::from("icons/b.png"),
                    source_key: "b.png".to_string(),
                    source_parent_key: "b.png".to_string(),
                    glyph_name: "b".to_string(),
                    width: 1,
                    height: 1,
                    coverage: vec![255],
                    image_fingerprint: "fnv1a64:test-b".to_string(),
                    composition_tile: None,
                },
                working_threshold: 72,
                saved_threshold: Some(72),
                saved_invert: false,
                working_invert: false,
            },
        ];

        assert!(animation_has_non_uniform_frame_thresholds(&app, &animation));
        app.glyphs[1].working_threshold = 60;
        assert!(!animation_has_non_uniform_frame_thresholds(
            &app, &animation
        ));

        fs::remove_dir_all(project_dir).expect("temp dir is removed");
    }

    #[test]
    fn animation_non_uniform_invert_detection_tracks_frame_specific_overrides() {
        let project_dir = make_temp_dir("animation-non-uniform-invert");
        let manifest_path = project_dir.join("petiglyph.toml");
        let animation = AnimationDef {
            name: "blink_anim".to_string(),
            animation_type: AnimationType::Standard,
            fps: 12,
            frames: vec!["a.png".to_string(), "b.png".to_string()],
            rows: None,
            cols: None,
            horizontal_bleed: None,
            vertical_bleed: None,
            grayscale_processing: None,
        };
        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-animation-non-uniform-invert".to_string(),
            input_dir: project_dir.join("icons"),
            out_dir: project_dir.join("build"),
            font_name: "Petiglyph".to_string(),
            glyph_size: 8,
            base_threshold: 64,
            threshold_overrides: BTreeMap::new(),
            invert_overrides: BTreeMap::new(),
            compositions: BTreeMap::new(),
            animations: vec![animation.clone()],
            codepoint_start: 0x10_0000,
        };
        let mut app = App::new(manifest_path, config);
        app.glyphs = vec![
            InteractiveGlyph {
                glyph: PreprocessedGlyph {
                    source_path: PathBuf::from("icons/a.png"),
                    source_key: "a.png".to_string(),
                    source_parent_key: "a.png".to_string(),
                    glyph_name: "a".to_string(),
                    width: 1,
                    height: 1,
                    coverage: vec![255],
                    image_fingerprint: "fnv1a64:test-a".to_string(),
                    composition_tile: None,
                },
                working_threshold: 64,
                saved_threshold: None,
                saved_invert: true,
                working_invert: true,
            },
            InteractiveGlyph {
                glyph: PreprocessedGlyph {
                    source_path: PathBuf::from("icons/b.png"),
                    source_key: "b.png".to_string(),
                    source_parent_key: "b.png".to_string(),
                    glyph_name: "b".to_string(),
                    width: 1,
                    height: 1,
                    coverage: vec![255],
                    image_fingerprint: "fnv1a64:test-b".to_string(),
                    composition_tile: None,
                },
                working_threshold: 64,
                saved_threshold: None,
                saved_invert: false,
                working_invert: false,
            },
        ];

        assert!(animation_has_non_uniform_frame_invert(&app, &animation));
        app.glyphs[1].working_invert = true;
        assert!(!animation_has_non_uniform_frame_invert(&app, &animation));

        fs::remove_dir_all(project_dir).expect("temp dir is removed");
    }

    #[test]
    fn drag_images_placeholder_handles_small_and_regular_regions() {
        assert!(
            drag_images_here_lines(
                6,
                2,
                ratatui::style::Color::Cyan,
                0,
                false,
                false,
                None,
                None,
            )
            .is_empty(),
            "very small regions should skip drag placeholder rendering"
        );

        let lines = drag_images_here_lines(
            40,
            7,
            ratatui::style::Color::Cyan,
            3,
            false,
            false,
            None,
            None,
        );
        assert_eq!(lines.len(), 7, "placeholder should fill requested height");
        let rendered = lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        assert!(
            rendered
                .iter()
                .any(|line| line.contains("DRAG/PASTE IMAGES HERE")),
            "placeholder body should include drag/paste label"
        );
        assert!(
            rendered.iter().any(|line| line.contains("Images added: 3")),
            "placeholder body should include import counter"
        );
        assert!(
            rendered
                .iter()
                .any(|line| line.contains("Images added: 3 ✓")),
            "placeholder should show a checkmark when images have been added"
        );

        let zero_lines = drag_images_here_lines(
            40,
            7,
            ratatui::style::Color::Cyan,
            0,
            false,
            false,
            None,
            None,
        );
        let zero_rendered = zero_lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        assert!(
            zero_rendered
                .iter()
                .any(|line| line.contains("Images added: 0")),
            "placeholder should still render counter at zero"
        );
        assert!(
            zero_rendered
                .iter()
                .all(|line| !line.contains("Images added: 0 ✓")),
            "placeholder should not show checkmark when no images were added"
        );

        let media_lines = drag_images_here_lines(
            40,
            7,
            ratatui::style::Color::Cyan,
            2,
            true,
            false,
            None,
            None,
        );
        let media_rendered = media_lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        assert!(
            media_rendered
                .iter()
                .any(|line| line.contains("DRAG/PASTE MEDIA HERE")),
            "animation placeholder should show media label"
        );
        assert!(
            media_rendered
                .iter()
                .any(|line| line.contains("Media added: 2")),
            "animation placeholder should show media counter"
        );

        let processing_lines = drag_images_here_lines(
            40,
            7,
            ratatui::style::Color::Cyan,
            0,
            true,
            false,
            Some("|"),
            None,
        );
        let processing_rendered = processing_lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        assert!(
            processing_rendered
                .iter()
                .any(|line| line.contains("Processing |")),
            "placeholder should show a processing spinner before completion"
        );
        assert!(
            processing_rendered.iter().all(|line| !line.contains("✓")),
            "placeholder should not show checkmark while processing is active"
        );

        let replace_lines = drag_images_here_lines(
            50,
            8,
            ratatui::style::Color::Cyan,
            1,
            false,
            false,
            None,
            Some("Replaced image: grid_1.png -> grid_2.png"),
        );
        let replace_rendered = replace_lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        assert!(
            replace_rendered
                .iter()
                .any(|line| line.contains("Replaced image: grid_1.png -> grid_2.png")),
            "placeholder should show inline replacement notice when provided"
        );
    }

    #[test]
    fn windows_creation_workflow_picker_copy_mentions_picker() {
        assert_eq!(
            super::home_import_missing_sources_message_for_os(HomeCreationKind::Glyph, "windows"),
            "pick at least one source image in the Windows file picker, then press Enter"
        );
        assert_eq!(
            super::home_import_missing_sources_message_for_os(HomeCreationKind::Grid, "windows"),
            "create grid: pick exactly one image in the Windows file picker, then press Enter"
        );
        assert_eq!(
            super::home_workflow_import_hint_for_os(HomeCreationKind::AnimatedGlyph, "windows"),
            "pick images/GIFs/videos in the Windows file picker for this popup."
        );
        assert_eq!(
            super::import_step_enter_help_for_os("windows"),
            "open file picker / continue after import"
        );
        assert_eq!(
            super::creation_workflow_import_fallback_label(false, true),
            " Pick image files here with the Windows file picker."
        );
        assert_eq!(
            super::creation_workflow_import_fallback_label(true, true),
            " Pick media files here with the Windows file picker."
        );
        assert_eq!(
            super::creation_workflow_import_area_label(false, true),
            "PICK/PASTE IMAGES HERE"
        );
        assert_eq!(
            super::creation_workflow_import_area_label(true, true),
            "PICK/PASTE MEDIA HERE"
        );
    }

    #[test]
    fn windows_picker_config_matches_workflow_constraints() {
        let glyph = super::windows_creation_workflow_picker_config(HomeCreationKind::Glyph);
        assert!(glyph.multiselect);
        assert!(glyph.filter.contains("*.gif"));

        let grid = super::windows_creation_workflow_picker_config(HomeCreationKind::Grid);
        assert!(!grid.multiselect);
        assert!(grid.title.contains("one source image"));

        let animated =
            super::windows_creation_workflow_picker_config(HomeCreationKind::AnimatedGridGlyph);
        assert!(animated.multiselect);
        assert!(animated.filter.contains("*.mp4"));
        assert!(animated.filter.contains("*.m4v"));
    }

    #[test]
    fn collect_dropped_paths_splits_concatenated_file_uris() {
        let project_dir = make_temp_dir("collect-dropped-paths-file-uri");
        let first = project_dir.join("a.png");
        let second = project_dir.join("b.png");
        fs::write(&first, b"a").expect("first file is written");
        fs::write(&second, b"b").expect("second file is written");

        let payload = format!("file://{}file://{}", first.display(), second.display());
        let paths = collect_dropped_paths(&payload);
        assert_eq!(paths.len(), 2, "payload should split both file URIs");
        assert!(paths.contains(&first));
        assert!(paths.contains(&second));

        fs::remove_dir_all(project_dir).expect("temp dir is removed");
    }

    #[test]
    fn shell_token_split_preserves_windows_path_separators() {
        let payload = r#"C:\Users\petiglyph\frame.png "C:\Users\petiglyph\space frame.png""#;
        let tokens = split_shell_like_tokens(payload);

        assert_eq!(
            tokens,
            vec![
                r"C:\Users\petiglyph\frame.png".to_string(),
                r"C:\Users\petiglyph\space frame.png".to_string(),
            ]
        );
    }

    #[test]
    fn static_import_does_not_fail_when_grayscale_skips_non_rewritten_formats() {
        let project_dir = make_temp_dir("static-import-grayscale-gif");
        let input_dir = project_dir.join("icons");
        let external_dir = project_dir.join("external");
        fs::create_dir_all(&input_dir).expect("input dir is created");
        fs::create_dir_all(&external_dir).expect("external dir is created");
        let gif_path = external_dir.join("frame.gif");
        fs::write(
            &gif_path,
            [
                0x47, 0x49, 0x46, 0x38, 0x39, 0x61, 0x01, 0x00, 0x01, 0x00, 0x80, 0x00, 0x00, 0x00,
                0x00, 0x00, 0xff, 0xff, 0xff, 0x21, 0xf9, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00, 0x2c,
                0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x02, 0x02, 0x44, 0x01, 0x00,
                0x3b,
            ],
        )
        .expect("gif file is written");

        let result = import_image_files_to_input(
            &input_dir,
            &gif_path.display().to_string(),
            ExistingImportPolicy::Rename,
            animation_media::AnimationImportProcessingOptions::default(),
        )
        .expect("import should succeed");

        assert_eq!(result.imported, 1);
        assert!(input_dir.join("frame.gif").is_file());

        fs::remove_dir_all(project_dir).expect("temp dir is removed");
    }

    #[test]
    fn unescape_backslashes_preserves_windows_path_separators() {
        assert_eq!(
            super::unescape_backslashes(r"C:\Users\alice\icons\frame.gif"),
            r"C:\Users\alice\icons\frame.gif"
        );
    }

    fn provider_commands(providers: &[super::ClipboardProvider]) -> Vec<&'static str> {
        providers.iter().map(|provider| provider.command).collect()
    }

    #[test]
    fn clipboard_provider_selection_simulates_cross_os_matrix() {
        assert_eq!(
            provider_commands(super::clipboard_providers_for_os("linux", true)),
            vec!["wl-copy"]
        );
        assert_eq!(
            provider_commands(super::clipboard_providers_for_os("linux", false)),
            vec!["xclip", "wl-copy"]
        );
        assert_eq!(
            provider_commands(super::clipboard_providers_for_os("macos", false)),
            vec!["pbcopy"]
        );
        assert_eq!(
            provider_commands(super::clipboard_providers_for_os("windows", false)),
            vec!["powershell", "clip.exe"]
        );
        assert_eq!(
            provider_commands(super::clipboard_providers_for_os("freebsd", false)),
            vec!["xclip", "wl-copy"]
        );
    }

    #[test]
    fn clipboard_copy_runner_uses_fallback_provider_after_failure() {
        let providers = super::clipboard_providers_for_os("windows", false);
        let mut attempts = Vec::new();
        let mut copied_payloads = Vec::new();

        let result = super::copy_to_clipboard_with_runner("abc123", providers, |provider, text| {
            attempts.push(provider.command.to_string());
            copied_payloads.push((provider.command.to_string(), text.to_string()));
            if provider.command == "powershell" {
                Err(anyhow!("provider unavailable"))
            } else {
                Ok(())
            }
        });

        assert!(result.is_ok(), "fallback provider should succeed");
        assert_eq!(attempts, vec!["powershell", "clip.exe"]);
        assert_eq!(
            copied_payloads,
            vec![
                ("powershell".to_string(), "abc123".to_string()),
                ("clip.exe".to_string(), "abc123".to_string())
            ]
        );
    }

    #[test]
    fn clipboard_copy_runner_reports_aggregate_failures() {
        let providers = super::clipboard_providers_for_os("linux", false);

        let result = super::copy_to_clipboard_with_runner("payload", providers, |provider, _| {
            Err(anyhow!("{} missing from PATH", provider.command))
        });

        let err = result.expect_err("all providers fail in this simulation");
        let message = err.to_string();
        assert!(message.contains("failed to copy to clipboard"));
        assert!(message.contains("tried: xclip, wl-copy"));
        assert!(message.contains("xclip: xclip missing from PATH"));
        assert!(message.contains("wl-copy: wl-copy missing from PATH"));
    }

    #[test]
    fn resolve_command_path_accepts_absolute_file_path() {
        let dir = make_temp_dir("clipboard-resolve");
        let tool = dir.join("tool-stub");
        fs::write(&tool, b"stub").expect("stub command file is written");

        let resolved = resolve_command_path(tool.to_str().expect("path should be utf-8"));
        assert_eq!(resolved, Some(tool.clone()));

        fs::remove_dir_all(dir).expect("temp dir is removed");
    }

    #[test]
    fn resolve_command_path_returns_none_for_missing_command() {
        let missing = format!(
            "petiglyph-tui-does-not-exist-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time is valid")
                .as_nanos()
        );
        assert_eq!(resolve_command_path(&missing), None);
    }
