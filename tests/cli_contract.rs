use image::{Rgba, RgbaImage};
use serde_json::Value;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_dir(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("valid clock")
        .as_nanos();
    let path = env::temp_dir().join(format!("petiglyph-cli-{name}-{nonce}"));
    fs::create_dir_all(&path).expect("temp directory");
    path
}

fn run(cwd: &Path, args: &[&str]) -> Output {
    let home = cwd.join(".home");
    fs::create_dir_all(&home).expect("home directory");
    Command::new(env!("CARGO_BIN_EXE_petiglyph"))
        .current_dir(cwd)
        .args(args)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("XDG_DATA_HOME", home.join(".local/share"))
        .output()
        .expect("petiglyph runs")
}

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("valid JSON output")
}

fn png(path: &Path) {
    let mut image = RgbaImage::from_pixel(8, 8, Rgba([255, 255, 255, 0]));
    image.put_pixel(2, 2, Rgba([0, 0, 0, 255]));
    image.save(path).expect("PNG written");
}

fn new_project(workspace: &Path, name: &str) -> PathBuf {
    let output = run(workspace, &["new-project", name]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    workspace.join(name)
}

fn assert_envelope(value: &Value, command: &str, ok: bool) {
    assert_eq!(value["ok"], ok);
    assert_eq!(value["command"], command);
    assert_eq!(value["version"], env!("CARGO_PKG_VERSION"));
    assert!(value.get("data").is_some());
    assert!(value.get("error").is_some());
}

#[test]
fn help_exposes_only_new_command_hierarchy() {
    let workspace = temp_dir("help");
    let output = run(&workspace, &["--help"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    for command in [
        "new-project",
        "use-project",
        "list",
        "delete-project",
        "uninstall-font",
        "uninstall-all-fonts",
        "doctor",
        "tui",
    ] {
        assert!(stdout.contains(command), "missing {command}: {stdout}");
    }
    for removed in ["set-threshold", "clear-threshold", "composition", "sample"] {
        assert!(
            !stdout
                .lines()
                .any(|line| line.trim_start().starts_with(removed))
        );
    }

    let create = run(&workspace, &["use-project", "demo", "create", "--help"]);
    assert!(create.status.success());
    let stdout = String::from_utf8_lossy(&create.stdout);
    for kind in [
        "glyph",
        "grid-glyph",
        "animated-glyph",
        "animated-grid-glyph",
    ] {
        assert!(stdout.contains(kind), "missing {kind}: {stdout}");
    }
    fs::remove_dir_all(workspace).expect("cleanup");
}

#[test]
fn new_project_and_depth_two_listing_json() {
    let workspace = temp_dir("list");
    let nested = workspace.join("group");
    fs::create_dir_all(&nested).expect("nested directory");
    let project = new_project(&nested, "demo");

    let output = run(&workspace, &["list", "projects", "--json"]);
    assert!(output.status.success());
    let payload = json(&output);
    assert_envelope(&payload, "list.projects", true);
    assert_eq!(payload["data"]["projects"][0]["directory_name"], "demo");
    assert_eq!(
        payload["data"]["projects"][0]["relative_path"],
        "group/demo"
    );
    assert!(project.join("images").is_dir());
    assert!(project.join("build").is_dir());
    fs::remove_dir_all(workspace).expect("cleanup");
}

#[test]
fn list_projects_surfaces_malformed_manifest_warning() {
    let workspace = temp_dir("malformed");
    let project = workspace.join("broken");
    fs::create_dir_all(&project).expect("project directory");
    fs::write(project.join("petiglyph.toml"), "not = [valid").expect("manifest");

    let output = run(&workspace, &["list", "projects", "--json"]);
    assert!(output.status.success());
    let payload = json(&output);
    assert_envelope(&payload, "list.projects", true);
    assert_eq!(
        payload["data"]["warnings"][0]["code"],
        "manifest_read_failed"
    );
    fs::remove_dir_all(workspace).expect("cleanup");
}

#[test]
fn glyph_creation_accepts_variadic_input_and_persists_configuration() {
    let workspace = temp_dir("glyph");
    let project = new_project(&workspace, "demo");
    let first = workspace.join("one.png");
    let second = workspace.join("two.png");
    png(&first);
    png(&second);

    let output = run(
        &workspace,
        &[
            "use-project",
            "demo",
            "create",
            "glyph",
            "--input",
            first.to_str().unwrap(),
            second.to_str().unwrap(),
            "--threshold",
            "91",
            "--invert",
            "on",
            "--json",
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload = json(&output);
    assert_envelope(&payload, "use-project.create.glyph", true);
    assert_eq!(
        payload["data"]["imported_sources"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        payload["data"]["completed_stages"],
        serde_json::json!(["resolve", "import", "configure"])
    );
    let manifest: toml::Value =
        toml::from_str(&fs::read_to_string(project.join("petiglyph.toml")).expect("manifest"))
            .expect("valid manifest");
    assert_eq!(
        manifest["threshold_overrides"]["one.png"].as_integer(),
        Some(91)
    );
    assert_eq!(
        manifest["threshold_overrides"]["two.png"].as_integer(),
        Some(91)
    );
    fs::remove_dir_all(workspace).expect("cleanup");
}

#[test]
fn grid_creation_and_configuration_update_manifest() {
    let workspace = temp_dir("grid");
    let project = new_project(&workspace, "demo");
    let source = workspace.join("sheet.png");
    png(&source);
    let create = run(
        &workspace,
        &[
            "use-project",
            "demo",
            "create",
            "grid-glyph",
            "--input",
            source.to_str().unwrap(),
            "--rows",
            "2",
            "--cols",
            "4",
            "--json",
        ],
    );
    assert!(create.status.success());
    let configure = run(
        &workspace,
        &[
            "use-project",
            "demo",
            "configure",
            "grid-glyph",
            "sheet.png",
            "--rows",
            "4",
            "--cols",
            "2",
            "--threshold",
            "77",
            "--invert",
            "on",
            "--json",
        ],
    );
    assert!(
        configure.status.success(),
        "{}",
        String::from_utf8_lossy(&configure.stderr)
    );
    assert_envelope(&json(&configure), "use-project.configure.grid-glyph", true);
    let manifest: toml::Value =
        toml::from_str(&fs::read_to_string(project.join("petiglyph.toml")).expect("manifest"))
            .expect("valid manifest");
    assert_eq!(
        manifest["compositions"]["sheet.png"]["rows"].as_integer(),
        Some(4)
    );
    assert_eq!(
        manifest["compositions"]["sheet.png"]["cols"].as_integer(),
        Some(2)
    );
    assert_eq!(
        manifest["threshold_overrides"]["sheet.png"].as_integer(),
        Some(77)
    );
    fs::remove_dir_all(workspace).expect("cleanup");
}

#[test]
fn show_sample_requires_existing_build_artifact() {
    let workspace = temp_dir("sample");
    new_project(&workspace, "demo");
    let output = run(
        &workspace,
        &["use-project", "demo", "show-sample", "--json"],
    );
    assert!(!output.status.success());
    let payload = json(&output);
    assert_envelope(&payload, "use-project.show-sample", false);
    assert!(
        payload["error"]["message"]
            .as_str()
            .unwrap()
            .contains("run `petiglyph use-project demo build`")
    );
    fs::remove_dir_all(workspace).expect("cleanup");
}

#[test]
fn duplicate_project_basenames_are_rejected_with_candidates() {
    let workspace = temp_dir("ambiguous");
    fs::create_dir_all(workspace.join("a")).expect("a");
    fs::create_dir_all(workspace.join("b")).expect("b");
    new_project(&workspace.join("a"), "demo");
    new_project(&workspace.join("b"), "demo");
    let output = run(&workspace, &["use-project", "demo", "build", "--json"]);
    assert!(!output.status.success());
    let payload = json(&output);
    assert_envelope(&payload, "use-project.build", false);
    let message = payload["error"]["message"].as_str().unwrap();
    assert!(message.contains("ambiguous"));
    assert!(message.contains("a/demo"));
    assert!(message.contains("b/demo"));
    fs::remove_dir_all(workspace).expect("cleanup");
}

#[test]
fn delete_project_batch_preflight_prevents_partial_mutation() {
    let workspace = temp_dir("delete-batch");
    let one = new_project(&workspace, "one");
    let output = run(&workspace, &["delete-project", "one", "missing", "--json"]);
    assert!(!output.status.success());
    assert!(
        one.is_dir(),
        "valid target must remain after failed preflight"
    );
    fs::remove_dir_all(workspace).expect("cleanup");
}

#[test]
fn removed_commands_are_rejected() {
    let workspace = temp_dir("removed");
    for args in [
        vec!["create", "demo"],
        vec!["glyph", "create"],
        vec!["build"],
        vec!["sample"],
        vec!["set-threshold", "a.png", "64"],
    ] {
        let output = run(&workspace, &args);
        assert!(
            !output.status.success(),
            "removed command accepted: {args:?}"
        );
    }
    fs::remove_dir_all(workspace).expect("cleanup");
}

#[test]
fn creation_failure_reports_stage_and_completed_work() {
    let workspace = temp_dir("stage-error");
    new_project(&workspace, "demo");
    let unsupported = workspace.join("notes.txt");
    fs::write(&unsupported, "not an image").expect("fixture");
    let output = run(
        &workspace,
        &[
            "use-project",
            "demo",
            "create",
            "glyph",
            "--input",
            unsupported.to_str().unwrap(),
            "--json",
        ],
    );
    assert!(!output.status.success());
    let payload = json(&output);
    assert_envelope(&payload, "use-project.create.glyph", false);
    assert_eq!(payload["error"]["code"], "creation_stage_failed");
    assert_eq!(payload["error"]["stage"], "import");
    assert_eq!(
        payload["data"]["completed_stages"],
        serde_json::json!(["resolve"])
    );
    fs::remove_dir_all(workspace).expect("cleanup");
}

#[test]
fn animated_creation_commands_persist_standard_and_grid_metadata() {
    let workspace = temp_dir("animated");
    let project = new_project(&workspace, "demo");
    let first = workspace.join("frame-1.png");
    let second = workspace.join("frame-2.png");
    png(&first);
    png(&second);

    let standard = run(
        &workspace,
        &[
            "use-project",
            "demo",
            "create",
            "animated-glyph",
            "--input",
            first.to_str().unwrap(),
            second.to_str().unwrap(),
            "--name",
            "spinner",
            "--fps",
            "8",
            "--json",
        ],
    );
    assert!(
        standard.status.success(),
        "{}",
        String::from_utf8_lossy(&standard.stderr)
    );
    assert_envelope(&json(&standard), "use-project.create.animated-glyph", true);

    let grid = run(
        &workspace,
        &[
            "use-project",
            "demo",
            "create",
            "animated-grid-glyph",
            "--input",
            first.to_str().unwrap(),
            second.to_str().unwrap(),
            "--name",
            "dashboard",
            "--fps",
            "10",
            "--rows",
            "2",
            "--cols",
            "2",
            "--json",
        ],
    );
    assert!(
        grid.status.success(),
        "{}",
        String::from_utf8_lossy(&grid.stderr)
    );
    assert_envelope(&json(&grid), "use-project.create.animated-grid-glyph", true);

    let manifest: toml::Value =
        toml::from_str(&fs::read_to_string(project.join("petiglyph.toml")).expect("manifest"))
            .expect("valid manifest");
    let animations = manifest["animations"].as_array().expect("animations");
    assert_eq!(animations.len(), 2);
    assert_eq!(animations[0]["name"].as_str(), Some("spinner"));
    assert_eq!(animations[0]["type"].as_str(), Some("standard"));
    assert_eq!(animations[1]["name"].as_str(), Some("dashboard"));
    assert_eq!(animations[1]["type"].as_str(), Some("grid"));
    assert_eq!(animations[1]["rows"].as_integer(), Some(2));
    assert_eq!(animations[1]["cols"].as_integer(), Some(2));
    fs::remove_dir_all(workspace).expect("cleanup");
}
