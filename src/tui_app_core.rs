impl App {
    fn rebuild_home_tweak_queue_for_glyph(&mut self) {
        let mut seen = BTreeSet::new();
        self.home_workflow_tweak_source_queue = self
            .home_workflow_recent_imported_source_keys
            .iter()
            .filter(|source| seen.insert((*source).clone()))
            .cloned()
            .collect();
        if self.home_workflow_tweak_source_index >= self.home_workflow_tweak_source_queue.len() {
            self.home_workflow_tweak_source_index = self.home_workflow_tweak_source_queue.len();
        }
    }

    fn current_glyph_tweak_source_key(&self) -> Option<&String> {
        self.home_workflow_tweak_source_queue
            .get(self.home_workflow_tweak_source_index)
    }

    fn sync_threshold_to_current_glyph_tweak_source(&mut self) {
        let Some(source_key) = self.current_glyph_tweak_source_key() else {
            return;
        };
        self.animation_import_settings.threshold = self
            .config
            .threshold_overrides
            .get(source_key)
            .copied()
            .unwrap_or(self.config.base_threshold);
    }

    fn live_import_source_coverage(&self, source_path: &Path) -> Option<Vec<u8>> {
        let fit_mode = self.live_import_source_fit_mode(source_path);
        let key = live_preview_coverage_key(
            source_path,
            self.config.glyph_size,
            &self.animation_import_settings,
            fit_mode,
        )?;
        if let Some(cached) = self.live_preview_coverage_cache.borrow().entries.get(&key) {
            return Some(cached.clone());
        }
        let coverage = live_import_source_coverage_uncached(
            source_path,
            self.config.glyph_size,
            &self.animation_import_settings,
            fit_mode,
        )?;
        let mut cache = self.live_preview_coverage_cache.borrow_mut();
        if cache.entries.len() > 32 {
            cache.entries.clear();
        }
        cache.entries.insert(key, coverage.clone());
        Some(coverage)
    }

    fn live_import_source_fit_mode(&self, source_path: &Path) -> SourceFitMode {
        let source_key = source_path
            .strip_prefix(&self.config.input_dir)
            .unwrap_or(source_path)
            .to_string_lossy()
            .replace('\\', "/");

        if matches!(
            self.home_workflow,
            HomeWorkflow::Import(HomeCreationKind::AnimatedGlyph)
                | HomeWorkflow::Tweaking(HomeCreationKind::AnimatedGlyph)
                | HomeWorkflow::Import(HomeCreationKind::AnimatedGridGlyph)
                | HomeWorkflow::Tweaking(HomeCreationKind::AnimatedGridGlyph)
                | HomeWorkflow::ConfigureAnimation(_)
        ) && (self.animation_selection_order.contains(&source_key)
            || self.animation_selection_set.contains(&source_key))
        {
            return SourceFitMode::PreserveFrame;
        }

        if let GlyphToolMode::ConfigureAnimation(config) = &self.glyph_tool_mode
            && config.selected_frames.contains(&source_key)
        {
            return SourceFitMode::PreserveFrame;
        }

        SourceFitMode::TrimContent
    }

    fn has_imported_home_sources(&self, kind: HomeCreationKind) -> bool {
        match kind {
            HomeCreationKind::Glyph => self.home_workflow_import_count > 0,
            HomeCreationKind::Grid => self.home_workflow_grid_source_key.is_some(),
            HomeCreationKind::AnimatedGlyph | HomeCreationKind::AnimatedGridGlyph => {
                !self.animation_selection_order.is_empty()
            }
        }
    }

    fn prompt_windows_home_import_picker(&mut self, kind: HomeCreationKind) -> Result<bool> {
        if cfg!(test) || !windows_creation_workflow_uses_picker() {
            return Ok(false);
        }

        let Some(payload) = open_windows_creation_workflow_picker(kind)? else {
            self.status =
                Some("Windows file picker canceled; press Enter to open it again".to_string());
            return Ok(true);
        };

        self.home_workflow_error = None;
        self.import_dropped_images(&payload)?;
        if !is_animated_home_creation(kind) && self.home_import_task.is_some() {
            self.status = Some(
                if matches!(kind, HomeCreationKind::Grid) {
                    "loading selected image...".to_string()
                } else {
                    "loading selected images...".to_string()
                },
            );
        }
        Ok(true)
    }

    fn start_home_workflow(&mut self, kind: HomeCreationKind) {
        self.home_workflow = HomeWorkflow::Import(kind);
        self.discard_next_animation_import_result = false;
        self.queued_drop_payload = None;
        self.home_workflow_import_count = 0;
        self.home_workflow_recent_imported_source_keys.clear();
        self.home_workflow_created_source_keys.clear();
        self.home_workflow_tweak_source_queue.clear();
        self.home_workflow_tweak_source_index = 0;
        self.home_workflow_grid_source_key = None;
        self.home_workflow_grid_inline_notice = None;
        self.home_workflow_error = None;
        self.animation_import_settings = AnimationImportSettingsState::default();
        self.animation_import_settings.threshold = self.config.base_threshold;
        if matches!(
            kind,
            HomeCreationKind::AnimatedGlyph | HomeCreationKind::AnimatedGridGlyph
        ) {
            self.clear_animation_draft();
            self.glyph_tool_mode = GlyphToolMode::ImportAnimationFrames;
            self.selecting_for_animation_frames = true;
        }
        if let Err(err) = self.prompt_windows_home_import_picker(kind) {
            let detail = format_status_from_error(&self.manifest_path, &err.to_string());
            self.home_workflow_error = Some(format!("Windows file picker failed: {detail}"));
            self.status = Some(format!("Windows file picker failed: {detail}"));
        }
    }

    fn reset_home_workflow(&mut self) {
        self.home_workflow = HomeWorkflow::Launcher;
        self.discard_next_animation_import_result = false;
        self.queued_drop_payload = None;
        self.home_workflow_import_count = 0;
        self.home_workflow_recent_imported_source_keys.clear();
        self.home_workflow_created_source_keys.clear();
        self.home_workflow_tweak_source_queue.clear();
        self.home_workflow_tweak_source_index = 0;
        self.animation_import_settings = AnimationImportSettingsState::default();
        self.home_workflow_grid_source_key = None;
        self.home_workflow_grid_inline_notice = None;
        self.home_workflow_error = None;
        self.grid_config = None;
        self.selecting_for_grid = false;
        self.clear_animation_draft();
        self.glyph_tool_mode = GlyphToolMode::None;
    }

    fn complete_home_workflow_to_glyphs(&mut self) {
        if let Err(err) = self.reload_glyphs() {
            self.status = Some(format_status_from_error(
                &self.manifest_path,
                &err.to_string(),
            ));
        }
        self.reset_home_workflow();
        self.view = AppView::Glyphs;
        self.glyphs_focus = GlyphsFocus::List;
    }

    fn complete_home_glyph_creation_to_glyphs(&mut self) {
        let reviewed_source_keys = self.home_workflow_tweak_source_queue.clone();
        if let Err(err) = self.reload_glyphs() {
            let reload_status = format_status_from_error(&self.manifest_path, &err.to_string());
            if !reviewed_source_keys.is_empty() {
                match load_interactive_glyphs_for_source_keys(&self.config, &reviewed_source_keys) {
                    Ok(created_glyphs) if !created_glyphs.is_empty() => {
                        self.merge_created_glyphs(created_glyphs);
                        self.status = Some(format!(
                            "{reload_status}; loaded reviewed glyphs into Glyphs"
                        ));
                    }
                    Ok(_) => {
                        self.status = Some(reload_status);
                    }
                    Err(fallback_err) => {
                        self.status = Some(format!(
                            "{reload_status}; reviewed glyph load failed: {fallback_err}"
                        ));
                    }
                }
            } else {
                self.status = Some(reload_status);
            }
        }
        self.reset_home_workflow();
        self.view = AppView::Glyphs;
        self.glyphs_focus = GlyphsFocus::List;
    }

    fn merge_created_glyphs(&mut self, created_glyphs: Vec<InteractiveGlyph>) {
        let created_sources = created_glyphs
            .iter()
            .map(|glyph| glyph.glyph.source_parent_key.clone())
            .collect::<BTreeSet<_>>();
        self.glyphs
            .retain(|glyph| !created_sources.contains(&glyph.glyph.source_parent_key));
        self.glyphs.extend(created_glyphs);
        self.clamp_glyph_selection();
        self.live_glyph_source_count = Some(self.glyphs.len());
        self.live_glyph_source_probe_at = Some(Instant::now());
        self.live_glyph_source_probe_fingerprint =
            glyph_source_fingerprint(&self.config.input_dir).ok();
    }

    fn cancel_home_workflow(&mut self) -> Result<()> {
        if self.animation_import_task.is_some() {
            self.discard_next_animation_import_result = true;
        }
        for source_key in self.home_workflow_created_source_keys.iter().rev() {
            if source_key.is_empty() {
                continue;
            }
            let path = self.config.input_dir.join(source_key);
            if path.exists() {
                fs::remove_file(&path)
                    .with_context(|| format!("failed to remove {}", path.display()))?;
            }
        }
        self.reload_glyphs()?;
        self.reset_home_workflow();
        Ok(())
    }

    fn clear_animation_draft(&mut self) {
        self.animation_selection_order.clear();
        self.animation_selection_set.clear();
        self.animation_imported_set.clear();
        self.animation_import_settings.grayscale_editor = None;
        self.selecting_for_animation_frames = false;
        self.animation_create_pending = None;
        self.animation_create_started_at = None;
    }

    fn start_animation_create(&mut self, config: AnimationConfig) {
        if self.animation_create_pending.is_some() {
            return;
        }
        self.home_workflow_error = None;
        self.animation_create_started_at = Some(Instant::now());
        self.animation_create_pending = Some(config);
    }

    fn animation_create_in_progress(&self) -> bool {
        self.animation_create_pending.is_some()
    }

    fn animation_create_spinner_frame(&self) -> &'static str {
        let Some(started_at) = self.animation_create_started_at else {
            return ANIMATION_IMPORT_SPINNER_FRAMES[0];
        };
        let elapsed_ms = Instant::now()
            .saturating_duration_since(started_at)
            .as_millis() as u64;
        let idx = ((elapsed_ms / FONT_TASK_SPINNER_FRAME_MS) as usize)
            % ANIMATION_IMPORT_SPINNER_FRAMES.len();
        ANIMATION_IMPORT_SPINNER_FRAMES[idx]
    }

    fn poll_animation_create_pending(&mut self) -> Result<()> {
        let Some(config) = self.animation_create_pending.take() else {
            return Ok(());
        };
        self.animation_create_started_at = None;
        if let Err(err) = self.create_animation_from_config(&config) {
            self.home_workflow_error = Some(format!(
                "failed to create animation: {}",
                format_status_from_error(&self.manifest_path, &err.to_string())
            ));
            return Err(err);
        }
        Ok(())
    }

    fn start_animation_config(&mut self, animation_type: AnimationType) {
        let mut frames = self.animation_selection_order.clone();
        if frames.is_empty() {
            let mut fallback = self
                .animation_selection_set
                .iter()
                .cloned()
                .collect::<Vec<_>>();
            sort_source_keys_for_animation_frames(&mut fallback);
            frames = fallback;
        }
        sort_source_keys_for_animation_frames(&mut frames);
        let name = default_animation_name_from_frames(&self.config, &frames);
        let grayscale_processing = Some(animation_import_processing_options(
            &self.animation_import_settings,
        ));
        self.glyph_tool_mode = GlyphToolMode::ConfigureAnimation(AnimationConfig {
            selected_frames: frames,
            animation_name: name,
            animation_type,
            fps: 8,
            rows: 2,
            cols: 2,
            horizontal_bleed: BleedLevel::Weak,
            vertical_bleed: BleedLevel::Off,
            grayscale_processing,
            focus: AnimationConfigFocus::Fps,
        });
    }

    fn create_animation_from_config(&mut self, config: &AnimationConfig) -> Result<()> {
        let name = config.animation_name.trim().to_string();
        if config.selected_frames.is_empty() {
            self.status = Some("animation requires at least one frame".to_string());
            self.home_workflow_error = Some("animation requires at least one frame".to_string());
            return Ok(());
        }
        if self.config.animations.iter().any(|a| a.name == name) {
            self.status = Some(format!("animation `{name}` already exists"));
            self.home_workflow_error = Some(format!("animation `{name}` already exists"));
            return Ok(());
        }
        let mut selected_frames = config.selected_frames.clone();
        sort_source_keys_for_animation_frames(&mut selected_frames);
        let mut duplicated_for_grid_conflicts = 0usize;

        if config.animation_type == AnimationType::Grid {
            let mut resolved_frames = Vec::with_capacity(selected_frames.len());
            for frame in &selected_frames {
                let desired = CompositionDef {
                    rows: config.rows as usize,
                    cols: config.cols as usize,
                    horizontal_bleed: config.horizontal_bleed,
                    vertical_bleed: config.vertical_bleed,
                };
                if let Some(existing) = self.config.compositions.get(frame) {
                    if existing != &desired {
                        let duplicated_frame =
                            duplicate_source_key_for_grid_conflict(&self.config.input_dir, frame)?;
                        persist_composition_definition(
                            &self.manifest_path,
                            &duplicated_frame,
                            Some(desired),
                        )?;
                        resolved_frames.push(duplicated_frame);
                        duplicated_for_grid_conflicts =
                            duplicated_for_grid_conflicts.saturating_add(1);
                        continue;
                    }
                    resolved_frames.push(frame.clone());
                } else {
                    persist_composition_definition(&self.manifest_path, frame, Some(desired))?;
                    resolved_frames.push(frame.clone());
                }
            }
            selected_frames = resolved_frames;
            self.reload_config()?;
        }

        let frames = selected_frames;

        let def = AnimationDef {
            name: name.clone(),
            animation_type: config.animation_type,
            fps: config.fps,
            frames,
            rows: (config.animation_type == AnimationType::Grid).then_some(config.rows as usize),
            cols: (config.animation_type == AnimationType::Grid).then_some(config.cols as usize),
            horizontal_bleed: (config.animation_type == AnimationType::Grid)
                .then_some(config.horizontal_bleed),
            vertical_bleed: (config.animation_type == AnimationType::Grid)
                .then_some(config.vertical_bleed),
            grayscale_processing: config.grayscale_processing,
        };
        persist_animation_definition(&self.manifest_path, def)?;
        self.reload_glyphs()?;
        self.refresh_workspace_discovery()?;
        self.glyph_tool_mode = GlyphToolMode::None;
        self.clear_animation_draft();
        self.home_workflow_error = None;
        if !matches!(self.home_workflow, HomeWorkflow::Launcher) {
            self.complete_home_workflow_to_glyphs();
        }
        self.status = Some(if duplicated_for_grid_conflicts > 0 {
            format!(
                "created animation `{name}` (auto-duplicated {duplicated_for_grid_conflicts} frame(s) for grid config conflicts)"
            )
        } else {
            format!("created animation `{name}`")
        });
        Ok(())
    }

    fn update_animation_preview(&mut self) {
        if self.config.animations.is_empty() {
            self.animation_preview = None;
            return;
        }
        let Some(animation) = self.selected_animation_for_preview() else {
            self.animation_preview = None;
            return;
        };
        let now = Instant::now();
        let mut preview = self.animation_preview.clone().unwrap_or(AnimationPreview {
            animation_name: animation.name.clone(),
            frame_index: 0,
            last_frame_at: now,
        });
        if preview.animation_name != animation.name {
            preview = AnimationPreview {
                animation_name: animation.name.clone(),
                frame_index: 0,
                last_frame_at: now,
            };
        }
        step_animation_preview(&mut preview, animation, now);
        self.animation_preview = Some(preview);
    }

    fn current_project_is_installed(&self) -> bool {
        self.active_project.is_some() && self.installed_font_path.is_some()
    }

    #[cfg(test)]
    pub(crate) fn new(manifest_path: PathBuf, config: RuntimeConfig) -> Self {
        Self::new_with_overrides(manifest_path, config, TuiLaunchOverrides::default(), None)
    }

    pub(crate) fn new_workspace(
        workspace_root: PathBuf,
        initial_manifest: Option<PathBuf>,
        launch_overrides: TuiLaunchOverrides,
    ) -> Result<Self> {
        let mut app = match initial_manifest {
            Some(manifest_path) => {
                let config = load_runtime_config(
                    &manifest_path,
                    launch_overrides.input_dir.clone(),
                    None,
                    launch_overrides.threshold,
                    launch_overrides.glyph_size,
                    launch_overrides.codepoint_start.clone(),
                )?;
                Self::new_with_overrides(
                    manifest_path,
                    config,
                    launch_overrides,
                    Some(workspace_root),
                )
            }
            None => Self::new_inactive(workspace_root, launch_overrides),
        };

        app.refresh_workspace_discovery()?;
        app.refresh_pua_usage_summary();
        if app.active_project.is_some() {
            app.reload_glyphs()?;
        }
        Ok(app)
    }

    fn new_inactive(workspace_root: PathBuf, launch_overrides: TuiLaunchOverrides) -> Self {
        let manifest_path = workspace_root.join("petiglyph.toml");
        let debug_enabled = glyph_debug::debug_enabled();
        Self {
            manifest_path,
            project_dir: workspace_root.clone(),
            config: inactive_runtime_config(&workspace_root),
            workspace_root,
            projects: Vec::new(),
            active_project: None,
            selected_project: 0,
            create_input: Input::default(),
            welcome_focus: WelcomeFocus::CreateInput,
            welcome_input_editing: false,
            verbose_paths: false,
            installed_fonts: Vec::new(),
            pua_usage_summary: None,
            installed_animation_started_at: Instant::now(),
            selected_installed_font: 0,
            selected_installed_font_sub_index: 0,
            installed_font_horizontal_focus_uninstall: false,
            last_copy_notification: None,
            switch_notice: None,
            selected: 0,
            selected_visible: 0,
            glyphs: Vec::new(),
            expanded_compositions: BTreeSet::new(),
            expanded_animations: BTreeSet::new(),
            quit: false,
            status: None,
            view: AppView::Welcome,
            glyphs_focus: GlyphsFocus::List,
            grid_config: None,
            selecting_for_grid: false,
            glyph_tool_mode: GlyphToolMode::None,
            glyph_preview_control: GlyphPreviewControl::Threshold,
            live_preview_coverage_cache: RefCell::new(LivePreviewCoverageCache::default()),
            animation_selection_order: Vec::new(),
            animation_selection_set: BTreeSet::new(),
            animation_imported_set: BTreeSet::new(),
            animation_preview: None,
            selecting_for_animation_frames: false,
            home_launcher_focus: HomeLauncherFocus::CreateGlyph,
            home_workflow: HomeWorkflow::Launcher,
            home_workflow_import_count: 0,
            animation_import_settings: AnimationImportSettingsState::default(),
            home_workflow_recent_imported_source_keys: Vec::new(),
            home_workflow_created_source_keys: Vec::new(),
            home_workflow_tweak_source_queue: Vec::new(),
            home_workflow_tweak_source_index: 0,
            home_workflow_grid_source_key: None,
            home_workflow_grid_inline_notice: None,
            home_workflow_error: None,
            discard_next_animation_import_result: false,
            last_build: None,
            last_sample: None,
            installed_font_path: None,
            delete_project_confirm_selection: None,
            renaming_input: None,
            renaming_original: None,
            first_install_notice_open: false,
            launch_overrides,
            install_task: None,
            project_switch_task: None,
            animation_import_task: None,
            home_import_task: None,
            queued_drop_payload: None,
            animation_create_pending: None,
            animation_create_started_at: None,
            live_glyph_source_count: None,
            live_glyph_source_probe_fingerprint: None,
            live_glyph_source_probe_at: None,
            debug_enabled,
            debug_log_path: None,
            debug_log_lines: Vec::new(),
        }
    }

    pub(crate) fn new_with_overrides(
        manifest_path: PathBuf,
        config: RuntimeConfig,
        launch_overrides: TuiLaunchOverrides,
        workspace_root: Option<PathBuf>,
    ) -> Self {
        let project_dir = manifest_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        let workspace_root = workspace_root.unwrap_or_else(|| project_dir.clone());
        let debug_enabled = glyph_debug::debug_enabled();
        let debug_log_path = Some(glyph_debug::session_log_path(&project_dir));
        let (last_build, last_sample) = cached_build_state(&config);
        let installed_font_path =
            cached_installed_font_path(&manifest_path, &config.font_name, &config.project_id);
        Self {
            manifest_path: manifest_path.clone(),
            project_dir,
            config,
            workspace_root,
            projects: Vec::new(),
            active_project: Some(manifest_path),
            selected_project: 0,
            create_input: Input::default(),
            welcome_focus: WelcomeFocus::CreateInput,
            welcome_input_editing: false,
            verbose_paths: false,
            installed_fonts: Vec::new(),
            pua_usage_summary: None,
            installed_animation_started_at: Instant::now(),
            selected_installed_font: 0,
            selected_installed_font_sub_index: 0,
            installed_font_horizontal_focus_uninstall: false,
            last_copy_notification: None,
            switch_notice: None,
            selected: 0,
            selected_visible: 0,
            glyphs: Vec::new(),
            expanded_compositions: BTreeSet::new(),
            expanded_animations: BTreeSet::new(),
            quit: false,
            status: None,
            view: AppView::Welcome,
            glyphs_focus: GlyphsFocus::List,
            grid_config: None,
            selecting_for_grid: false,
            glyph_tool_mode: GlyphToolMode::None,
            glyph_preview_control: GlyphPreviewControl::Threshold,
            live_preview_coverage_cache: RefCell::new(LivePreviewCoverageCache::default()),
            animation_selection_order: Vec::new(),
            animation_selection_set: BTreeSet::new(),
            animation_imported_set: BTreeSet::new(),
            animation_preview: None,
            selecting_for_animation_frames: false,
            home_launcher_focus: HomeLauncherFocus::CreateGlyph,
            home_workflow: HomeWorkflow::Launcher,
            home_workflow_import_count: 0,
            animation_import_settings: AnimationImportSettingsState::default(),
            home_workflow_recent_imported_source_keys: Vec::new(),
            home_workflow_created_source_keys: Vec::new(),
            home_workflow_tweak_source_queue: Vec::new(),
            home_workflow_tweak_source_index: 0,
            home_workflow_grid_source_key: None,
            home_workflow_grid_inline_notice: None,
            home_workflow_error: None,
            discard_next_animation_import_result: false,
            last_build,
            last_sample,
            installed_font_path,
            delete_project_confirm_selection: None,
            renaming_input: None,
            renaming_original: None,
            first_install_notice_open: false,
            launch_overrides,
            install_task: None,
            project_switch_task: None,
            animation_import_task: None,
            home_import_task: None,
            queued_drop_payload: None,
            animation_create_pending: None,
            animation_create_started_at: None,
            live_glyph_source_count: None,
            live_glyph_source_probe_fingerprint: None,
            live_glyph_source_probe_at: None,
            debug_enabled,
            debug_log_path,
            debug_log_lines: Vec::new(),
        }
    }

    fn refresh_workspace_discovery(&mut self) -> Result<()> {
        self.projects = scan_projects_in_folder(&self.workspace_root)?;
        self.sync_selected_project();

        match scan_installed_petiglyph_fonts(&self.workspace_root) {
            Ok(fonts) => self.installed_fonts = fonts,
            Err(err) => {
                self.installed_fonts.clear();
                self.status = Some(format!("font scan warning: {err}"));
            }
        }
        self.sync_selected_installed_font();

        if self.projects.is_empty() {
            self.welcome_focus = WelcomeFocus::CreateInput;
            self.welcome_input_editing = false;
            if self.active_project.is_none() {
                self.status = Some(format!(
                    "no petiglyph project in {}",
                    self.workspace_root.display()
                ));
            }
        } else if self.active_project.is_none() && self.welcome_focus == WelcomeFocus::CreateInput {
            self.welcome_focus = WelcomeFocus::ProjectList;
        }

        if self.welcome_focus == WelcomeFocus::InstalledFontList && self.installed_fonts.is_empty()
        {
            self.welcome_focus = if self.active_project.is_some() {
                WelcomeFocus::InstallButton
            } else if !self.projects.is_empty() {
                WelcomeFocus::ProjectList
            } else {
                WelcomeFocus::CreateInput
            };
        }

        if self.welcome_focus == WelcomeFocus::DeleteProjectButton
            && !self.active_project_can_be_deleted()
        {
            self.welcome_focus = if self.active_project.is_some() {
                WelcomeFocus::InstallButton
            } else if !self.projects.is_empty() {
                WelcomeFocus::ProjectList
            } else {
                WelcomeFocus::CreateInput
            };
        }

        if self.active_project.is_none()
            && matches!(
                self.welcome_focus,
                WelcomeFocus::BuildButton
                    | WelcomeFocus::InstallButton
                    | WelcomeFocus::DeleteProjectButton
            )
        {
            self.welcome_focus = if self.projects.is_empty() {
                WelcomeFocus::CreateInput
            } else {
                WelcomeFocus::ProjectList
            };
        }

        Ok(())
    }

    fn refresh_pua_usage_summary(&mut self) {
        self.pua_usage_summary = supplementary_pua_usage_summary().ok();
    }

    fn sync_selected_project(&mut self) {
        if self.projects.is_empty() {
            self.selected_project = 0;
            return;
        }

        if let Some(active_project) = &self.active_project
            && let Some(idx) = self
                .projects
                .iter()
                .position(|project| &project.manifest_path == active_project)
        {
            self.selected_project = idx;
            return;
        }

        self.selected_project = self.selected_project.min(self.projects.len() - 1);
    }

    fn sync_selected_installed_font(&mut self) {
        if self.installed_fonts.is_empty() {
            self.selected_installed_font = 0;
            self.selected_installed_font_sub_index = 0;
            return;
        }

        self.selected_installed_font = self
            .selected_installed_font
            .min(self.installed_fonts.len() - 1);

        let sub_count = self.installed_font_sub_row_count(self.selected_installed_font);
        self.selected_installed_font_sub_index = self
            .selected_installed_font_sub_index
            .min(sub_count.saturating_sub(1));
    }

    fn installed_font_sub_row_count(&self, idx: usize) -> usize {
        let font = match self.installed_fonts.get(idx) {
            Some(f) => f,
            None => return 0,
        };
        // 1 (Title) + number of sample blocks + animation rows
        1 + font.blocks.len() + font.animation_rows.len()
    }

    fn visible_glyph_rows(&self) -> Vec<VisibleGlyphRow> {
        let mut rows = Vec::new();

        let animation_frame_sources = animation_frame_parent_sources(&self.config);
        for (animation_idx, animation) in self.config.animations.iter().enumerate() {
            rows.push(VisibleGlyphRow::AnimationParent { animation_idx });
            if self.expanded_animations.contains(&animation.name) {
                for (frame_idx, source_key) in animation.frames.iter().enumerate() {
                    let glyph_idx = self.glyphs.iter().position(|glyph| {
                        glyph_matches_animation_row_frame(glyph, animation, source_key)
                    });
                    rows.push(VisibleGlyphRow::AnimationFrame {
                        animation_idx,
                        frame_idx,
                        source_key: source_key.clone(),
                        glyph_idx,
                    });
                }
            }
        }

        let mut idx = 0usize;
        while idx < self.glyphs.len() {
            let glyph = &self.glyphs[idx];
            if animation_frame_sources.contains(&glyph.glyph.source_parent_key)
                || animation_frame_sources.contains(&glyph.glyph.source_key)
            {
                idx += 1;
                continue;
            }
            if let Some(tile) = &glyph.glyph.composition_tile {
                if tile.row == 0 && tile.col == 0 {
                    let source_key = glyph.glyph.source_parent_key.clone();
                    if animation_frame_sources.contains(&source_key) {
                        idx = idx.saturating_add(tile.rows.saturating_mul(tile.cols).max(1));
                        continue;
                    }
                    rows.push(VisibleGlyphRow::CompositionParent {
                        source_key: source_key.clone(),
                        rows: tile.rows,
                        cols: tile.cols,
                        first_child_idx: idx,
                    });
                    let span = tile.rows.saturating_mul(tile.cols);
                    if self.expanded_compositions.contains(&source_key) {
                        for offset in 0..span {
                            if let Some(child) = self.glyphs.get(idx + offset)
                                && let Some(child_tile) = &child.glyph.composition_tile
                            {
                                rows.push(VisibleGlyphRow::CompositionChild {
                                    glyph_idx: idx + offset,
                                    source_key: source_key.clone(),
                                    row: child_tile.row,
                                    col: child_tile.col,
                                });
                            }
                        }
                    }
                    idx = idx.saturating_add(span.max(1));
                    continue;
                }
                idx += 1;
                continue;
            }

            rows.push(VisibleGlyphRow::Single { glyph_idx: idx });
            idx += 1;
        }
        rows
    }

    fn selected_animation_for_preview(&self) -> Option<&AnimationDef> {
        let row = self.selected_visible_row()?;
        match row {
            VisibleGlyphRow::AnimationParent { animation_idx }
            | VisibleGlyphRow::AnimationFrame { animation_idx, .. } => {
                self.config.animations.get(animation_idx)
            }
            _ => {
                let source_key = selected_source_parent_key(self)?;
                self.config.animations.iter().find(|a| {
                    a.frames.iter().any(|frame| frame == &source_key)
                        || a.frames
                            .iter()
                            .any(|frame| frame.starts_with(&format!("{source_key}#compose:")))
                })
            }
        }
    }

    fn clamp_glyph_selection(&mut self) {
        let rows = self.visible_glyph_rows();
        if rows.is_empty() {
            self.selected_visible = 0;
            self.selected = 0;
            return;
        }

        self.selected_visible = self.selected_visible.min(rows.len() - 1);
        self.selected = match &rows[self.selected_visible] {
            VisibleGlyphRow::AnimationParent { animation_idx } => self
                .config
                .animations
                .get(*animation_idx)
                .and_then(|animation| {
                    animation.frames.first().and_then(|frame| {
                        self.glyphs.iter().position(|glyph| {
                            glyph_matches_animation_row_frame(glyph, animation, frame)
                        })
                    })
                })
                .unwrap_or(0),
            VisibleGlyphRow::AnimationFrame { glyph_idx, .. } => glyph_idx.unwrap_or(0),
            VisibleGlyphRow::Single { glyph_idx }
            | VisibleGlyphRow::CompositionChild { glyph_idx, .. } => *glyph_idx,
            VisibleGlyphRow::CompositionParent {
                first_child_idx, ..
            } => *first_child_idx,
        };
    }

    fn normalize_glyphs_focus(&mut self) {
        if self.active_project.is_none() {
            self.glyphs_focus = GlyphsFocus::List;
            return;
        }
        if self.visible_glyph_rows().is_empty() {
            self.glyphs_focus = GlyphsFocus::List;
        }
    }

    fn selected_visible_row(&self) -> Option<VisibleGlyphRow> {
        let rows = self.visible_glyph_rows();
        if rows.is_empty() {
            return None;
        }
        rows.get(self.selected_visible.min(rows.len() - 1)).cloned()
    }

    fn toggle_selected_composition_expansion(&mut self) {
        let Some(row) = self.selected_visible_row() else {
            return;
        };
        let source_key = match row {
            VisibleGlyphRow::CompositionParent { source_key, .. }
            | VisibleGlyphRow::CompositionChild { source_key, .. } => source_key,
            VisibleGlyphRow::AnimationParent { animation_idx } => {
                if let Some(animation) = self.config.animations.get(animation_idx) {
                    if !self.expanded_animations.insert(animation.name.clone()) {
                        self.expanded_animations.remove(&animation.name);
                    }
                }
                self.clamp_glyph_selection();
                return;
            }
            VisibleGlyphRow::AnimationFrame { .. } => return,
            VisibleGlyphRow::Single { .. } => return,
        };

        if !self.expanded_compositions.insert(source_key.clone()) {
            self.expanded_compositions.remove(&source_key);
        }
        self.clamp_glyph_selection();
    }

    fn active_project_can_be_deleted(&self) -> bool {
        let Some(active_manifest) = &self.active_project else {
            return false;
        };
        let Some(project_dir) = active_manifest.parent() else {
            return false;
        };

        if project_dir == self.workspace_root {
            return false;
        }
        if !project_dir.starts_with(&self.workspace_root) {
            return false;
        }

        self.projects
            .iter()
            .any(|project| project.manifest_path == *active_manifest)
    }

    fn cancel_delete_project_confirmation(&mut self) {
        self.delete_project_confirm_selection = None;
        self.status = Some("project deletion canceled".to_string());
    }

    fn begin_delete_project_confirmation(&mut self) -> Result<()> {
        if self.install_in_progress() {
            self.status = Some(
                "a background task is in progress; wait before deleting a project".to_string(),
            );
            return Ok(());
        }
        if !self.active_project_can_be_deleted() {
            self.status =
                Some("only nested workspace projects can be deleted from Home".to_string());
            return Ok(());
        }
        self.welcome_input_editing = false;
        self.delete_project_confirm_selection = Some(DELETE_CONFIRM_CANCEL_INDEX);
        self.status = None;
        Ok(())
    }

    fn confirm_delete_project(&mut self) -> Result<()> {
        let Some(active_manifest) = self.active_project.clone() else {
            self.status = Some("no active project selected".to_string());
            self.delete_project_confirm_selection = None;
            return Ok(());
        };

        if !self.active_project_can_be_deleted() {
            self.status = Some("active project is not deletable from this workspace".to_string());
            self.delete_project_confirm_selection = None;
            return Ok(());
        }

        let deleted_project_name = active_manifest
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .unwrap_or("project")
            .to_string();

        delete_project_for_manifest(&active_manifest)?;
        self.delete_project_confirm_selection = None;
        self.active_project = None;
        self.manifest_path = self.workspace_root.join("petiglyph.toml");
        self.project_dir = self.workspace_root.clone();
        self.reload_config()?;
        self.glyphs.clear();
        self.selected = 0;
        self.selected_visible = 0;
        self.expanded_compositions.clear();
        self.expanded_animations.clear();
        self.refresh_workspace_discovery()?;
        self.welcome_focus = if self.projects.is_empty() {
            WelcomeFocus::CreateInput
        } else {
            WelcomeFocus::ProjectList
        };
        self.status = Some(format!("deleted project `{deleted_project_name}`"));
        Ok(())
    }

    fn confirm_rename(&mut self) -> Result<()> {
        let Some(input) = self.renaming_input.take() else {
            return Ok(());
        };
        let new_name = input.value().trim().to_string();
        self.renaming_original = None;

        if new_name.is_empty() {
            self.status = Some("project name cannot be empty; rename canceled".to_string());
            return Ok(());
        }

        let old_dir = self.project_dir.clone();
        if old_dir == self.workspace_root {
            self.status = Some("refusing to rename the workspace root directory".to_string());
            return Ok(());
        }

        let new_dir = self.workspace_root.join(&new_name);
        if new_dir.exists() {
            self.status = Some(format!("directory already exists: {}", new_dir.display()));
            return Ok(());
        }

        let old_name = self.config.font_name.clone();
        fs::rename(&old_dir, &new_dir).with_context(|| {
            format!(
                "failed to rename {} to {}",
                old_dir.display(),
                new_dir.display()
            )
        })?;

        let new_manifest_path = new_dir.join("petiglyph.toml");
        let mut manifest = read_manifest(&new_manifest_path)?;
        manifest.font_name = new_name.clone();
        write_manifest(&new_manifest_path, &manifest)?;

        let out_dir = new_dir.join(&manifest.out_dir);
        let old_ttf = out_dir.join(format!("{old_name}.ttf"));
        let new_ttf = out_dir.join(format!("{new_name}.ttf"));
        if old_ttf.is_file() && !new_ttf.exists() {
            fs::rename(&old_ttf, &new_ttf).with_context(|| {
                format!(
                    "failed to rename {} to {}",
                    old_ttf.display(),
                    new_ttf.display()
                )
            })?;
        }
        let old_bdf = out_dir.join(format!("{old_name}.bdf"));
        let new_bdf = out_dir.join(format!("{new_name}.bdf"));
        if old_bdf.is_file() && !new_bdf.exists() {
            fs::rename(&old_bdf, &new_bdf).with_context(|| {
                format!(
                    "failed to rename {} to {}",
                    old_bdf.display(),
                    new_bdf.display()
                )
            })?;
        }

        self.manifest_path = new_manifest_path;
        self.project_dir = new_dir;
        self.active_project = Some(self.manifest_path.clone());
        self.reload_config()?;
        self.refresh_workspace_discovery()?;
        self.status = Some(format!("renamed project from `{old_name}` to `{new_name}`"));
        Ok(())
    }

    fn submit_create(&mut self) -> Result<()> {
        let project_name = self.create_input.value().trim().to_string();
        if project_name.is_empty() {
            self.status = Some("project name cannot be empty".to_string());
            self.welcome_focus = WelcomeFocus::CreateInput;
            self.welcome_input_editing = true;
            return Ok(());
        }

        if self.install_in_progress() {
            self.status = Some(
                "a background task is in progress; wait before switching projects".to_string(),
            );
            return Ok(());
        }

        let manifest_path = create_project_in_dir(&self.workspace_root, &project_name)?;
        self.create_input = Input::default();
        self.welcome_input_editing = false;
        self.refresh_workspace_discovery()?;
        self.set_active_project(manifest_path)?;
        self.status = Some(format!("created and opened project `{project_name}`"));
        Ok(())
    }

}
