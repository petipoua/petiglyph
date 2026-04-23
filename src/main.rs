mod build;
mod cli;
mod install;
mod project;
mod tui;

fn main() {
    cli::run();
}

#[cfg(test)]
mod tests;
