#![allow(
    clippy::collapsible_if,
    clippy::collapsible_match,
    clippy::enum_variant_names,
    clippy::if_same_then_else,
    clippy::manual_div_ceil,
    clippy::match_single_binding,
    clippy::redundant_closure,
    clippy::single_match,
    clippy::too_many_arguments
)]

use anyhow::{Context, Result, anyhow, bail};
use crossterm::ExecutableCommand;
use crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyEventState, KeyModifiers, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use image::{Rgba, RgbaImage};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Tabs, Wrap,
};
use ratatui::{Frame, Terminal};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::env;
use std::fs::{self, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    OnceLock,
    mpsc::{self, Receiver, TryRecvError},
};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tui_input::{Input, backend::crossterm::EventHandler};
use walkdir::WalkDir;

use crate::animation_media;
use crate::artifact_warning::incompatible_artifact_warning;
use crate::build::{
    BuildSummary, MappingEntry, PreprocessedGlyph, build_outputs, expected_bdf_path,
    expected_ttf_path, is_supported_source,
    preprocess_sources_with_compositions_and_standard_sources,
};
use crate::glyph_debug;
use crate::image_pipeline::{
    coverage_map_from_image, load_source_rgba, preprocess_standard_source,
};
use crate::install::{
    DEFAULT_INSTALL_NAME_MODE, FontInstallNameMode, effective_font_name,
    expected_install_ttf_path_for_mode, install_built_font, install_dir_for_manifest,
    installed_ttf_candidates_for_manifest_font, supplementary_pua_usage_summary,
    uninstall_installed_font_file,
};
use crate::project::{
    AnimationDef, AnimationType, BleedLevel, CompositionDef, RuntimeConfig, create_project_in_dir,
    delete_project_for_manifest, discover_project_manifests, format_codepoint, load_runtime_config,
    read_manifest, slugify, write_manifest,
};

include!("tui_session.rs");
include!("tui_installed_fonts.rs");
include!("tui_debug.rs");
include!("tui_input_support.rs");
include!("tui_glyph_keys.rs");
include!("tui_home_workflow.rs");
include!("tui_welcome_render.rs");
include!("tui_app_core.rs");
include!("tui_app_tasks.rs");
include!("tui_glyph_model.rs");
include!("tui_events.rs");
include!("tui_render_shell.rs");
include!("tui_render_workflows.rs");
include!("tui_glyph_render.rs");
include!("tui_imports.rs");
include!("tui_render_helpers.rs");

#[cfg(test)]
mod tests {
    include!("tui_tests_core.rs");
    include!("tui_tests_workflow.rs");
    include!("tui_tests_imports.rs");
}
