mod artifact_warning;
mod build;
mod cli;
mod compose;
mod doctor;
mod install;
mod project;
mod tui;

fn main() {
    cli::run();
}

#[cfg(test)]
mod tests;
