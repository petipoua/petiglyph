    #[test]
    fn glyph_creation_tweaking_next_persists_per_image_and_advances_preview() {
        let project_dir = make_temp_dir("glyph-creation-next-per-image");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");
        let icons_dir = project_dir.join("images");
        fs::create_dir_all(&icons_dir).expect("images dir is created");
        write_test_png(&icons_dir.join("source_a.png"));
        write_test_png(&icons_dir.join("source_b.png"));

        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-glyph-creation-next-per-image".to_string(),
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
        app.home_workflow_recent_imported_source_keys =
            vec!["source_a.png".to_string(), "source_b.png".to_string()];
        app.home_workflow_import_count = 2;
        app.rebuild_home_tweak_queue_for_glyph();

        assert_eq!(glyph_tweak_continue_label(&app), "Next");
        app.animation_import_settings.threshold = 90;
        handle_key(&mut app, KeyCode::Enter).expect("first next should persist first image");
        let manifest_after_first = read_manifest(&manifest_path).expect("manifest reload succeeds");
        assert_eq!(
            manifest_after_first.threshold_overrides.get("source_a.png"),
            Some(&90)
        );
        assert!(
            !manifest_after_first
                .threshold_overrides
                .contains_key("source_b.png")
        );
        assert_eq!(app.home_workflow_tweak_source_index, 1);
        assert!(matches!(
            app.home_workflow,
            HomeWorkflow::Tweaking(HomeCreationKind::Glyph)
        ));
        assert_eq!(glyph_tweak_continue_label(&app), "Finish");

        app.animation_import_settings.threshold = 101;
        handle_key(&mut app, KeyCode::Enter).expect("second next should finish workflow");
        let manifest_after_second =
            read_manifest(&manifest_path).expect("manifest reload succeeds");
        assert_eq!(
            manifest_after_second
                .threshold_overrides
                .get("source_b.png"),
            Some(&101)
        );
        assert!(matches!(app.home_workflow, HomeWorkflow::Launcher));
        assert_eq!(app.view, AppView::Glyphs);
        assert_eq!(
            app.glyphs
                .iter()
                .filter(|glyph| glyph.glyph.composition_tile.is_none())
                .count(),
            2,
            "finishing the final preview should reload created glyphs into the Glyphs panel"
        );
    }

    #[test]
    fn glyph_creation_tweaking_progress_clamps_after_final_preview() {
        let project_dir = make_temp_dir("glyph-creation-progress-clamps");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");
        let icons_dir = project_dir.join("images");
        fs::create_dir_all(&icons_dir).expect("images dir is created");

        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-glyph-creation-progress-clamps".to_string(),
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
        app.home_workflow_tweak_source_queue = (0..20)
            .map(|index| format!("source_{index:02}.png"))
            .collect();
        app.home_workflow_tweak_source_index = 20;

        assert_eq!(glyph_tweak_progress_label(&app), "Image 20/20  ");
    }

    #[test]
    fn glyph_creation_tweaking_single_preview_uses_finish_label() {
        let project_dir = make_temp_dir("glyph-creation-single-preview-finish-label");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");
        let icons_dir = project_dir.join("images");
        fs::create_dir_all(&icons_dir).expect("images dir is created");
        write_test_png(&icons_dir.join("source.png"));

        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-glyph-creation-single-preview-finish-label".to_string(),
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
        app.home_workflow = HomeWorkflow::Tweaking(HomeCreationKind::Glyph);
        app.home_workflow_recent_imported_source_keys = vec!["source.png".to_string()];
        app.home_workflow_import_count = 1;
        app.rebuild_home_tweak_queue_for_glyph();

        assert_eq!(glyph_tweak_continue_label(&app), "Finish");
    }

    #[test]
    fn glyph_creation_multi_image_enter_on_finish_completes_full_keyboard_flow() {
        let project_dir = make_temp_dir("glyph-creation-multi-image-keyboard-finish");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");
        let icons_dir = project_dir.join("images");
        let external_dir = project_dir.join("external");
        fs::create_dir_all(&icons_dir).expect("images dir is created");
        fs::create_dir_all(&external_dir).expect("external dir is created");

        let source_a = external_dir.join("source_a.png");
        let source_b = external_dir.join("source_b.png");
        write_test_png(&source_a);
        write_test_png(&source_b);

        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-glyph-creation-multi-image-keyboard-finish".to_string(),
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

        handle_paste_event_for_test(
            &mut app,
            &format!("{}\n{}", source_a.display(), source_b.display()),
        )
        .expect("drop/paste import should succeed");
        drain_background_tasks(&mut app);

        handle_key(&mut app, KeyCode::Enter).expect("enter should advance from import to tweak");
        assert!(matches!(
            app.home_workflow,
            HomeWorkflow::Tweaking(HomeCreationKind::Glyph)
        ));
        assert_eq!(
            app.animation_import_settings.focus,
            AnimationImportSettingsFocus::Continue
        );
        assert_eq!(glyph_tweak_continue_label(&app), "Next");

        handle_key(&mut app, KeyCode::Enter)
            .expect("enter on Next should advance to final preview");
        assert!(matches!(
            app.home_workflow,
            HomeWorkflow::Tweaking(HomeCreationKind::Glyph)
        ));
        assert_eq!(
            app.animation_import_settings.focus,
            AnimationImportSettingsFocus::Continue
        );
        assert_eq!(glyph_tweak_continue_label(&app), "Finish");

        handle_key(&mut app, KeyCode::Enter).expect("enter on Finish should complete the workflow");

        assert!(matches!(app.home_workflow, HomeWorkflow::Launcher));
        assert_eq!(app.view, AppView::Glyphs);
        assert_eq!(
            app.glyphs
                .iter()
                .filter(|glyph| glyph.glyph.composition_tile.is_none())
                .count(),
            2,
            "finish should close the popup and load both created glyphs"
        );
    }

    #[test]
    fn glyph_creation_finish_closes_workflow_when_final_reload_reports_error() {
        let project_dir = make_temp_dir("glyph-creation-finish-reload-error");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");
        let icons_dir = project_dir.join("images");
        fs::create_dir_all(&icons_dir).expect("images dir is created");
        fs::write(icons_dir.join("source.png"), b"not an image").expect("invalid png is written");

        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-glyph-creation-finish-reload-error".to_string(),
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
        app.home_workflow = HomeWorkflow::Tweaking(HomeCreationKind::Glyph);
        app.home_workflow_recent_imported_source_keys = vec!["source.png".to_string()];
        app.home_workflow_import_count = 1;
        app.rebuild_home_tweak_queue_for_glyph();
        app.animation_import_settings.focus = AnimationImportSettingsFocus::Continue;

        handle_key(&mut app, KeyCode::Enter)
            .expect("finish should not keep the workflow open on reload errors");

        assert!(matches!(app.home_workflow, HomeWorkflow::Launcher));
        assert_eq!(app.view, AppView::Glyphs);
        assert!(
            app.status
                .as_deref()
                .is_some_and(|status| status.contains("failed")),
            "reload error should be surfaced after closing workflow, got {:?}",
            app.status
        );
    }

    #[test]
    fn glyph_creation_finish_adds_reviewed_glyphs_when_full_reload_hits_bad_existing_source() {
        let project_dir = make_temp_dir("glyph-creation-finish-partial-reload");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");
        let icons_dir = project_dir.join("images");
        let external_dir = project_dir.join("external");
        fs::create_dir_all(&icons_dir).expect("images dir is created");
        fs::create_dir_all(&external_dir).expect("external dir is created");
        fs::write(icons_dir.join("broken.png"), b"not an image").expect("bad source is written");
        let source = external_dir.join("source.png");
        write_test_png(&source);

        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-glyph-creation-finish-partial-reload".to_string(),
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

        handle_paste_event_for_test(&mut app, &source.display().to_string())
            .expect("drop/paste import should succeed");
        drain_background_tasks(&mut app);
        handle_key(&mut app, KeyCode::Enter).expect("enter should advance from import to tweak");
        assert_eq!(glyph_tweak_continue_label(&app), "Finish");

        handle_key(&mut app, KeyCode::Enter)
            .expect("finish should switch to glyphs and load reviewed glyphs");

        assert!(matches!(app.home_workflow, HomeWorkflow::Launcher));
        assert_eq!(app.view, AppView::Glyphs);
        assert!(
            app.glyphs
                .iter()
                .any(|glyph| glyph.glyph.source_parent_key == "source.png"),
            "reviewed source should be added to Glyphs even when another source fails"
        );
        assert!(
            app.status
                .as_deref()
                .is_some_and(|status| status.contains("loaded reviewed glyphs")),
            "status should mention fallback reviewed glyph loading, got {:?}",
            app.status
        );
    }

    #[test]
    fn glyph_creation_tweaking_next_finishes_when_index_is_already_at_end() {
        let project_dir = make_temp_dir("glyph-creation-next-at-end");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");
        let icons_dir = project_dir.join("images");
        fs::create_dir_all(&icons_dir).expect("images dir is created");
        write_test_png(&icons_dir.join("source.png"));

        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-glyph-creation-next-at-end".to_string(),
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
        app.home_workflow = HomeWorkflow::Tweaking(HomeCreationKind::Glyph);
        app.home_workflow_recent_imported_source_keys = vec!["source.png".to_string()];
        app.home_workflow_import_count = 1;
        app.rebuild_home_tweak_queue_for_glyph();
        app.home_workflow_tweak_source_index = 1;
        app.animation_import_settings.focus = AnimationImportSettingsFocus::Continue;

        handle_key(&mut app, KeyCode::Enter).expect("next at end should finish workflow");

        assert!(matches!(app.home_workflow, HomeWorkflow::Launcher));
        assert_eq!(app.view, AppView::Glyphs);
        assert!(
            app.glyphs
                .iter()
                .any(|glyph| glyph.glyph.source_parent_key == "source.png"),
            "finishing from an end index should still reload glyphs"
        );
    }

    #[test]
    fn glyph_creation_tweaking_enter_on_continue_finishes_final_preview() {
        let project_dir = make_temp_dir("glyph-creation-continue-enter-finishes");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");
        let icons_dir = project_dir.join("images");
        fs::create_dir_all(&icons_dir).expect("images dir is created");
        write_test_png(&icons_dir.join("source_a.png"));
        write_test_png(&icons_dir.join("source_b.png"));

        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-glyph-creation-continue-enter-finishes".to_string(),
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
        app.home_workflow_recent_imported_source_keys =
            vec!["source_a.png".to_string(), "source_b.png".to_string()];
        app.home_workflow_import_count = 2;
        app.rebuild_home_tweak_queue_for_glyph();
        app.home_workflow_tweak_source_index = 1;
        app.animation_import_settings.focus = AnimationImportSettingsFocus::Continue;
        app.animation_import_settings.threshold = 99;

        handle_key(&mut app, KeyCode::Enter)
            .expect("enter on final continue control should finish workflow");

        let manifest_after = read_manifest(&manifest_path).expect("manifest reload succeeds");
        assert_eq!(
            manifest_after.threshold_overrides.get("source_b.png"),
            Some(&99)
        );
        assert!(matches!(app.home_workflow, HomeWorkflow::Launcher));
        assert_eq!(app.view, AppView::Glyphs);
        assert_eq!(
            app.glyphs
                .iter()
                .filter(|glyph| glyph.glyph.composition_tile.is_none())
                .count(),
            2,
            "finishing from continue focus should reload glyph rows"
        );
    }

    #[test]
    fn glyph_creation_tweaking_enter_press_event_on_continue_finishes_final_preview() {
        let project_dir = make_temp_dir("glyph-creation-continue-press-event-finishes");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");
        let icons_dir = project_dir.join("images");
        fs::create_dir_all(&icons_dir).expect("images dir is created");
        write_test_png(&icons_dir.join("source_a.png"));
        write_test_png(&icons_dir.join("source_b.png"));

        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-glyph-creation-continue-press-event-finishes".to_string(),
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
        app.home_workflow_recent_imported_source_keys =
            vec!["source_a.png".to_string(), "source_b.png".to_string()];
        app.home_workflow_import_count = 2;
        app.rebuild_home_tweak_queue_for_glyph();
        app.home_workflow_tweak_source_index = 1;
        app.animation_import_settings.focus = AnimationImportSettingsFocus::Continue;
        app.animation_import_settings.threshold = 99;

        handle_key_event_for_test(
            &mut app,
            KeyEvent::new(KeyCode::Enter, crossterm::event::KeyModifiers::NONE),
        )
        .expect("press event on final continue should finish workflow");

        let manifest_after = read_manifest(&manifest_path).expect("manifest reload succeeds");
        assert_eq!(
            manifest_after.threshold_overrides.get("source_b.png"),
            Some(&99)
        );
        assert!(matches!(app.home_workflow, HomeWorkflow::Launcher));
        assert_eq!(app.view, AppView::Glyphs);
    }

    #[test]
    fn glyph_creation_tweaking_twentieth_next_finishes_without_extra_step() {
        let project_dir = make_temp_dir("glyph-creation-twentieth-next-finishes");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");
        let icons_dir = project_dir.join("images");
        fs::create_dir_all(&icons_dir).expect("images dir is created");
        let source_keys = (1..=20)
            .map(|index| format!("triangle-{index:03}.png"))
            .collect::<Vec<_>>();
        for source_key in &source_keys {
            write_test_png(&icons_dir.join(source_key));
        }

        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-glyph-creation-twentieth-next-finishes".to_string(),
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
        app.home_workflow_recent_imported_source_keys = source_keys;
        app.home_workflow_import_count = 20;
        app.rebuild_home_tweak_queue_for_glyph();
        app.home_workflow_tweak_source_index = 19;
        app.animation_import_settings.focus = AnimationImportSettingsFocus::Continue;
        app.animation_import_settings.threshold = 77;

        assert_eq!(glyph_tweak_progress_label(&app), "Image 20/20  ");
        handle_key(&mut app, KeyCode::Enter).expect("twentieth next should finish immediately");

        let manifest_after = read_manifest(&manifest_path).expect("manifest reload succeeds");
        assert_eq!(
            manifest_after.threshold_overrides.get("triangle-020.png"),
            Some(&77)
        );
        assert!(matches!(app.home_workflow, HomeWorkflow::Launcher));
        assert_eq!(app.view, AppView::Glyphs);
        assert_eq!(
            app.glyphs
                .iter()
                .filter(|glyph| glyph.glyph.composition_tile.is_none())
                .count(),
            20,
            "final Next should close the popup and refresh all created glyph rows"
        );
    }

    #[test]
    fn glyph_creation_tweaking_enter_on_threshold_finishes_final_preview() {
        let project_dir = make_temp_dir("glyph-creation-threshold-enter-finishes");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");
        let icons_dir = project_dir.join("images");
        fs::create_dir_all(&icons_dir).expect("images dir is created");
        write_test_png(&icons_dir.join("source_a.png"));
        write_test_png(&icons_dir.join("source_b.png"));

        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-glyph-creation-threshold-enter-finishes".to_string(),
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
        app.home_workflow_recent_imported_source_keys =
            vec!["source_a.png".to_string(), "source_b.png".to_string()];
        app.home_workflow_import_count = 2;
        app.rebuild_home_tweak_queue_for_glyph();
        app.home_workflow_tweak_source_index = 1;
        app.animation_import_settings.focus = AnimationImportSettingsFocus::Threshold;
        app.animation_import_settings.threshold = 99;

        handle_key(&mut app, KeyCode::Enter)
            .expect("enter on final threshold control should finish workflow");

        let manifest_after = read_manifest(&manifest_path).expect("manifest reload succeeds");
        assert_eq!(
            manifest_after.threshold_overrides.get("source_b.png"),
            Some(&99)
        );
        assert!(matches!(app.home_workflow, HomeWorkflow::Launcher));
        assert_eq!(app.view, AppView::Glyphs);
        assert_eq!(
            app.glyphs
                .iter()
                .filter(|glyph| glyph.glyph.composition_tile.is_none())
                .count(),
            2,
            "finishing from threshold focus should reload glyph rows"
        );
    }

    #[test]
    fn grid_creation_tweaking_enter_on_threshold_advances_to_grid_config() {
        let project_dir = make_temp_dir("grid-creation-threshold-enter-advances");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");
        let icons_dir = project_dir.join("images");
        fs::create_dir_all(&icons_dir).expect("images dir is created");
        write_test_png(&icons_dir.join("grid_source.png"));

        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-grid-creation-threshold-enter-advances".to_string(),
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
        app.animation_import_settings.focus = AnimationImportSettingsFocus::Threshold;

        handle_key(&mut app, KeyCode::Enter)
            .expect("enter on grid threshold should continue to grid config");

        assert!(matches!(app.home_workflow, HomeWorkflow::ConfigureGrid));
        assert!(
            app.grid_config.is_some(),
            "grid config should be initialized after continuing"
        );
    }

    #[test]
    fn animated_creation_tweaking_enter_on_frames_advances_to_animation_config() {
        let project_dir = make_temp_dir("animated-creation-frames-enter-advances");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");
        let icons_dir = project_dir.join("images");
        fs::create_dir_all(&icons_dir).expect("images dir is created");
        write_test_png(&icons_dir.join("frame_a.png"));
        write_test_png(&icons_dir.join("frame_b.png"));

        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-animated-creation-frames-enter-advances".to_string(),
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
        app.home_workflow = HomeWorkflow::Tweaking(HomeCreationKind::AnimatedGlyph);
        app.animation_selection_order = vec!["frame_a.png".to_string(), "frame_b.png".to_string()];
        app.home_workflow_import_count = 2;
        app.animation_import_settings.focus = AnimationImportSettingsFocus::FramesButton;

        handle_key(&mut app, KeyCode::Enter)
            .expect("enter on frames should continue to animation config");

        assert!(matches!(
            app.home_workflow,
            HomeWorkflow::ConfigureAnimation(AnimationType::Standard)
        ));
        assert!(matches!(
            app.glyph_tool_mode,
            GlyphToolMode::ConfigureAnimation(_)
        ));
    }

    #[test]
    fn glyph_creation_tweaking_skip_all_applies_defaults_to_remaining_images() {
        let project_dir = make_temp_dir("glyph-creation-skip-all-defaults");
        let manifest_path = project_dir.join("petiglyph.toml");
        let mut manifest = Manifest::default();
        manifest
            .threshold_overrides
            .insert("source_b.png".to_string(), 118);
        write_manifest(&manifest_path, &manifest).expect("manifest is written");
        let icons_dir = project_dir.join("images");
        fs::create_dir_all(&icons_dir).expect("images dir is created");
        write_test_png(&icons_dir.join("source_a.png"));
        write_test_png(&icons_dir.join("source_b.png"));

        let mut threshold_overrides = BTreeMap::new();
        threshold_overrides.insert("source_b.png".to_string(), 118);
        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-glyph-creation-skip-all-defaults".to_string(),
            input_dir: icons_dir,
            out_dir: project_dir.join("build"),
            font_name: "Petiglyph".to_string(),
            glyph_size: 8,
            base_threshold: 64,
            threshold_overrides,
            invert_overrides: BTreeMap::new(),
            compositions: BTreeMap::new(),
            animations: Vec::new(),
            codepoint_start: 0x10_0000,
        };
        let mut app = App::new(manifest_path.clone(), config);
        app.start_home_workflow(HomeCreationKind::Glyph);
        app.home_workflow = HomeWorkflow::Tweaking(HomeCreationKind::Glyph);
        app.home_workflow_recent_imported_source_keys =
            vec!["source_a.png".to_string(), "source_b.png".to_string()];
        app.home_workflow_import_count = 2;
        app.rebuild_home_tweak_queue_for_glyph();
        app.animation_import_settings.focus = super::AnimationImportSettingsFocus::SkipAll;
        app.animation_import_settings.threshold = 97;

        handle_key(&mut app, KeyCode::Enter).expect("skip all should finish workflow");

        let manifest_after = read_manifest(&manifest_path).expect("manifest reload succeeds");
        assert!(
            !manifest_after
                .threshold_overrides
                .contains_key("source_a.png")
        );
        assert!(
            !manifest_after
                .threshold_overrides
                .contains_key("source_b.png")
        );
        assert!(matches!(app.home_workflow, HomeWorkflow::Launcher));
    }

    #[test]
    fn glyph_creation_tweaking_focus_skips_hidden_animation_controls() {
        let mut settings = AnimationImportSettingsState {
            focus: AnimationImportSettingsFocus::Threshold,
            ..AnimationImportSettingsState::default()
        };

        move_import_settings_focus(&mut settings, HomeCreationKind::Glyph, 1);
        assert_eq!(settings.focus, AnimationImportSettingsFocus::Continue);
        move_import_settings_focus(&mut settings, HomeCreationKind::Glyph, 1);
        assert_eq!(settings.focus, AnimationImportSettingsFocus::Back);
        move_import_settings_focus(&mut settings, HomeCreationKind::Glyph, 1);
        assert_eq!(settings.focus, AnimationImportSettingsFocus::SkipAll);
        move_import_settings_focus(&mut settings, HomeCreationKind::Glyph, -1);
        assert_eq!(settings.focus, AnimationImportSettingsFocus::Back);
    }

    #[test]
    fn glyph_creation_tweaking_visible_controls_fit_popup_row() {
        let controls = import_settings_visible_focuses(HomeCreationKind::Glyph);
        assert_eq!(
            controls,
            vec![
                AnimationImportSettingsFocus::GrayscaleToggle,
                AnimationImportSettingsFocus::GrayscaleOptionsButton,
                AnimationImportSettingsFocus::Threshold,
                AnimationImportSettingsFocus::Continue,
                AnimationImportSettingsFocus::Back,
                AnimationImportSettingsFocus::SkipAll,
            ]
        );
        let button_width = 18usize;
        let gap_width = 2usize;
        let rendered_width =
            controls.len() * button_width + controls.len().saturating_sub(1) * gap_width;
        assert!(
            rendered_width <= 118,
            "standard glyph tweak controls should fit inside the popup body"
        );
    }

    fn render_area_buffer<F>(width: u16, height: u16, draw: F) -> ratatui::buffer::Buffer
    where
        F: FnOnce(&mut ratatui::Frame),
    {
        let backend = ratatui::backend::TestBackend::new(width, height);
        let mut terminal = ratatui::Terminal::new(backend).expect("test terminal should create");
        terminal.draw(draw).expect("area should render");
        terminal.backend().buffer().clone()
    }

    fn find_text_in_buffer(
        buffer: &ratatui::buffer::Buffer,
        needle: &str,
    ) -> Option<(u16, u16)> {
        for y in 0..buffer.area.height {
            let mut row = String::with_capacity(buffer.area.width as usize);
            for x in 0..buffer.area.width {
                row.push_str(buffer[(x, y)].symbol());
            }
            if let Some(index) = row.find(needle) {
                return Some((index as u16, y));
            }
        }
        None
    }

    fn buffer_row(buffer: &ratatui::buffer::Buffer, y: u16) -> String {
        let mut row = String::with_capacity(buffer.area.width as usize);
        for x in 0..buffer.area.width {
            row.push_str(buffer[(x, y)].symbol());
        }
        row
    }

    #[test]
    fn glyph_creation_tweak_controls_render_as_home_style_buttons() {
        let project_dir = make_temp_dir("glyph-workflow-thin-buttons");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");
        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-glyph-workflow-thin-buttons".to_string(),
            input_dir: project_dir.join("images"),
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
        app.home_workflow = HomeWorkflow::Tweaking(HomeCreationKind::Glyph);
        app.animation_import_settings.focus = AnimationImportSettingsFocus::GrayscaleToggle;

        let buffer = render_area_buffer(140, 12, |frame| {
            draw_animation_import_workflow_ui(
                frame,
                &app,
                ratatui::layout::Rect::new(0, 0, 140, 12),
                ratatui::style::Color::Cyan,
                ratatui::style::Color::Gray,
                HomeCreationKind::Glyph,
            );
        });
        let (x, y) = find_text_in_buffer(&buffer, " Gray: ON ").expect("gray toggle should render");
        find_text_in_buffer(&buffer, " Threshold: 64 ").expect("threshold button should render");
        let row = buffer_row(&buffer, y);
        let start = x as usize;
        let end = start + " Gray: ON ".len();
        let window_start = start.saturating_sub(2);
        let window_end = (end + 2).min(row.len());

        assert_eq!(buffer[(x, y)].style().bg, Some(ratatui::style::Color::Cyan));
        assert!(
            !row[window_start..window_end].contains('│'),
            "flat tweak button should not render boxed vertical borders: {row}"
        );
    }

    #[test]
    fn grid_creation_config_controls_render_as_home_style_buttons() {
        let project_dir = make_temp_dir("grid-workflow-thin-buttons");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");
        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-grid-workflow-thin-buttons".to_string(),
            input_dir: project_dir.join("images"),
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
        app.home_workflow = HomeWorkflow::ConfigureGrid;
        app.grid_config = Some(GridConfig {
            source_key: "grid_source.png".to_string(),
            rows: 3,
            cols: 4,
            horizontal_bleed: BleedLevel::Weak,
            vertical_bleed: BleedLevel::Off,
            focus: GridConfigFocus::Rows,
        });

        let buffer = render_area_buffer(140, 14, |frame| {
            draw_grid_config_ui(
                frame,
                &app,
                app.grid_config.as_ref().expect("grid config should exist"),
                ratatui::layout::Rect::new(0, 0, 140, 14),
                ratatui::style::Color::Cyan,
                ratatui::style::Color::Gray,
            );
        });
        let (x, y) = find_text_in_buffer(&buffer, " Rows: 3 ").expect("rows control should render");
        let row = buffer_row(&buffer, y);
        let start = x as usize;
        let end = start + " Rows: 3 ".len();
        let window_start = start.saturating_sub(2);
        let window_end = (end + 2).min(row.len());

        assert_eq!(buffer[(x, y)].style().bg, Some(ratatui::style::Color::Cyan));
        assert!(
            !row[window_start..window_end].contains('│'),
            "flat grid config button should not render boxed vertical borders: {row}"
        );
    }

    #[test]
    fn animated_grid_creation_config_controls_render_as_home_style_buttons() {
        let project_dir = make_temp_dir("animated-grid-workflow-thin-buttons");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");
        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-animated-grid-workflow-thin-buttons".to_string(),
            input_dir: project_dir.join("images"),
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
        app.home_workflow = HomeWorkflow::ConfigureAnimation(AnimationType::Grid);
        app.glyph_tool_mode = GlyphToolMode::ConfigureAnimation(AnimationConfig {
            selected_frames: vec!["frame_01.png".to_string()],
            animation_name: "runner".to_string(),
            animation_type: AnimationType::Grid,
            fps: 12,
            rows: 2,
            cols: 3,
            horizontal_bleed: BleedLevel::Weak,
            vertical_bleed: BleedLevel::Strong,
            grayscale_processing: None,
            focus: AnimationConfigFocus::Fps,
        });

        let buffer = render_area_buffer(140, 14, |frame| {
            if let GlyphToolMode::ConfigureAnimation(config) = &app.glyph_tool_mode {
                draw_animation_config_ui(
                    frame,
                    &app,
                    config,
                    ratatui::layout::Rect::new(0, 0, 140, 14),
                    ratatui::style::Color::Cyan,
                    ratatui::style::Color::Gray,
                );
            }
        });
        let (x, y) = find_text_in_buffer(&buffer, " FPS: 12 ").expect("fps control should render");
        let row = buffer_row(&buffer, y);
        let start = x as usize;
        let end = start + " FPS: 12 ".len();
        let window_start = start.saturating_sub(2);
        let window_end = (end + 2).min(row.len());

        assert_eq!(buffer[(x, y)].style().bg, Some(ratatui::style::Color::Cyan));
        assert!(
            !row[window_start..window_end].contains('│'),
            "flat animation config button should not render boxed vertical borders: {row}"
        );
    }

    #[test]
    fn glyph_creation_cancel_removes_workflow_imports() {
        let project_dir = make_temp_dir("glyph-creation-cancel-removes-imports");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");
        let icons_dir = project_dir.join("images");
        fs::create_dir_all(&icons_dir).expect("images dir is created");
        write_test_png(&icons_dir.join("kept.png"));

        let external_dir = project_dir.join("external");
        fs::create_dir_all(&external_dir).expect("external dir is created");
        let dropped = external_dir.join("dropped.png");
        write_test_png(&dropped);

        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-glyph-creation-cancel-removes-imports".to_string(),
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
        app.reload_glyphs().expect("initial glyphs load");
        let initial_count = app.glyphs.len();

        app.start_home_workflow(HomeCreationKind::Glyph);
        handle_paste_event_for_test(&mut app, &dropped.display().to_string())
            .expect("drop/paste import should succeed");
        for _ in 0..50 {
            app.poll_home_import_task();
            if app.home_import_task.is_none() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            app.home_import_task.is_none(),
            "import task should complete"
        );
        let imported_key = app
            .home_workflow_recent_imported_source_keys
            .last()
            .cloned()
            .expect("workflow should track imported key");
        let imported_path = icons_dir.join(&imported_key);
        assert!(
            imported_path.exists(),
            "workflow import should stage file in images while editing"
        );

        handle_key(&mut app, KeyCode::Char('q')).expect("q cancels workflow");

        assert!(
            !imported_path.exists(),
            "cancel should remove workflow-added files"
        );
        assert!(
            icons_dir.join("kept.png").exists(),
            "existing file should remain"
        );
        assert_eq!(app.glyphs.len(), initial_count);
        assert!(matches!(app.home_workflow, HomeWorkflow::Launcher));
    }

    #[test]
    fn glyph_creation_drop_avif_imports_png_source() {
        let project_dir = make_temp_dir("glyph-creation-drop-avif");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");
        let icons_dir = project_dir.join("images");
        fs::create_dir_all(&icons_dir).expect("images dir is created");

        let external_dir = project_dir.join("external");
        fs::create_dir_all(&external_dir).expect("external dir is created");
        let dropped = external_dir.join("dropped.avif");
        if !write_test_avif_with_ffmpeg(&dropped) {
            fs::remove_dir_all(project_dir).expect("temp project dir is removed");
            return;
        }

        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-glyph-creation-drop-avif".to_string(),
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
        handle_paste_event_for_test(&mut app, &dropped.display().to_string())
            .expect("AVIF drop should start import");
        drain_background_tasks(&mut app);

        assert!(
            icons_dir.join("dropped.png").is_file(),
            "AVIF drop should be converted into a canonical PNG source"
        );
        assert!(
            !icons_dir.join("dropped.avif").exists(),
            "raw AVIF should not be copied into images"
        );
        assert_eq!(
            app.home_workflow_recent_imported_source_keys,
            vec!["dropped.png"]
        );
        assert!(matches!(
            app.home_workflow,
            HomeWorkflow::Import(HomeCreationKind::Glyph)
        ));

        handle_key(&mut app, KeyCode::Enter).expect("enter should open glyph tweaking");
        assert!(matches!(
            app.home_workflow,
            HomeWorkflow::Tweaking(HomeCreationKind::Glyph)
        ));

        app.animation_import_settings.focus = AnimationImportSettingsFocus::Continue;
        handle_key(&mut app, KeyCode::Enter).expect("finish should complete glyph workflow");

        assert_eq!(app.view, AppView::Glyphs);
        assert!(matches!(app.home_workflow, HomeWorkflow::Launcher));
        assert_eq!(app.glyphs.len(), 1);
        assert_eq!(app.glyphs[0].glyph.source_parent_key, "dropped.png");

        fs::remove_dir_all(project_dir).expect("temp project dir is removed");
    }

    #[test]
    fn animated_creation_cancel_removes_workflow_imports() {
        let project_dir = make_temp_dir("animated-creation-cancel-removes-imports");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");
        let icons_dir = project_dir.join("images");
        fs::create_dir_all(&icons_dir).expect("images dir is created");
        write_test_png(&icons_dir.join("kept.png"));

        let external_dir = project_dir.join("external");
        fs::create_dir_all(&external_dir).expect("external dir is created");
        let dropped = external_dir.join("frame_01.png");
        write_test_png(&dropped);

        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-animated-creation-cancel-removes-imports".to_string(),
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
        app.reload_glyphs().expect("initial glyphs load");
        let initial_count = app.glyphs.len();

        app.start_home_workflow(HomeCreationKind::AnimatedGlyph);
        handle_paste_event_for_test(&mut app, &dropped.display().to_string())
            .expect("drop/paste import should succeed");
        for _ in 0..50 {
            app.poll_animation_import_task();
            if app.animation_import_task.is_none() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            app.animation_import_task.is_none(),
            "animation import task should complete"
        );
        let imported_key = app
            .home_workflow_created_source_keys
            .last()
            .cloned()
            .expect("workflow should track created frame key");
        let imported_path = icons_dir.join(&imported_key);
        assert!(
            imported_path.exists(),
            "workflow import should stage frame in images while editing"
        );

        handle_key(&mut app, KeyCode::Char('q')).expect("q cancels workflow");

        assert!(
            !imported_path.exists(),
            "cancel should remove workflow-added animated frames"
        );
        assert!(
            icons_dir.join("kept.png").exists(),
            "existing file should remain"
        );
        assert_eq!(app.glyphs.len(), initial_count);
        assert!(matches!(app.home_workflow, HomeWorkflow::Launcher));
    }

    #[test]
    fn canceled_animated_import_result_is_discarded_and_cleaned_up() {
        let project_dir = make_temp_dir("animated-import-discard-result");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");
        let icons_dir = project_dir.join("images");
        fs::create_dir_all(&icons_dir).expect("images dir is created");
        let staged = icons_dir.join("frame_01.png");
        write_test_png(&staged);

        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-animated-import-discard-result".to_string(),
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
        app.discard_next_animation_import_result = true;

        app.finish_animation_import(AnimationImportTaskOutput {
            import: DropImportResult {
                imported: 1,
                renamed: 0,
                skipped_existing: 0,
                skipped_unsupported: 0,
                skipped_missing: 0,
                imported_source_keys: vec!["frame_01.png".to_string()],
                created_source_keys: vec!["frame_01.png".to_string()],
            },
            loaded: None,
            detail_status: None,
        });

        assert!(
            !staged.exists(),
            "discarded animation result should delete staged created files"
        );
        assert!(
            app.animation_selection_order.is_empty(),
            "discarded animation result should not alter selection"
        );
        assert!(
            !app.discard_next_animation_import_result,
            "discard flag should reset after one discarded result"
        );
    }

    #[test]
    fn animated_creation_tweak_threshold_applies_to_all_selected_frames() {
        let project_dir = make_temp_dir("animated-creation-threshold-persists");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");
        let icons_dir = project_dir.join("images");
        fs::create_dir_all(&icons_dir).expect("images dir is created");
        write_test_png(&icons_dir.join("frame_1.png"));
        write_test_png(&icons_dir.join("frame_2.png"));

        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-animated-creation-threshold-persists".to_string(),
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
        app.reload_glyphs().expect("glyphs load");
        app.start_home_workflow(HomeCreationKind::AnimatedGlyph);
        app.home_workflow = HomeWorkflow::Tweaking(HomeCreationKind::AnimatedGlyph);
        app.animation_selection_order = vec!["frame_1.png".to_string(), "frame_2.png".to_string()];
        app.animation_import_settings.threshold = 103;

        continue_home_workflow_after_tweaking(&mut app, HomeCreationKind::AnimatedGlyph)
            .expect("continue should persist animation frame thresholds");

        let manifest = read_manifest(&manifest_path).expect("manifest reload succeeds");
        assert_eq!(manifest.threshold_overrides.get("frame_1.png"), Some(&103));
        assert_eq!(manifest.threshold_overrides.get("frame_2.png"), Some(&103));
        assert!(
            app.glyphs.iter().all(|glyph| {
                glyph.working_threshold == 103 && glyph.saved_threshold == Some(103)
            })
        );
    }

    #[test]
    fn tweaking_grayscale_knobs_change_live_test_image_output() {
        let project_dir = make_temp_dir("tweaking-live-grayscale-output");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");
        let icons_dir = project_dir.join("images");
        fs::create_dir_all(&icons_dir).expect("images dir is created");

        let source_path = icons_dir.join("source.png");
        let mut image = RgbaImage::from_pixel(8, 8, Rgba([0, 0, 0, 0]));
        for y in 2..6 {
            for x in 2..6 {
                let shade = if x < 4 { 80 } else { 180 };
                image.put_pixel(x, y, Rgba([shade, shade, shade, 255]));
            }
        }
        image.save(&source_path).expect("source image is written");

        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-tweaking-live-grayscale-output".to_string(),
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
        app.home_workflow = HomeWorkflow::Tweaking(HomeCreationKind::Glyph);
        app.home_workflow_recent_imported_source_keys = vec!["source.png".to_string()];
        app.animation_import_settings.threshold = 180;
        app.animation_import_settings.grayscale_enabled = true;

        app.animation_import_settings.grayscale_options.brightness = -80;
        let darker = app
            .render_test_image_for_source("source.png")
            .expect("darker render succeeds")
            .expect("darker render exists")
            .0;

        app.animation_import_settings.grayscale_options.brightness = 80;
        let brighter = app
            .render_test_image_for_source("source.png")
            .expect("brighter render succeeds")
            .expect("brighter render exists")
            .0;

        assert_ne!(
            darker.as_raw(),
            brighter.as_raw(),
            "live grayscale knob changes should affect test-image output"
        );
    }

    #[test]
    fn create_workflow_tweaking_popup_previews_jpg_sources() {
        let project_dir = make_temp_dir("create-workflow-jpg-preview");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");
        let icons_dir = project_dir.join("images");
        fs::create_dir_all(&icons_dir).expect("images dir is created");
        let external_dir = project_dir.join("external");
        fs::create_dir_all(&external_dir).expect("external dir is created");
        let dropped = external_dir.join("source.jpg");
        write_test_jpg(&dropped);

        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-create-workflow-jpg-preview".to_string(),
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
        handle_paste_event_for_test(&mut app, &dropped.display().to_string())
            .expect("jpg drop/paste import should succeed");
        drain_background_tasks(&mut app);
        app.home_workflow = HomeWorkflow::Tweaking(HomeCreationKind::Glyph);

        let (title, lines) = home_workflow_preview_lines(&app, HomeCreationKind::Glyph, 16, 16);
        let rendered = format!("{lines:?}");

        assert_eq!(title, "Source: source.jpg");
        assert!(
            !rendered.contains("Preview not available yet"),
            "jpg source should render a live preview in the tweaking popup"
        );
        assert!(
            rendered.contains("█") || rendered.contains("▄") || rendered.contains("▀"),
            "jpg preview should contain rendered semi-block glyphs"
        );
        assert!(
            lines
                .last()
                .is_some_and(|line| format!("{line:?}").contains("█")
                    || format!("{line:?}").contains("▄")
                    || format!("{line:?}").contains("▀")),
            "jpg preview should not leave blank rows after the rendered glyph"
        );
    }

    #[test]
    fn create_workflow_tweaking_popup_previews_png_renamed_as_jpg() {
        let project_dir = make_temp_dir("create-workflow-renamed-png-preview");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");
        let icons_dir = project_dir.join("images");
        fs::create_dir_all(&icons_dir).expect("images dir is created");
        let external_dir = project_dir.join("external");
        fs::create_dir_all(&external_dir).expect("external dir is created");
        let source_png = external_dir.join("source.png");
        let dropped = external_dir.join("source.jpg");
        write_test_png(&source_png);
        fs::rename(&source_png, &dropped).expect("png fixture is renamed as jpg");

        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-create-workflow-renamed-png-preview".to_string(),
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
        handle_paste_event_for_test(&mut app, &dropped.display().to_string())
            .expect("renamed png drop/paste import should succeed");
        drain_background_tasks(&mut app);
        app.home_workflow = HomeWorkflow::Tweaking(HomeCreationKind::Glyph);

        let (title, lines) = home_workflow_preview_lines(&app, HomeCreationKind::Glyph, 16, 16);
        let rendered = format!("{lines:?}");

        assert_eq!(title, "Source: source.jpg");
        assert!(
            !rendered.contains("Preview not available yet"),
            "png bytes with a jpg extension should still render a live preview"
        );
        assert!(
            rendered.contains("█") || rendered.contains("▄") || rendered.contains("▀"),
            "renamed png preview should contain rendered semi-block glyphs"
        );
        assert!(
            lines
                .last()
                .is_some_and(|line| format!("{line:?}").contains("█")
                    || format!("{line:?}").contains("▄")
                    || format!("{line:?}").contains("▀")),
            "renamed png preview should not leave blank rows after the rendered glyph"
        );
    }

    #[test]
    fn create_workflow_tweaking_popup_previews_svg_sources() {
        let project_dir = make_temp_dir("create-workflow-svg-preview");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");
        let icons_dir = project_dir.join("images");
        fs::create_dir_all(&icons_dir).expect("images dir is created");
        let external_dir = project_dir.join("external");
        fs::create_dir_all(&external_dir).expect("external dir is created");
        let dropped = external_dir.join("source.svg");
        write_test_svg(&dropped);

        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-create-workflow-svg-preview".to_string(),
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
        handle_paste_event_for_test(&mut app, &dropped.display().to_string())
            .expect("svg drop/paste import should succeed");
        drain_background_tasks(&mut app);
        app.home_workflow = HomeWorkflow::Tweaking(HomeCreationKind::Glyph);

        let (title, lines) = home_workflow_preview_lines(&app, HomeCreationKind::Glyph, 16, 16);
        let rendered = format!("{lines:?}");

        assert_eq!(title, "Source: source.svg");
        assert!(
            !rendered.contains("Preview not available yet"),
            "svg source should render a live preview in the tweaking popup"
        );
        assert!(
            rendered.contains("█") || rendered.contains("▄") || rendered.contains("▀"),
            "svg preview should contain rendered semi-block glyphs"
        );
        assert!(
            lines
                .last()
                .is_some_and(|line| format!("{line:?}").contains("█")
                    || format!("{line:?}").contains("▄")
                    || format!("{line:?}").contains("▀")),
            "svg preview should not leave blank rows after the rendered glyph"
        );
    }

    #[test]
    fn create_workflow_tweaking_popup_previews_copilot_svg_fixture() {
        let fixture =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-assets/images/diamond-128.svg");
        assert!(fixture.is_file(), "svg fixture should exist");

        let project_dir = make_temp_dir("create-workflow-copilot-svg-preview");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");
        let icons_dir = project_dir.join("images");
        fs::create_dir_all(&icons_dir).expect("images dir is created");

        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-create-workflow-copilot-svg-preview".to_string(),
            input_dir: icons_dir,
            out_dir: project_dir.join("build"),
            font_name: "Petiglyph".to_string(),
            glyph_size: 64,
            base_threshold: 64,
            threshold_overrides: BTreeMap::new(),
            invert_overrides: BTreeMap::new(),
            compositions: BTreeMap::new(),
            animations: Vec::new(),
            codepoint_start: 0x10_0000,
        };
        let mut app = App::new(manifest_path, config);
        app.start_home_workflow(HomeCreationKind::Glyph);
        handle_paste_event_for_test(&mut app, &fixture.display().to_string())
            .expect("copilot svg drop/paste import should succeed");
        drain_background_tasks(&mut app);
        app.home_workflow = HomeWorkflow::Tweaking(HomeCreationKind::Glyph);

        let (title, lines) = home_workflow_preview_lines(&app, HomeCreationKind::Glyph, 32, 32);
        let rendered = format!("{lines:?}");

        assert_eq!(title, "Source: diamond-128.svg");
        assert!(
            !rendered.contains("Preview not available yet"),
            "svg fixture should render a live preview in the tweaking popup"
        );
        assert!(
            rendered.contains("█") || rendered.contains("▄") || rendered.contains("▀"),
            "svg fixture preview should contain rendered semi-block glyphs"
        );
        assert!(
            lines
                .last()
                .is_some_and(|line| format!("{line:?}").contains("█")
                    || format!("{line:?}").contains("▄")
                    || format!("{line:?}").contains("▀")),
            "copilot svg preview should not leave blank rows after the rendered glyph"
        );

        let (_, tall_panel_lines) =
            home_workflow_preview_lines(&app, HomeCreationKind::Glyph, 32, 96);
        assert!(
            tall_panel_lines.len() <= 32,
            "create workflow preview should fit aspect instead of stretching into all vertical space"
        );
    }
