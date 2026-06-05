impl App {
    fn start_project_switch_task(
        &mut self,
        manifest_path: PathBuf,
        project_name: String,
    ) -> Result<()> {
        if self.install_in_progress() || self.project_switch_task.is_some() {
            self.status = Some(
                "a background task is in progress; wait before switching projects".to_string(),
            );
            return Ok(());
        }

        let launch_overrides = self.launch_overrides.clone();
        let target_manifest_path = manifest_path.clone();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let result = load_project_switch_task(manifest_path, launch_overrides)
                .map_err(|err| err.to_string());
            let _ = sender.send(result);
        });

        self.project_switch_task = Some(ProjectSwitchTask {
            target_manifest_path,
            receiver,
            spinner_index: 0,
            spinner_last_frame_at: Instant::now(),
        });
        self.status = Some(format!("switching to project `{project_name}`..."));
        Ok(())
    }

    fn set_active_project(&mut self, manifest_path: PathBuf) -> Result<()> {
        if self.install_in_progress() || self.project_switch_task.is_some() {
            self.status = Some(
                "a background task is in progress; wait before switching projects".to_string(),
            );
            return Ok(());
        }

        let old_manifest = self.active_project.clone();
        let old_label = self.active_project_switch_label();
        let changed = old_manifest.as_ref() != Some(&manifest_path);

        self.manifest_path = manifest_path.clone();
        self.project_dir = manifest_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        self.active_project = Some(manifest_path);
        self.reload_glyphs()?;
        self.sync_selected_project();

        if changed {
            self.switch_notice = Some(ProjectSwitchNotice {
                from_label: old_label,
                to_label: self.active_project_switch_label(),
                started_at: Instant::now(),
            });
        }

        Ok(())
    }

    fn reload_config(&mut self) -> Result<()> {
        if self.active_project.is_none() {
            self.config = inactive_runtime_config(&self.workspace_root);
            self.last_build = None;
            self.last_sample = None;
            self.installed_font_path = None;
            self.debug_log_path = None;
            self.debug_log_lines.clear();
            return Ok(());
        }

        self.config = load_runtime_config(
            &self.manifest_path,
            self.launch_overrides.input_dir.clone(),
            None,
            self.launch_overrides.threshold,
            self.launch_overrides.glyph_size,
            self.launch_overrides.codepoint_start.clone(),
        )?;
        let (last_build, last_sample) = cached_build_state(&self.config);
        self.last_build = last_build;
        self.last_sample = last_sample;
        self.installed_font_path = cached_installed_font_path(
            &self.manifest_path,
            &self.config.font_name,
            &self.config.project_id,
        );
        self.debug_log_path = Some(glyph_debug::session_log_path(&self.config.project_dir));
        Ok(())
    }

    fn reload_glyphs(&mut self) -> Result<()> {
        if self.active_project.is_none() {
            self.glyphs.clear();
            self.selected = 0;
            self.selected_visible = 0;
            self.expanded_compositions.clear();
            self.expanded_animations.clear();
            self.live_glyph_source_count = None;
            self.live_glyph_source_probe_fingerprint = None;
            self.live_glyph_source_probe_at = Some(Instant::now());
            self.status = Some("create a project in Home or relaunch with --manifest".to_string());
            return Ok(());
        }

        self.reload_config()?;
        if self.debug_enabled {
            glyph_debug::begin_session(&self.config.project_dir, "tui.reload_glyphs");
        }

        if !self.config.input_dir.exists() {
            self.glyphs.clear();
            self.selected = 0;
            self.selected_visible = 0;
            self.expanded_compositions.clear();
            self.expanded_animations.clear();
            self.live_glyph_source_count = Some(0);
            self.live_glyph_source_probe_fingerprint = Some(0);
            self.live_glyph_source_probe_at = Some(Instant::now());
            self.status = Some(format!(
                "icons directory not found yet: {}",
                self.config.input_dir.display()
            ));
            return Ok(());
        }

        let mut sources = Vec::new();
        for entry in WalkDir::new(&self.config.input_dir).follow_links(true) {
            let entry = entry.with_context(|| {
                format!("failed while scanning {}", self.config.input_dir.display())
            })?;
            if entry.file_type().is_file() && is_supported_source(entry.path()) {
                sources.push(entry.path().to_path_buf());
            }
        }
        sources.sort();

        if sources.is_empty() {
            self.glyphs.clear();
            self.selected = 0;
            self.selected_visible = 0;
            self.expanded_compositions.clear();
            self.expanded_animations.clear();
            self.live_glyph_source_count = Some(0);
            self.live_glyph_source_probe_fingerprint = Some(0);
            self.live_glyph_source_probe_at = Some(Instant::now());
            self.status = Some(format!(
                "add or drag image files into {}",
                self.config.input_dir.display()
            ));
            return Ok(());
        }

        let glyphs = preprocess_sources_for_config(&sources, &self.config)?
        .into_iter()
        .map(|glyph| {
            let saved_threshold = self
                .config
                .threshold_overrides
                .get(&glyph.source_parent_key)
                .copied();
            let working_threshold = saved_threshold.unwrap_or(self.config.base_threshold);
            let saved_invert = self
                .config
                .invert_overrides
                .get(&glyph.source_parent_key)
                .copied()
                .unwrap_or(false);
            InteractiveGlyph {
                glyph,
                saved_threshold,
                working_threshold,
                saved_invert,
                working_invert: saved_invert,
            }
        })
        .collect::<Vec<_>>();

        self.glyphs = glyphs;
        let active_compositions = self
            .glyphs
            .iter()
            .filter_map(|g| {
                g.glyph
                    .composition_tile
                    .as_ref()
                    .map(|_| g.glyph.source_parent_key.clone())
            })
            .collect::<BTreeSet<_>>();
        self.expanded_compositions
            .retain(|source| active_compositions.contains(source));
        let active_animations = self
            .config
            .animations
            .iter()
            .map(|animation| animation.name.clone())
            .collect::<BTreeSet<_>>();
        self.expanded_animations
            .retain(|name| active_animations.contains(name));
        self.clamp_glyph_selection();
        self.live_glyph_source_count = Some(self.glyphs.len());
        self.live_glyph_source_probe_fingerprint =
            Some(glyph_source_fingerprint(&self.config.input_dir)?);
        self.live_glyph_source_probe_at = Some(Instant::now());
        let mut status = format!(
            "loaded {} glyph{} from {}",
            self.glyphs.len(),
            if self.glyphs.len() == 1 { "" } else { "s" },
            self.config.input_dir.display()
        );
        if self.debug_enabled {
            status.push_str(&format!(
                " | debug: {}",
                self.config.project_dir.join("debug").display()
            ));
        }
        self.status = Some(status);
        Ok(())
    }

    fn refresh_pipeline_debug_log(&mut self) {
        if !self.debug_enabled {
            self.debug_log_lines.clear();
            return;
        }
        let Some(path) = &self.debug_log_path else {
            self.debug_log_lines.clear();
            return;
        };
        self.debug_log_lines = glyph_debug::read_recent_log_lines(path, DEBUG_LOG_VISIBLE_LINES);
    }

    fn refresh_live_glyph_source_count(&mut self) {
        if self.active_project.is_none() {
            self.live_glyph_source_count = None;
            self.live_glyph_source_probe_fingerprint = None;
            self.live_glyph_source_probe_at = Some(Instant::now());
            return;
        }

        let now = Instant::now();
        if self.live_glyph_source_probe_at.is_some_and(|at| {
            now.duration_since(at) < Duration::from_millis(GLYPH_SOURCE_COUNT_REFRESH_MS)
        }) {
            return;
        }
        self.live_glyph_source_probe_at = Some(now);

        let Ok(next_fingerprint) = glyph_source_fingerprint(&self.config.input_dir) else {
            return;
        };

        if self.live_glyph_source_probe_fingerprint == Some(next_fingerprint) {
            return;
        }

        self.live_glyph_source_probe_fingerprint = Some(next_fingerprint);
        self.live_glyph_source_count =
            Some(count_supported_sources(&self.config.input_dir).unwrap_or(self.glyphs.len()));
    }

    fn import_dropped_images(&mut self, payload: &str) -> Result<()> {
        if self.install_in_progress()
            || self.animation_import_task.is_some()
            || self.home_import_task.is_some()
        {
            self.status =
                Some("a background task is in progress; wait before importing images".to_string());
            return Ok(());
        }

        if self.active_project.is_none() {
            self.status =
                Some("create or select a project before importing dropped images".to_string());
            return Ok(());
        }

        self.reload_config()?;
        if matches!(
            self.home_workflow,
            HomeWorkflow::Import(HomeCreationKind::AnimatedGlyph)
                | HomeWorkflow::Import(HomeCreationKind::AnimatedGridGlyph)
                | HomeWorkflow::Tweaking(HomeCreationKind::AnimatedGlyph)
                | HomeWorkflow::Tweaking(HomeCreationKind::AnimatedGridGlyph)
        ) || matches!(self.glyph_tool_mode, GlyphToolMode::ImportAnimationFrames)
        {
            self.start_animation_frame_import(payload.to_string())?;
            return Ok(());
        }

        if matches!(
            self.home_workflow,
            HomeWorkflow::Import(HomeCreationKind::Glyph)
                | HomeWorkflow::Tweaking(HomeCreationKind::Glyph)
                | HomeWorkflow::Import(HomeCreationKind::Grid)
                | HomeWorkflow::Tweaking(HomeCreationKind::Grid)
        ) {
            if matches!(
                self.home_workflow,
                HomeWorkflow::Import(HomeCreationKind::Grid)
                    | HomeWorkflow::Tweaking(HomeCreationKind::Grid)
            ) && collect_dropped_paths(payload).len() != 1
            {
                self.home_workflow_error =
                    Some("drop only ONE IMAGE for the grid (selection unchanged)".to_string());
                self.status = Some(
                    "create grid: drop only one image at a time (kept current selection)"
                        .to_string(),
                );
                return Ok(());
            }
            self.start_home_import_task(payload.to_string())?;
            return Ok(());
        }

        let processing = if matches!(
            self.home_workflow,
            HomeWorkflow::Import(_) | HomeWorkflow::Tweaking(_)
        ) {
            animation_import_processing_options(&self.animation_import_settings)
        } else {
            animation_media::AnimationImportProcessingOptions {
                grayscale_enabled: false,
                ..Default::default()
            }
        };
        let import = import_image_files_to_input(
            &self.config.input_dir,
            payload,
            ExistingImportPolicy::Rename,
            processing,
        )?;
        self.finish_static_home_import(import)
    }

    fn start_home_import_task(&mut self, payload: String) -> Result<()> {
        if self.home_import_task.is_some() {
            self.status = Some("image import is already processing".to_string());
            return Ok(());
        }

        let config = self.config.clone();
        let processing = animation_import_processing_options(&self.animation_import_settings);
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let result = import_image_files_to_input(
                &config.input_dir,
                &payload,
                ExistingImportPolicy::Rename,
                processing,
            )
            .map_err(|err| err.to_string());
            let _ = sender.send(result);
        });

        self.home_import_task = Some(HomeImportTask {
            receiver,
            spinner_index: 0,
            spinner_last_frame_at: Instant::now(),
        });
        Ok(())
    }

    fn finish_static_home_import(&mut self, import: DropImportResult) -> Result<()> {
        let in_creation_workflow = matches!(
            self.home_workflow,
            HomeWorkflow::Import(_) | HomeWorkflow::Tweaking(_)
        );
        if matches!(
            self.home_workflow,
            HomeWorkflow::Import(HomeCreationKind::Glyph)
                | HomeWorkflow::Tweaking(HomeCreationKind::Glyph)
        ) {
            self.home_workflow_recent_imported_source_keys
                .extend(import.imported_source_keys.clone());
            self.rebuild_home_tweak_queue_for_glyph();
        } else {
            self.home_workflow_recent_imported_source_keys = import.imported_source_keys.clone();
        }
        self.home_workflow_created_source_keys
            .extend(import.created_source_keys.clone());

        if matches!(
            self.home_workflow,
            HomeWorkflow::Import(HomeCreationKind::Grid)
                | HomeWorkflow::Tweaking(HomeCreationKind::Grid)
        ) {
            if import.imported_source_keys.len() != 1 {
                self.home_workflow_grid_inline_notice = None;
                self.home_workflow_error =
                    Some("drop only ONE IMAGE for the grid (selection unchanged)".to_string());
                self.status = Some(
                    "create grid: drop only one image at a time (kept current selection)"
                        .to_string(),
                );
                return Ok(());
            }
            let next_source_key = import.imported_source_keys.first().cloned();
            let previous_source_key = self.home_workflow_grid_source_key.clone();
            self.home_workflow_grid_source_key = next_source_key.clone();
            self.home_workflow_import_count = usize::from(next_source_key.is_some());
            self.home_workflow_error = None;
            if let (Some(previous), Some(next)) = (previous_source_key, next_source_key) {
                if previous != next {
                    self.home_workflow_grid_inline_notice =
                        Some(format!("Replaced image: {previous} -> {next}"));
                }
            } else {
                self.home_workflow_grid_inline_notice =
                    Some("Drop another image to replace this selection".to_string());
            }
        }

        if import.imported > 0 {
            if !matches!(
                self.home_workflow,
                HomeWorkflow::Import(HomeCreationKind::Grid)
                    | HomeWorkflow::Tweaking(HomeCreationKind::Grid)
            ) {
                self.home_workflow_import_count = self
                    .home_workflow_import_count
                    .saturating_add(import.imported);
            }
            if !in_creation_workflow {
                self.reload_glyphs()?;
            }
            if self.view == AppView::Welcome && matches!(self.home_workflow, HomeWorkflow::Launcher)
            {
                self.welcome_input_editing = false;
                self.view = AppView::Glyphs;
            }
        }

        self.status = Some(format_drop_import_status(
            import.imported,
            import.renamed,
            import.skipped_existing,
            import.skipped_unsupported,
            import.skipped_missing,
        ));
        Ok(())
    }

    fn start_animation_frame_import(&mut self, payload: String) -> Result<()> {
        if self.animation_import_task.is_some() {
            self.status = Some("animation frames are already loading".to_string());
            return Ok(());
        }

        let config = self.config.clone();
        let processing = animation_import_processing_options(&self.animation_import_settings);
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let result = (|| -> Result<AnimationImportTaskOutput> {
                let media_import = animation_media::import_animation_media_to_input(
                    &config.input_dir,
                    &payload,
                    animation_media::ExistingImportPolicy::ReuseIdentical,
                    processing,
                )?;
                let import = DropImportResult {
                    imported: media_import.imported,
                    renamed: media_import.renamed,
                    skipped_existing: media_import.skipped_existing,
                    skipped_unsupported: media_import.skipped_unsupported,
                    skipped_missing: media_import.skipped_missing,
                    imported_source_keys: media_import.imported_source_keys,
                    created_source_keys: media_import.created_source_keys,
                };
                let loaded = if !import.imported_source_keys.is_empty() {
                    Some(load_interactive_glyphs_from_config(&config)?)
                } else {
                    None
                };
                let detail_status = Some(format_animation_media_import_status(
                    import.imported,
                    import.renamed,
                    import.skipped_existing,
                    import.skipped_unsupported,
                    import.skipped_missing,
                    media_import.media_files_processed,
                    media_import.frames_extracted,
                ));
                Ok(AnimationImportTaskOutput {
                    import,
                    loaded,
                    detail_status,
                })
            })()
            .map_err(|err| err.to_string());
            let _ = sender.send(result);
        });

        self.animation_import_task = Some(AnimationImportTask {
            receiver,
            spinner_index: 0,
            spinner_last_frame_at: Instant::now(),
        });
        self.status = Some("loading animation frames...".to_string());
        Ok(())
    }

    fn export_animation_import_test_image(&mut self) -> Result<()> {
        if self.animation_import_task.is_some() {
            self.status = Some("wait for frame import to finish before exporting".to_string());
            return Ok(());
        }
        let source_keys = self.test_image_export_source_keys();
        if source_keys.is_empty() {
            self.status = Some("import at least one source image first".to_string());
            return Ok(());
        }

        let test_images_dir = self.config.project_dir.join("test-images");
        fs::create_dir_all(&test_images_dir)
            .with_context(|| format!("failed to create {}", test_images_dir.display()))?;
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_millis();
        let grayscale_enabled = self.animation_import_settings.grayscale_enabled;
        let grayscale_options = self.animation_import_settings.grayscale_options;
        let mut first_out_path: Option<PathBuf> = None;
        let mut exported = 0usize;

        for (index, source_key) in source_keys.iter().enumerate() {
            let Some((image, threshold, invert, is_composed)) =
                self.render_test_image_for_source(source_key)?
            else {
                continue;
            };
            let source_slug = slugify(source_key).trim_matches('_').to_string();
            let source_label = if source_slug.is_empty() {
                "frame".to_string()
            } else {
                source_slug
            };
            let filename = format!(
                "import_test_{}_{}_gray_{}_b{}_c{}_g{}_th{:03}_inv{}_{}_f{:03}.png",
                if is_composed { "composition" } else { "source" },
                source_label,
                if grayscale_enabled { "on" } else { "off" },
                signed_filename_value(grayscale_options.brightness),
                signed_filename_value(grayscale_options.contrast),
                grayscale_options.gamma_percent,
                threshold,
                if invert { 1 } else { 0 },
                now_ms,
                index + 1
            );
            let out_path = test_images_dir.join(filename);
            image
                .save(&out_path)
                .with_context(|| format!("failed to save {}", out_path.display()))?;
            if first_out_path.is_none() {
                first_out_path = Some(out_path.clone());
            }
            exported += 1;
        }

        if exported == 0 {
            self.status = Some("no matching glyph coverage found for exported sources".to_string());
            return Ok(());
        }

        if let Some(first_out_path) = first_out_path {
            self.animation_import_settings.last_exported_test_image = Some(first_out_path.clone());
            self.status = Some(format!(
                "exported {} test image(s) to {}",
                exported,
                test_images_dir.display()
            ));
        }
        Ok(())
    }

    fn test_image_export_source_keys(&self) -> Vec<String> {
        if matches!(
            self.home_workflow,
            HomeWorkflow::Import(HomeCreationKind::AnimatedGlyph)
                | HomeWorkflow::Import(HomeCreationKind::AnimatedGridGlyph)
                | HomeWorkflow::Tweaking(HomeCreationKind::AnimatedGlyph)
                | HomeWorkflow::Tweaking(HomeCreationKind::AnimatedGridGlyph)
        ) {
            let limit = usize::from(self.animation_import_settings.export_frame_count);
            return self
                .animation_selection_order
                .iter()
                .take(limit)
                .cloned()
                .collect();
        }
        if matches!(
            self.home_workflow,
            HomeWorkflow::Import(HomeCreationKind::Grid)
                | HomeWorkflow::Tweaking(HomeCreationKind::Grid)
        ) {
            return self
                .home_workflow_grid_source_key
                .iter()
                .cloned()
                .collect::<Vec<_>>();
        }
        self.home_workflow_recent_imported_source_keys
            .last()
            .cloned()
            .into_iter()
            .collect()
    }

    fn render_test_image_for_source(
        &self,
        source_key: &str,
    ) -> Result<Option<(RgbaImage, u8, bool, bool)>> {
        if let Some(def) = self.config.compositions.get(source_key) {
            let rows = def.rows;
            let cols = emitted_composition_cols(def.cols);
            let tiles = self
                .glyphs
                .iter()
                .filter(|glyph| {
                    glyph.glyph.source_parent_key == source_key
                        && glyph.glyph.composition_tile.is_some()
                })
                .collect::<Vec<_>>();
            if let Some(image) = render_test_image_from_composition_tiles(rows, cols, &tiles)? {
                let threshold = self.animation_import_settings.threshold;
                let invert = tiles
                    .first()
                    .map(|glyph| glyph.working_invert)
                    .unwrap_or(false);
                return Ok(Some((image, threshold, invert, true)));
            }
        }

        let source_path = self.config.input_dir.join(source_key);
        if let Some(coverage) = self.live_import_source_coverage(&source_path) {
            let threshold = self.animation_import_settings.threshold;
            let invert = self
                .config
                .invert_overrides
                .get(source_key)
                .copied()
                .unwrap_or(false);
            let image = render_test_image_from_coverage(
                &coverage,
                self.config.glyph_size,
                self.config.glyph_size,
                threshold,
                invert,
                source_key,
            )?;
            return Ok(Some((image, threshold, invert, false)));
        }

        let Some(active) = self
            .glyphs
            .iter()
            .find(|glyph| {
                glyph.glyph.source_parent_key == source_key
                    && glyph.glyph.composition_tile.is_none()
            })
            .or_else(|| {
                self.glyphs
                    .iter()
                    .find(|glyph| glyph.glyph.source_parent_key == source_key)
            })
        else {
            let source_path = self.config.input_dir.join(source_key);
            if !source_path.is_file() || !is_supported_source(&source_path) {
                return Ok(None);
            }
            let threshold = self.animation_import_settings.threshold;
            let invert = self
                .config
                .invert_overrides
                .get(source_key)
                .copied()
                .unwrap_or(false);
            let coverage = preprocess_standard_source(
                &source_path,
                self.config.glyph_size,
                self.config.glyph_size,
                source_key,
            )?;
            let image = render_test_image_from_coverage(
                &coverage,
                self.config.glyph_size,
                self.config.glyph_size,
                threshold,
                invert,
                source_key,
            )?;
            return Ok(Some((image, threshold, invert, false)));
        };

        let image = render_test_image_from_single_glyph(active)?;
        Ok(Some((
            image,
            self.animation_import_settings.threshold,
            active.working_invert,
            active.glyph.composition_tile.is_some(),
        )))
    }

    fn start_install_font(&mut self) {
        if self.active_project.is_none() {
            self.status = Some(
                "create a project in Home or relaunch with --manifest before installing"
                    .to_string(),
            );
            return;
        }

        if self.install_task.is_some() {
            self.status = Some("font operation already in progress".to_string());
            return;
        }

        let manifest_path = self.manifest_path.clone();
        let launch_overrides = self.launch_overrides.clone();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let result =
                build_and_install(manifest_path, launch_overrides).map_err(|err| err.to_string());
            let _ = sender.send(result);
        });

        self.install_task = Some(InstallTask {
            kind: FontTaskKind::Install,
            receiver,
            spinner_index: 0,
            spinner_last_frame_at: Instant::now(),
        });
        self.status = None;
    }

    fn start_uninstall_selected_installed_font(&mut self) -> Result<()> {
        if self.install_in_progress() {
            self.status =
                Some("font operation is in progress; wait before uninstalling".to_string());
            return Ok(());
        }

        let Some(font) = self
            .installed_fonts
            .get(self.selected_installed_font)
            .cloned()
        else {
            self.status = Some("no installed font selected".to_string());
            return Ok(());
        };

        let target_path = font.path.clone();
        let file_name = font.file_name.clone();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let result = uninstall_installed_font_task(target_path.clone(), file_name)
                .map_err(|err| err.to_string());
            let _ = sender.send(result);
        });

        self.install_task = Some(InstallTask {
            kind: FontTaskKind::UninstallInstalled { path: font.path },
            receiver,
            spinner_index: 0,
            spinner_last_frame_at: Instant::now(),
        });
        self.status = None;
        Ok(())
    }

    fn poll_font_task(&mut self) {
        let mut task_result = None;
        let mut disconnected = false;

        if let Some(task) = self.install_task.as_mut() {
            let frame_duration = task.kind.spinner_frame_duration();
            let now = Instant::now();
            while now.duration_since(task.spinner_last_frame_at) >= frame_duration {
                task.spinner_index = (task.spinner_index + 1) % task.kind.spinner_frames().len();
                task.spinner_last_frame_at += frame_duration;
            }
            match task.receiver.try_recv() {
                Ok(result) => task_result = Some(result),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => disconnected = true,
            }
        }

        if disconnected {
            let operation = self
                .install_task
                .as_ref()
                .map(|task| {
                    if task.kind.is_uninstall() {
                        "uninstall"
                    } else {
                        "install"
                    }
                })
                .unwrap_or("font");
            self.install_task = None;
            self.status = Some(format!("{operation} task terminated unexpectedly"));
            return;
        }

        let Some(result) = task_result else {
            return;
        };

        self.install_task = None;
        match result {
            Ok(InstallTaskOutput::Install {
                summary,
                sample,
                installed_path,
                first_install_on_machine,
            }) => {
                self.last_build = Some(*summary);
                self.last_sample = sample;
                self.installed_font_path = Some(installed_path.clone());
                if let Err(err) = self.refresh_workspace_discovery() {
                    self.status = Some(format!(
                        "installed font to {}; refresh failed: {err}",
                        installed_path.display()
                    ));
                } else {
                    self.refresh_pua_usage_summary();
                    if let Some(idx) = self
                        .installed_fonts
                        .iter()
                        .position(|font| font.path == installed_path)
                    {
                        self.selected_installed_font = idx;
                    }
                    self.status = Some(format!("installed font to {}", installed_path.display()));
                }
                if first_install_on_machine {
                    self.first_install_notice_open = true;
                }
            }
            Ok(InstallTaskOutput::Uninstall { status_message }) => {
                if let Err(err) = self.refresh_workspace_discovery() {
                    self.status = Some(format!("{status_message}; refresh failed: {err}"));
                } else if self.active_project.is_some() {
                    self.refresh_pua_usage_summary();
                    if let Err(err) = self.reload_config() {
                        self.status = Some(format!("{status_message}; reload failed: {err}"));
                    } else {
                        self.status = Some(status_message);
                    }
                } else {
                    self.refresh_pua_usage_summary();
                    self.status = Some(status_message);
                }
            }
            Err(err) => {
                self.status = Some(format_status_from_error(&self.manifest_path, &err));
                let _ = self.reload_config();
            }
        }
    }

    fn poll_project_switch_task(&mut self) {
        let mut task_result = None;
        let mut disconnected = false;

        if let Some(task) = self.project_switch_task.as_mut() {
            let frame_duration = Duration::from_millis(FONT_TASK_SPINNER_FRAME_MS);
            let now = Instant::now();
            while now.duration_since(task.spinner_last_frame_at) >= frame_duration {
                task.spinner_index = (task.spinner_index + 1) % INSTALL_SPINNER_FRAMES.len();
                task.spinner_last_frame_at += frame_duration;
            }

            match task.receiver.try_recv() {
                Ok(result) => task_result = Some(result),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => disconnected = true,
            }
        }

        if disconnected {
            self.project_switch_task = None;
            self.status = Some("project switch task terminated unexpectedly".to_string());
            return;
        }

        let Some(result) = task_result else {
            return;
        };

        self.project_switch_task = None;
        match result {
            Ok(output) => {
                let old_label = self.active_project_switch_label();
                let changed = self.active_project.as_ref() != Some(&output.manifest_path);

                self.manifest_path = output.manifest_path.clone();
                self.project_dir = output
                    .manifest_path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .to_path_buf();
                self.active_project = Some(output.manifest_path.clone());
                self.config = output.config;
                self.glyphs = output.loaded.glyphs;
                self.live_glyph_source_count = Some(self.glyphs.len());
                self.live_glyph_source_probe_fingerprint = Some(output.loaded.source_fingerprint);
                self.live_glyph_source_probe_at = Some(Instant::now());
                self.last_build = output.last_build;
                self.last_sample = output.last_sample;
                self.installed_font_path = output.installed_font_path;
                self.debug_log_path = Some(glyph_debug::session_log_path(&self.config.project_dir));

                self.clamp_glyph_selection();
                self.sync_selected_project();
                self.status = Some(format!("opened project `{}`", self.config.font_name));

                if changed {
                    self.switch_notice = Some(ProjectSwitchNotice {
                        from_label: old_label,
                        to_label: self.active_project_switch_label(),
                        started_at: Instant::now(),
                    });
                }
            }
            Err(err) => {
                self.status = Some(format_status_from_error(&self.manifest_path, &err));
            }
        }
    }

    fn poll_animation_import_task(&mut self) {
        let mut task_result = None;
        let mut disconnected = false;

        if let Some(task) = self.animation_import_task.as_mut() {
            let frame_duration = Duration::from_millis(FONT_TASK_SPINNER_FRAME_MS);
            let now = Instant::now();
            while now.duration_since(task.spinner_last_frame_at) >= frame_duration {
                task.spinner_index =
                    (task.spinner_index + 1) % ANIMATION_IMPORT_SPINNER_FRAMES.len();
                task.spinner_last_frame_at += frame_duration;
            }
            match task.receiver.try_recv() {
                Ok(result) => task_result = Some(result),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => disconnected = true,
            }
        }

        if disconnected {
            self.animation_import_task = None;
            self.status = Some("animation frame import task terminated unexpectedly".to_string());
            return;
        }

        let Some(result) = task_result else {
            return;
        };

        self.animation_import_task = None;
        match result {
            Ok(output) => self.finish_animation_import(output),
            Err(err) => self.status = Some(format!("animation frame import failed: {err}")),
        }
    }

    fn poll_home_import_task(&mut self) {
        let mut task_result = None;
        let mut disconnected = false;

        if let Some(task) = self.home_import_task.as_mut() {
            let frame_duration = Duration::from_millis(FONT_TASK_SPINNER_FRAME_MS);
            let now = Instant::now();
            while now.duration_since(task.spinner_last_frame_at) >= frame_duration {
                task.spinner_index =
                    (task.spinner_index + 1) % ANIMATION_IMPORT_SPINNER_FRAMES.len();
                task.spinner_last_frame_at += frame_duration;
            }
            match task.receiver.try_recv() {
                Ok(result) => task_result = Some(result),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => disconnected = true,
            }
        }

        if disconnected {
            self.home_import_task = None;
            self.status = Some("image import task terminated unexpectedly".to_string());
            return;
        }

        let Some(result) = task_result else {
            return;
        };

        self.home_import_task = None;
        match result {
            Ok(import) => {
                if let Err(err) = self.finish_static_home_import(import) {
                    self.status = Some(format_status_from_error(
                        &self.manifest_path,
                        &err.to_string(),
                    ));
                }
            }
            Err(err) => self.status = Some(format!("image import failed: {err}")),
        }
    }

    fn finish_animation_import(&mut self, mut output: AnimationImportTaskOutput) {
        if self.discard_next_animation_import_result {
            self.discard_next_animation_import_result = false;
            for source_key in output.import.created_source_keys.drain(..) {
                if source_key.is_empty() {
                    continue;
                }
                let path = self.config.input_dir.join(source_key);
                let _ = fs::remove_file(path);
            }
            let _ = self.reload_glyphs();
            return;
        }

        if let Some(loaded) = output.loaded {
            self.glyphs = loaded.glyphs;
            self.clamp_glyph_selection();
            self.live_glyph_source_count = Some(self.glyphs.len());
            self.live_glyph_source_probe_fingerprint = Some(loaded.source_fingerprint);
            self.live_glyph_source_probe_at = Some(Instant::now());
            if self.view == AppView::Welcome && matches!(self.home_workflow, HomeWorkflow::Launcher)
            {
                self.welcome_input_editing = false;
                self.view = AppView::Glyphs;
            }
        }

        let has_selected_sources = !output.import.imported_source_keys.is_empty();
        if has_selected_sources {
            self.home_workflow_import_count = self
                .home_workflow_import_count
                .saturating_add(output.import.imported_source_keys.len());
        } else if output.import.imported > 0 {
            self.home_workflow_import_count = self
                .home_workflow_import_count
                .saturating_add(output.import.imported);
        }
        for source_key in output.import.imported_source_keys {
            self.animation_imported_set.insert(source_key.clone());
            if self.animation_selection_set.insert(source_key.clone()) {
                self.animation_selection_order.push(source_key);
            }
        }
        self.home_workflow_created_source_keys
            .extend(output.import.created_source_keys);

        if has_selected_sources {
            self.status = Some(format!(
                "animation draft import: {} frame{} selected",
                self.animation_selection_order.len(),
                if self.animation_selection_order.len() == 1 {
                    ""
                } else {
                    "s"
                }
            ));
        } else {
            self.status = Some(output.detail_status.unwrap_or_else(|| {
                format_drop_import_status(
                    output.import.imported,
                    output.import.renamed,
                    output.import.skipped_existing,
                    output.import.skipped_unsupported,
                    output.import.skipped_missing,
                )
            }));
        }
    }

    fn font_task_kind(&self) -> Option<&FontTaskKind> {
        self.install_task.as_ref().map(|task| &task.kind)
    }

    fn font_task_spinner_frame(&self) -> Option<&'static str> {
        self.install_task.as_ref().map(|task| {
            let frames = task.kind.spinner_frames();
            frames[task.spinner_index % frames.len()]
        })
    }

    fn animation_import_spinner_frame(&self) -> Option<&'static str> {
        self.animation_import_task.as_ref().map(|task| {
            ANIMATION_IMPORT_SPINNER_FRAMES
                [task.spinner_index % ANIMATION_IMPORT_SPINNER_FRAMES.len()]
        })
    }

    fn home_import_spinner_frame(&self) -> Option<&'static str> {
        self.home_import_task.as_ref().map(|task| {
            ANIMATION_IMPORT_SPINNER_FRAMES
                [task.spinner_index % ANIMATION_IMPORT_SPINNER_FRAMES.len()]
        })
    }

    fn project_switch_spinner_frame(&self) -> Option<&'static str> {
        self.project_switch_task
            .as_ref()
            .map(|task| INSTALL_SPINNER_FRAMES[task.spinner_index % INSTALL_SPINNER_FRAMES.len()])
    }

    fn project_switch_target_manifest_path(&self) -> Option<&Path> {
        self.project_switch_task
            .as_ref()
            .map(|task| task.target_manifest_path.as_path())
    }

    fn font_task_button_style(&self) -> Option<Style> {
        self.font_task_kind().map(FontTaskKind::progress_style)
    }

    fn is_selected_font_uninstall_in_progress(&self, font_path: &Path) -> bool {
        matches!(
            self.font_task_kind(),
            Some(FontTaskKind::UninstallInstalled { path }) if path == font_path
        )
    }

    fn install_in_progress(&self) -> bool {
        self.install_task.is_some()
    }

    #[cfg(test)]
    pub(crate) fn background_task_in_progress(&self) -> bool {
        self.install_in_progress()
            || self.project_switch_task.is_some()
            || self.animation_import_task.is_some()
            || self.home_import_task.is_some()
    }

    #[cfg(test)]
    pub(crate) fn poll_background_tasks_for_test(&mut self) {
        self.poll_font_task();
        self.poll_project_switch_task();
        self.poll_animation_import_task();
        self.poll_home_import_task();
    }

    fn active_project_label(&self) -> String {
        let Some(active_project) = &self.active_project else {
            return "none".to_string();
        };

        if self.verbose_paths {
            let folder = active_project
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .display();
            format!("{} ({folder})", self.config.font_name)
        } else {
            self.config.font_name.clone()
        }
    }

    fn active_project_switch_label(&self) -> String {
        let Some(active_project) = &self.active_project else {
            return "none".to_string();
        };

        active_project
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| self.config.font_name.clone())
    }

    fn clear_expired_switch_notice(&mut self) {
        if self
            .switch_notice
            .as_ref()
            .is_some_and(|notice| !switch_notice_visible(notice.started_at, Instant::now()))
        {
            self.switch_notice = None;
        }
    }
}
