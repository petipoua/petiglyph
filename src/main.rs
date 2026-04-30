mod build;
mod cli;
mod doctor;
mod install;
mod project;
mod tui;

fn main() {
    cli::run();
}

#[cfg(test)]
mod tests;
