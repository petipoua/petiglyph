#![allow(clippy::collapsible_else_if, clippy::uninlined_format_args)]

mod animation_media;
mod artifact_warning;
mod build;
#[allow(dead_code)]
mod cli;
mod compose;
mod doctor;
mod glyph_debug;
mod image_pipeline;
#[allow(dead_code)]
mod install;
#[allow(dead_code)]
mod project;
mod tui;

fn main() {
    cli::run();
}

#[cfg(test)]
mod tests;
