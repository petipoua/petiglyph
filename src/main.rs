#![allow(clippy::collapsible_else_if, clippy::uninlined_format_args)]

mod animation_media;
mod artifact_warning;
mod build;
mod cli;
mod compose;
mod doctor;
mod glyph_debug;
mod image_pipeline;
mod install;
mod project;
mod tui;

fn main() {
    cli::run();
}

#[cfg(test)]
mod tests;
