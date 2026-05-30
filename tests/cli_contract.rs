use image::{Rgba, RgbaImage};
use serde_json::Value;
use std::env;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

fn make_temp_dir(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time is valid")
        .as_nanos();
    let dir = env::temp_dir().join(format!("petiglyph-cli-{name}-{nonce}"));
    fs::create_dir_all(&dir).expect("temp dir is created");
    dir
}

fn write_test_png(path: &Path) {
    let mut img = RgbaImage::from_pixel(8, 8, Rgba([255, 255, 255, 0]));
    img.put_pixel(2, 2, Rgba([0, 0, 0, 255]));
    img.put_pixel(5, 5, Rgba([0, 0, 0, 255]));
    img.save(path).expect("test image should be written");
}

fn run_petiglyph(
    cwd: &Path,
    args: &[&str],
    home_override: Option<&Path>,
    path_override: Option<&str>,
) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_petiglyph"));
    cmd.current_dir(cwd).args(args);

    if let Some(home) = home_override {
        cmd.env("HOME", home);
        cmd.env("USERPROFILE", home);
    }

    if let Some(path) = path_override {
        cmd.env("PATH", path);
    }

    cmd.output().expect("petiglyph command should run")
}

fn parse_json_stdout(output: &Output) -> Value {
    let stdout = String::from_utf8(output.stdout.clone()).expect("stdout is utf8");
    serde_json::from_str(stdout.trim()).expect("stdout is valid json")
}

fn assert_api_envelope(payload: &Value, command: &str, ok: bool) {
    assert_eq!(payload["ok"].as_bool(), Some(ok), "ok should match");
    assert_eq!(
        payload["command"].as_str(),
        Some(command),
        "command should match"
    );
    assert_eq!(
        payload["version"].as_str(),
        Some(env!("CARGO_PKG_VERSION")),
        "version should match package version"
    );
    assert!(payload.get("data").is_some(), "data field should exist");
}

#[test]
fn cli_help_describes_polished_command_surface() {
    let workspace = make_temp_dir("help-polish");

    let top = run_petiglyph(&workspace, &["--help"], None, None);
    assert!(top.status.success(), "top-level help should succeed");
    let top_stdout = String::from_utf8_lossy(&top.stdout);
    assert!(
        top_stdout.contains("set-threshold    Shortcut for `glyph set-threshold`"),
        "top-level threshold command should be described as a shortcut: {top_stdout}"
    );
    assert!(
        top_stdout.contains(
            "sample           Build, install, refresh font cache, and print the sample private-use string"
        ),
        "sample help should describe its install/cache behavior: {top_stdout}"
    );
    assert!(
        top_stdout.contains(
            "nuke-everything  Remove all petiglyph-managed user state (fonts, registry, and metadata)"
        ),
        "nuke-everything name should be kept with clear wording: {top_stdout}"
    );

    let animation = run_petiglyph(&workspace, &["animation", "--help"], None, None);
    assert!(animation.status.success(), "animation help should succeed");
    let animation_stdout = String::from_utf8_lossy(&animation.stdout);
    for expected in [
        "create-standard  Import media frames and create a standard animation",
        "create-grid      Import media frames and create a grid animation",
        "set-fps          Update an animation's frames-per-second value",
        "delete           Delete an animation definition from the project manifest",
    ] {
        assert!(
            animation_stdout.contains(expected),
            "animation help should contain `{expected}`: {animation_stdout}"
        );
    }

    fs::remove_dir_all(workspace).expect("temp dir is removed");
}

fn create_project_with_icon(workspace: &Path, project_name: &str) -> (PathBuf, PathBuf) {
    let project_dir = workspace.join(project_name);
    let create = run_petiglyph(
        workspace,
        &["create", project_name, "--no-launch"],
        None,
        None,
    );
    assert!(create.status.success(), "create command should succeed");

    let manifest_path = project_dir.join("petiglyph.toml");
    let icons_dir = project_dir.join("icons");
    write_test_png(&icons_dir.join("alpha.png"));

    (project_dir, manifest_path)
}

#[cfg(target_os = "linux")]
fn make_fake_fc_cache_path(workspace: &Path) -> String {
    let fake_bin = workspace.join("fake-bin");
    fs::create_dir_all(&fake_bin).expect("fake bin directory is created");

    let script_path = fake_bin.join("fc-cache");
    fs::write(
        &script_path,
        "#!/usr/bin/env bash\n# petiglyph test fc-cache shim\nexit 0\n",
    )
    .expect("fake fc-cache is written");

    #[cfg(unix)]
    {
        let mut perms = fs::metadata(&script_path)
            .expect("script metadata is readable")
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).expect("script permissions are updated");
    }

    format!(
        "{}:{}",
        fake_bin.display(),
        env::var("PATH").unwrap_or_default()
    )
}

#[test]
fn cli_list_json_returns_projects_and_fonts() {
    let workspace = make_temp_dir("list-json");
    let home = workspace.join("home");
    fs::create_dir_all(&home).expect("home dir is created");

    let (project_dir, manifest_path) = create_project_with_icon(&workspace, "list-demo");

    let output = run_petiglyph(&workspace, &["list", "--json"], Some(&home), None);
    assert!(output.status.success(), "list --json should succeed");

    let payload = parse_json_stdout(&output);
    assert_api_envelope(&payload, "list", true);

    let projects = payload["data"]["projects"]
        .as_array()
        .expect("projects array");
    assert_eq!(projects.len(), 1, "should detect the created project");
    assert_eq!(
        projects[0]["manifest_path"].as_str(),
        Some(manifest_path.to_string_lossy().as_ref()),
        "manifest path should match"
    );
    assert!(payload["data"]["installed_fonts"].as_array().is_some());

    fs::remove_dir_all(project_dir).expect("project dir is removed");
    fs::remove_dir_all(workspace).expect("temp dir is removed");
}

#[test]
fn cli_delete_json_removes_project() {
    let workspace = make_temp_dir("delete-json");
    let (project_dir, manifest_path) = create_project_with_icon(&workspace, "delete-demo");

    assert!(
        project_dir.exists(),
        "project dir should exist before delete"
    );

    let output = run_petiglyph(
        &workspace,
        &[
            "delete",
            "--json",
            "--manifest",
            manifest_path.to_str().unwrap(),
        ],
        None,
        None,
    );
    assert!(output.status.success(), "delete --json should succeed");

    let payload = parse_json_stdout(&output);
    assert_api_envelope(&payload, "delete", true);
    assert_eq!(
        payload["data"]["deleted_dir"].as_str(),
        Some(project_dir.to_string_lossy().as_ref())
    );

    assert!(!project_dir.exists(), "project dir should be deleted");

    fs::remove_dir_all(workspace).expect("temp dir is removed");
}

#[test]
fn cli_set_threshold_json_updates_manifest() {
    let workspace = make_temp_dir("set-threshold-json");
    let (project_dir, manifest_path) = create_project_with_icon(&workspace, "threshold-demo");

    let output = run_petiglyph(
        &project_dir,
        &["set-threshold", "alpha.png", "128", "--json"],
        None,
        None,
    );
    assert!(
        output.status.success(),
        "set-threshold --json should succeed"
    );

    let payload = parse_json_stdout(&output);
    assert_api_envelope(&payload, "set-threshold", true);
    assert_eq!(payload["data"]["image_name"].as_str(), Some("alpha.png"));
    assert_eq!(payload["data"]["threshold"].as_u64(), Some(128));

    let manifest_content = fs::read_to_string(&manifest_path).expect("manifest should be readable");
    assert!(
        manifest_content.contains("\"alpha.png\" = 128"),
        "manifest should contain the override"
    );

    fs::remove_dir_all(project_dir).expect("project dir is removed");
    fs::remove_dir_all(workspace).expect("temp dir is removed");
}

#[test]
fn cli_clear_threshold_json_updates_manifest() {
    let workspace = make_temp_dir("clear-threshold-json");
    let (project_dir, manifest_path) = create_project_with_icon(&workspace, "clear-threshold-demo");

    // First set the threshold
    run_petiglyph(
        &project_dir,
        &["set-threshold", "alpha.png", "128", "--json"],
        None,
        None,
    );

    // Then clear it
    let output = run_petiglyph(
        &project_dir,
        &["clear-threshold", "alpha.png", "--json"],
        None,
        None,
    );
    assert!(
        output.status.success(),
        "clear-threshold --json should succeed"
    );

    let payload = parse_json_stdout(&output);
    assert_api_envelope(&payload, "clear-threshold", true);
    assert_eq!(payload["data"]["image_name"].as_str(), Some("alpha.png"));
    assert_eq!(payload["data"]["was_present"].as_bool(), Some(true));

    let manifest_content = fs::read_to_string(&manifest_path).expect("manifest should be readable");
    assert!(
        !manifest_content.contains("\"alpha.png\" = 128"),
        "manifest should no longer contain the override"
    );

    fs::remove_dir_all(project_dir).expect("project dir is removed");
    fs::remove_dir_all(workspace).expect("temp dir is removed");
}

#[test]
fn cli_glyph_set_invert_json_updates_manifest() {
    let workspace = make_temp_dir("glyph-set-invert-json");
    let (project_dir, manifest_path) = create_project_with_icon(&workspace, "invert-demo");

    let output = run_petiglyph(
        &project_dir,
        &[
            "glyph",
            "set-invert",
            "alpha.png",
            "--invert",
            "on",
            "--json",
        ],
        None,
        None,
    );
    assert!(
        output.status.success(),
        "glyph set-invert --json should succeed"
    );
    let payload = parse_json_stdout(&output);
    assert_api_envelope(&payload, "glyph.set-invert", true);
    assert_eq!(payload["data"]["image_name"].as_str(), Some("alpha.png"));
    assert_eq!(payload["data"]["invert"].as_bool(), Some(true));

    let manifest_content = fs::read_to_string(&manifest_path).expect("manifest should be readable");
    assert!(
        manifest_content.contains("[invert_overrides]")
            && manifest_content.contains("\"alpha.png\" = true"),
        "manifest should persist invert override"
    );
}

#[test]
fn cli_composition_set_and_clear_json_updates_manifest() {
    let workspace = make_temp_dir("composition-set-clear-json");
    let (project_dir, manifest_path) = create_project_with_icon(&workspace, "composition-demo");

    let set_output = run_petiglyph(
        &project_dir,
        &[
            "composition",
            "set",
            "alpha.png",
            "--rows",
            "2",
            "--cols",
            "3",
            "--horizontal-bleed",
            "weak",
            "--vertical-bleed",
            "off",
            "--json",
        ],
        None,
        None,
    );
    assert!(
        set_output.status.success(),
        "composition set --json should succeed"
    );
    let set_payload = parse_json_stdout(&set_output);
    assert_api_envelope(&set_payload, "composition.set", true);

    let after_set = fs::read_to_string(&manifest_path).expect("manifest should be readable");
    let parsed_after_set: toml::Value = toml::from_str(&after_set).expect("valid manifest toml");
    let comp = parsed_after_set
        .get("compositions")
        .and_then(|v| v.get("alpha.png"))
        .expect("composition entry should exist");
    assert!(
        comp.get("rows").and_then(|v| v.as_integer()) == Some(2)
            && comp.get("cols").and_then(|v| v.as_integer()) == Some(3),
        "composition should be persisted"
    );

    let clear_output = run_petiglyph(
        &project_dir,
        &["composition", "clear", "alpha.png", "--json"],
        None,
        None,
    );
    assert!(
        clear_output.status.success(),
        "composition clear --json should succeed"
    );
    let clear_payload = parse_json_stdout(&clear_output);
    assert_api_envelope(&clear_payload, "composition.clear", true);

    let after_clear = fs::read_to_string(&manifest_path).expect("manifest should be readable");
    let parsed_after_clear: toml::Value =
        toml::from_str(&after_clear).expect("valid manifest toml");
    assert!(
        parsed_after_clear
            .get("compositions")
            .and_then(|v| v.get("alpha.png"))
            .is_none(),
        "composition should be removed"
    );
}

#[test]
fn cli_animation_create_set_fps_delete_json_updates_manifest() {
    let workspace = make_temp_dir("animation-lifecycle-json");
    let (project_dir, manifest_path) = create_project_with_icon(&workspace, "animation-demo");
    write_test_png(&project_dir.join("icons/frame2.png"));

    let create_output = run_petiglyph(
        &project_dir,
        &[
            "animation",
            "create-standard",
            "--input",
            "icons/alpha.png",
            "--input",
            "icons/frame2.png",
            "--name",
            "walk",
            "--fps",
            "8",
            "--json",
        ],
        None,
        None,
    );
    assert!(
        create_output.status.success(),
        "animation create-standard --json should succeed"
    );
    let create_payload = parse_json_stdout(&create_output);
    assert_api_envelope(&create_payload, "animation.create-standard", true);
    assert_eq!(create_payload["data"]["name"].as_str(), Some("walk"));
    assert_eq!(create_payload["data"]["fps"].as_u64(), Some(8));

    let set_fps_output = run_petiglyph(
        &project_dir,
        &["animation", "set-fps", "walk", "--fps", "10", "--json"],
        None,
        None,
    );
    assert!(
        set_fps_output.status.success(),
        "animation set-fps --json should succeed"
    );
    let set_fps_payload = parse_json_stdout(&set_fps_output);
    assert_api_envelope(&set_fps_payload, "animation.set-fps", true);
    assert_eq!(set_fps_payload["data"]["fps"].as_u64(), Some(10));

    let manifest_after_set = fs::read_to_string(&manifest_path).expect("manifest readable");
    assert!(
        manifest_after_set.contains("name = \"walk\"") && manifest_after_set.contains("fps = 10"),
        "manifest should reflect updated fps"
    );

    let delete_output = run_petiglyph(
        &project_dir,
        &["animation", "delete", "walk", "--json"],
        None,
        None,
    );
    assert!(
        delete_output.status.success(),
        "animation delete --json should succeed"
    );
    let delete_payload = parse_json_stdout(&delete_output);
    assert_api_envelope(&delete_payload, "animation.delete", true);

    let manifest_after_delete = fs::read_to_string(&manifest_path).expect("manifest readable");
    assert!(
        !manifest_after_delete.contains("name = \"walk\""),
        "animation should be removed from manifest"
    );
}

#[test]
fn cli_glyph_create_json_imports_and_persists_defaults() {
    let workspace = make_temp_dir("glyph-create-json");
    let (project_dir, manifest_path) = create_project_with_icon(&workspace, "glyph-create-demo");
    let source = workspace.join("glyph-source.png");
    write_test_png(&source);

    let output = run_petiglyph(
        &project_dir,
        &[
            "glyph",
            "create",
            "--input",
            source.to_str().expect("source path should be utf8"),
            "--threshold",
            "97",
            "--invert",
            "on",
            "--json",
        ],
        None,
        None,
    );
    assert!(
        output.status.success(),
        "glyph create --json should succeed"
    );
    let payload = parse_json_stdout(&output);
    assert_api_envelope(&payload, "glyph.create", true);
    assert_eq!(
        payload["data"]["imported_sources"]
            .as_array()
            .map(|v| v.len()),
        Some(1),
        "one source should be imported"
    );

    let manifest_content = fs::read_to_string(&manifest_path).expect("manifest should be readable");
    assert!(
        manifest_content.contains("= 97"),
        "manifest should persist threshold override for imported source"
    );
    assert!(
        manifest_content.contains("[invert_overrides]"),
        "manifest should persist invert override section"
    );
}

#[test]
fn cli_grid_create_json_persists_composition_and_rejects_multiple_sources() {
    let workspace = make_temp_dir("grid-create-json");
    let (project_dir, manifest_path) = create_project_with_icon(&workspace, "grid-create-demo");
    let source1 = workspace.join("grid-source-1.png");
    let source2 = workspace.join("grid-source-2.png");
    write_test_png(&source1);
    write_test_png(&source2);

    let ok_output = run_petiglyph(
        &project_dir,
        &[
            "grid",
            "create",
            "--input",
            source1.to_str().expect("source path should be utf8"),
            "--rows",
            "2",
            "--cols",
            "2",
            "--horizontal-bleed",
            "strong",
            "--vertical-bleed",
            "weak",
            "--threshold",
            "111",
            "--json",
        ],
        None,
        None,
    );
    assert!(
        ok_output.status.success(),
        "grid create --json should succeed"
    );
    let ok_payload = parse_json_stdout(&ok_output);
    assert_api_envelope(&ok_payload, "grid.create", true);

    let manifest_content = fs::read_to_string(&manifest_path).expect("manifest should be readable");
    assert!(
        manifest_content.contains("rows = 2")
            && manifest_content.contains("cols = 2")
            && manifest_content.contains("horizontal_bleed = \"strong\"")
            && manifest_content.contains("vertical_bleed = \"weak\""),
        "grid composition settings should be persisted"
    );

    let error_output = run_petiglyph(
        &project_dir,
        &[
            "grid",
            "create",
            "--input",
            source1.to_str().expect("source path should be utf8"),
            "--input",
            source2.to_str().expect("source path should be utf8"),
            "--rows",
            "2",
            "--cols",
            "2",
            "--json",
        ],
        None,
        None,
    );
    assert!(
        !error_output.status.success(),
        "grid create should fail when multiple sources are imported"
    );
    let error_payload = parse_json_stdout(&error_output);
    assert_api_envelope(&error_payload, "grid.create", false);
    assert!(
        error_payload["error"]["message"]
            .as_str()
            .expect("error message")
            .contains("exactly one imported source"),
        "error should explain single-source requirement"
    );
}

#[test]
fn cli_animation_create_grid_json_sets_animation_and_compositions() {
    let workspace = make_temp_dir("animation-create-grid-json");
    let (project_dir, manifest_path) = create_project_with_icon(&workspace, "anim-grid-demo");
    let frame2 = workspace.join("frame-b.png");
    write_test_png(&frame2);

    let output = run_petiglyph(
        &project_dir,
        &[
            "animation",
            "create-grid",
            "--input",
            "icons/alpha.png",
            "--input",
            frame2.to_str().expect("frame2 path should be utf8"),
            "--name",
            "gridwalk",
            "--fps",
            "12",
            "--rows",
            "2",
            "--cols",
            "2",
            "--horizontal-bleed",
            "weak",
            "--vertical-bleed",
            "off",
            "--json",
        ],
        None,
        None,
    );
    assert!(
        output.status.success(),
        "animation create-grid --json should succeed"
    );
    let payload = parse_json_stdout(&output);
    assert_api_envelope(&payload, "animation.create-grid", true);
    assert_eq!(payload["data"]["name"].as_str(), Some("gridwalk"));
    assert_eq!(payload["data"]["fps"].as_u64(), Some(12));
    assert_eq!(payload["data"]["frame_count"].as_u64(), Some(2));

    let manifest_content = fs::read_to_string(&manifest_path).expect("manifest should be readable");
    let parsed: toml::Value = toml::from_str(&manifest_content).expect("valid manifest toml");
    let animations = parsed
        .get("animations")
        .and_then(|v| v.as_array())
        .expect("animations should be an array");
    let anim = animations
        .iter()
        .find(|entry| entry.get("name").and_then(|v| v.as_str()) == Some("gridwalk"))
        .expect("gridwalk animation should exist");
    assert_eq!(anim.get("rows").and_then(|v| v.as_integer()), Some(2));
    assert_eq!(anim.get("cols").and_then(|v| v.as_integer()), Some(2));
}

#[test]
fn cli_animation_set_fps_json_rejects_out_of_range() {
    let workspace = make_temp_dir("animation-set-fps-invalid-json");
    let (project_dir, _) = create_project_with_icon(&workspace, "anim-fps-demo");

    let output = run_petiglyph(
        &project_dir,
        &["animation", "set-fps", "missing", "--fps", "0", "--json"],
        None,
        None,
    );
    assert!(
        !output.status.success(),
        "animation set-fps should reject invalid fps"
    );
    let payload = parse_json_stdout(&output);
    assert_api_envelope(&payload, "animation.set-fps", false);
    assert!(
        payload["error"]["message"]
            .as_str()
            .expect("error message")
            .contains("fps must be in 1..=30"),
        "error should include fps range"
    );
}

#[test]
fn cli_legacy_and_nested_threshold_commands_match_manifest_mutation() {
    let workspace = make_temp_dir("threshold-parity-json");
    let (project_dir, manifest_path) =
        create_project_with_icon(&workspace, "threshold-parity-demo");

    let legacy_output = run_petiglyph(
        &project_dir,
        &["set-threshold", "alpha.png", "140", "--json"],
        None,
        None,
    );
    assert!(
        legacy_output.status.success(),
        "legacy set-threshold should succeed"
    );
    let legacy_payload = parse_json_stdout(&legacy_output);
    assert_api_envelope(&legacy_payload, "set-threshold", true);

    let nested_output = run_petiglyph(
        &project_dir,
        &["glyph", "set-threshold", "alpha.png", "141", "--json"],
        None,
        None,
    );
    assert!(
        nested_output.status.success(),
        "nested glyph set-threshold should succeed"
    );
    let nested_payload = parse_json_stdout(&nested_output);
    assert_api_envelope(&nested_payload, "glyph.set-threshold", true);

    let clear_nested_output = run_petiglyph(
        &project_dir,
        &["glyph", "clear-threshold", "alpha.png", "--json"],
        None,
        None,
    );
    assert!(
        clear_nested_output.status.success(),
        "nested glyph clear-threshold should succeed"
    );
    let clear_nested_payload = parse_json_stdout(&clear_nested_output);
    assert_api_envelope(&clear_nested_payload, "glyph.clear-threshold", true);

    let manifest_content = fs::read_to_string(&manifest_path).expect("manifest should be readable");
    assert!(
        !manifest_content.contains("\"alpha.png\" = 141"),
        "clear-threshold alias should remove the override"
    );
}

#[test]
fn cli_no_subcommand_errors_without_manifest() {
    let workspace = make_temp_dir("no-manifest");

    let output = run_petiglyph(&workspace, &[], None, None);

    assert!(
        !output.status.success(),
        "command should fail without manifest"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("interactive petiglyph TUI requires a terminal"),
        "stderr should mention interactive TUI requirement in non-tty runs: {stderr}"
    );

    fs::remove_dir_all(workspace).expect("temp dir is removed");
}

#[test]
fn cli_nested_manifest_autodetection_works_for_single_project() {
    let workspace = make_temp_dir("nested-autodetect");
    let (project_dir, manifest_path) = create_project_with_icon(&workspace, "demo-font");

    let output = run_petiglyph(&workspace, &["uninstall-font", "--json"], None, None);
    assert!(
        output.status.success(),
        "uninstall-font --json should succeed when one nested manifest exists"
    );

    let payload = parse_json_stdout(&output);
    assert_api_envelope(&payload, "uninstall-font", true);
    assert_eq!(
        payload["data"]["manifest"].as_str(),
        Some(manifest_path.to_string_lossy().as_ref()),
        "autodetected manifest should point to nested project"
    );

    fs::remove_dir_all(project_dir).expect("project dir is removed");
    fs::remove_dir_all(workspace).expect("temp dir is removed");
}

#[test]
fn cli_human_build_and_sample_workflow() {
    let workspace = make_temp_dir("workflow-human");
    let (project_dir, _) = create_project_with_icon(&workspace, "demo-font");

    let build = run_petiglyph(&project_dir, &["build"], None, None);
    assert!(build.status.success(), "build command should succeed");
    assert!(project_dir.join("build/glyph-map.json").exists());
    assert!(project_dir.join("build/glyph-sample.txt").exists());
    assert!(project_dir.join("build/previews/alpha.png").exists());

    #[cfg(target_os = "linux")]
    let sample = {
        let home = workspace.join("home");
        fs::create_dir_all(&home).expect("home dir is created");
        let fake_path = make_fake_fc_cache_path(&workspace);
        run_petiglyph(&project_dir, &["sample"], Some(&home), Some(&fake_path))
    };
    #[cfg(not(target_os = "linux"))]
    let sample = run_petiglyph(&project_dir, &["sample"], None, None);
    assert!(sample.status.success(), "sample command should succeed");
    let sample_stdout = String::from_utf8_lossy(&sample.stdout);
    assert!(
        sample_stdout.contains("petiglyph sample"),
        "sample output should include header: {sample_stdout}"
    );
    assert!(
        sample_stdout.contains("installed:"),
        "sample output should include installed artifact path: {sample_stdout}"
    );

    fs::remove_dir_all(workspace).expect("temp dir is removed");
}

#[test]
fn cli_build_json_schema_and_exit_code() {
    let workspace = make_temp_dir("build-json");
    let (project_dir, _) = create_project_with_icon(&workspace, "api-font");

    let output = run_petiglyph(&project_dir, &["build", "--json"], None, None);
    assert!(output.status.success(), "build --json should succeed");
    assert!(
        output.stderr.is_empty(),
        "json mode should keep diagnostics off stderr on success"
    );

    let payload = parse_json_stdout(&output);
    assert_api_envelope(&payload, "build", true);
    assert_eq!(
        payload["data"]["glyph_count"].as_u64(),
        Some(1),
        "glyph count should match test icon set"
    );
    assert!(
        payload["data"]["ttf"]
            .as_str()
            .expect("ttf path")
            .ends_with(".ttf")
    );

    fs::remove_dir_all(workspace).expect("temp dir is removed");
}

#[test]
fn cli_build_json_failure_returns_error_payload() {
    let workspace = make_temp_dir("build-json-failure");
    let project_dir = workspace.join("empty-project");

    let create = run_petiglyph(
        &workspace,
        &["create", "empty-project", "--no-launch"],
        None,
        None,
    );
    assert!(create.status.success(), "create command should succeed");

    let output = run_petiglyph(&project_dir, &["build", "--json"], None, None);
    assert!(
        !output.status.success(),
        "build --json should fail with no icons"
    );
    assert!(
        output.stderr.is_empty(),
        "json mode should avoid stderr noise for machine callers"
    );

    let payload = parse_json_stdout(&output);
    assert_api_envelope(&payload, "build", false);
    assert!(
        payload["error"]["message"]
            .as_str()
            .expect("error message")
            .contains("no supported images found"),
        "error payload should carry actionable message"
    );

    fs::remove_dir_all(workspace).expect("temp dir is removed");
}

#[test]
fn cli_doctor_json_reports_global_health_without_manifest() {
    let workspace = make_temp_dir("doctor-json-global");
    let home = workspace.join("home");
    fs::create_dir_all(&home).expect("home dir is created");

    let output = run_petiglyph(&workspace, &["doctor", "--json"], Some(&home), None);
    assert!(output.status.success(), "doctor --json should succeed");
    assert!(
        output.stderr.is_empty(),
        "doctor --json should keep stderr clean on success"
    );

    let payload = parse_json_stdout(&output);
    assert_api_envelope(&payload, "doctor", true);
    assert_eq!(payload["data"]["repair"].as_bool(), Some(false));
    assert!(payload["data"]["install_dir"].as_str().is_some());
    assert!(payload["data"]["registry_path"].as_str().is_some());
    assert!(payload["data"]["findings"].as_array().is_some());

    fs::remove_dir_all(workspace).expect("temp dir is removed");
}

#[test]
fn cli_tool_uninstall_json_is_idempotent() {
    let workspace = make_temp_dir("tool-uninstall-json");
    let home = workspace.join("home");
    fs::create_dir_all(&home).expect("home dir is created");

    let output = run_petiglyph(
        &workspace,
        &["nuke-everything", "--json"],
        Some(&home),
        None,
    );
    assert!(
        output.status.success(),
        "nuke-everything --json should succeed"
    );
    assert!(
        output.stderr.is_empty(),
        "nuke-everything --json should keep stderr clean on success"
    );

    let payload = parse_json_stdout(&output);
    assert_api_envelope(&payload, "nuke-everything", true);
    assert_eq!(payload["data"]["outcome"].as_str(), Some("already_absent"));
    assert_eq!(payload["data"]["removed_ttf_count"].as_u64(), Some(0));
    assert_eq!(payload["data"]["removed_metadata_count"].as_u64(), Some(0));
    assert_eq!(
        payload["data"]["removed_state_file_count"].as_u64(),
        Some(0)
    );

    fs::remove_dir_all(workspace).expect("temp dir is removed");
}

#[cfg(target_os = "linux")]
#[test]
fn cli_tool_uninstall_json_removes_managed_install_state() {
    let workspace = make_temp_dir("tool-uninstall-removes-state");
    let (project_dir, _) = create_project_with_icon(&workspace, "tool-uninstall-demo");
    let home = workspace.join("home");
    fs::create_dir_all(&home).expect("home dir is created");
    let fake_path = make_fake_fc_cache_path(&workspace);

    let install = run_petiglyph(
        &project_dir,
        &["install-font", "--json"],
        Some(&home),
        Some(&fake_path),
    );
    assert!(
        install.status.success(),
        "install-font --json should succeed"
    );

    let uninstall = run_petiglyph(
        &workspace,
        &["nuke-everything", "--json"],
        Some(&home),
        Some(&fake_path),
    );
    assert!(
        uninstall.status.success(),
        "nuke-everything --json should succeed"
    );
    assert!(
        uninstall.stderr.is_empty(),
        "nuke-everything --json should keep stderr clean on success"
    );

    let payload = parse_json_stdout(&uninstall);
    assert_api_envelope(&payload, "nuke-everything", true);
    assert_eq!(payload["data"]["outcome"].as_str(), Some("removed"));
    assert!(
        payload["data"]["removed_ttf_count"].as_u64().unwrap_or(0) >= 1,
        "tool uninstall should remove at least one managed ttf"
    );
    assert!(
        payload["data"]["removed_metadata_count"]
            .as_u64()
            .unwrap_or(0)
            >= 1,
        "tool uninstall should remove managed metadata"
    );
    assert!(
        payload["data"]["removed_state_file_count"]
            .as_u64()
            .unwrap_or(0)
            >= 1,
        "tool uninstall should remove managed state files"
    );
    let install_dir = PathBuf::from(
        payload["data"]["install_dir"]
            .as_str()
            .expect("install dir should be present in uninstall payload"),
    );
    assert!(
        !install_dir.exists(),
        "tool uninstall should remove empty install directory"
    );

    fs::remove_dir_all(workspace).expect("temp dir is removed");
}

#[cfg(target_os = "linux")]
#[test]
fn cli_install_and_uninstall_json_lifecycle_is_idempotent() {
    let workspace = make_temp_dir("install-lifecycle");
    let (project_dir, _) = create_project_with_icon(&workspace, "demo-font");
    let home = workspace.join("home");
    fs::create_dir_all(&home).expect("home dir is created");

    let fake_path = make_fake_fc_cache_path(&workspace);

    let install_1 = run_petiglyph(
        &project_dir,
        &["install-font", "--json"],
        Some(&home),
        Some(&fake_path),
    );
    assert!(install_1.status.success(), "first install should succeed");
    let install_1_payload = parse_json_stdout(&install_1);
    assert_api_envelope(&install_1_payload, "install-font", true);
    assert_eq!(
        install_1_payload["data"]["platform"].as_str(),
        Some("linux")
    );
    let installed_ttf_1 = install_1_payload["data"]["installed_ttf"]
        .as_str()
        .expect("installed ttf");
    assert!(
        installed_ttf_1.contains("/.local/share/fonts/petiglyph/demo_font")
            && installed_ttf_1.ends_with(".ttf")
            && !installed_ttf_1.contains("demo_font_demo_font_"),
        "CLI install should use progressive immutable artifact naming, got {installed_ttf_1}"
    );
    let alias_path = home
        .join(".config")
        .join("fontconfig")
        .join("conf.d")
        .join("99-petiglyph.conf");
    assert!(
        alias_path.exists(),
        "linux install should publish petiglyph fontconfig alias"
    );
    assert_eq!(
        install_1_payload["data"]["replaced_previous_ttf_count"].as_u64(),
        Some(0)
    );

    let install_2 = run_petiglyph(
        &project_dir,
        &["install-font", "--json"],
        Some(&home),
        Some(&fake_path),
    );
    assert!(install_2.status.success(), "second install should succeed");
    let install_2_payload = parse_json_stdout(&install_2);
    assert_api_envelope(&install_2_payload, "install-font", true);
    assert_eq!(
        install_2_payload["data"]["replaced_previous_ttf_count"].as_u64(),
        Some(0),
        "second identical install should keep immutable artifact without replacement"
    );

    let uninstall_1 = run_petiglyph(
        &project_dir,
        &["uninstall-font", "--json"],
        Some(&home),
        Some(&fake_path),
    );
    assert!(
        uninstall_1.status.success(),
        "first uninstall should succeed"
    );
    let uninstall_1_payload = parse_json_stdout(&uninstall_1);
    assert_api_envelope(&uninstall_1_payload, "uninstall-font", true);
    assert_eq!(
        uninstall_1_payload["data"]["outcome"].as_str(),
        Some("removed")
    );
    assert_eq!(
        uninstall_1_payload["data"]["removed_ttf_count"].as_u64(),
        Some(1)
    );

    let uninstall_2 = run_petiglyph(
        &project_dir,
        &["uninstall-font", "--json"],
        Some(&home),
        Some(&fake_path),
    );
    assert!(
        uninstall_2.status.success(),
        "second uninstall should succeed"
    );
    let uninstall_2_payload = parse_json_stdout(&uninstall_2);
    assert_api_envelope(&uninstall_2_payload, "uninstall-font", true);
    assert_eq!(
        uninstall_2_payload["data"]["outcome"].as_str(),
        Some("already_absent")
    );
    assert!(
        !alias_path.exists(),
        "fontconfig alias should be removed when no managed fonts remain"
    );

    let installed_dir = home
        .join(".local")
        .join("share")
        .join("fonts")
        .join("petiglyph");
    let plain_path = installed_dir.join("demo_font.ttf");
    let prefixed_path = installed_dir.join("demo_font_demo_font.ttf");
    assert!(
        !plain_path.exists(),
        "plain install candidate should be absent after uninstall"
    );
    assert!(
        !prefixed_path.exists(),
        "project-prefixed install candidate should be absent after uninstall"
    );
    let remaining_ttf_count = fs::read_dir(&installed_dir)
        .ok()
        .into_iter()
        .flat_map(|entries| entries.filter_map(|entry| entry.ok()))
        .filter(|entry| {
            entry
                .path()
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("ttf"))
        })
        .count();
    assert_eq!(
        remaining_ttf_count, 0,
        "immutable install artifacts should be fully removed on uninstall"
    );

    fs::remove_dir_all(workspace).expect("temp dir is removed");
}

#[cfg(target_os = "linux")]
#[test]
fn cli_install_identity_isolated_even_for_slug_collisions() {
    let workspace = make_temp_dir("install-slug-collision");
    let (project_a, _) = create_project_with_icon(&workspace, "my-proj");
    let (project_b, _) = create_project_with_icon(&workspace, "my_proj");
    let home = workspace.join("home");
    fs::create_dir_all(&home).expect("home dir is created");
    let fake_path = make_fake_fc_cache_path(&workspace);

    let install_a = run_petiglyph(
        &project_a,
        &["install-font", "--json"],
        Some(&home),
        Some(&fake_path),
    );
    assert!(
        install_a.status.success(),
        "project A install should succeed"
    );
    let payload_a = parse_json_stdout(&install_a);
    assert_api_envelope(&payload_a, "install-font", true);
    let installed_a = payload_a["data"]["installed_ttf"]
        .as_str()
        .expect("project A installed_ttf")
        .to_string();

    let install_b = run_petiglyph(
        &project_b,
        &["install-font", "--json"],
        Some(&home),
        Some(&fake_path),
    );
    assert!(
        install_b.status.success(),
        "project B install should succeed"
    );
    let payload_b = parse_json_stdout(&install_b);
    assert_api_envelope(&payload_b, "install-font", true);
    let installed_b = payload_b["data"]["installed_ttf"]
        .as_str()
        .expect("project B installed_ttf")
        .to_string();

    assert_ne!(
        installed_a, installed_b,
        "install artifacts must remain isolated even when project slugs collide"
    );

    fs::remove_dir_all(workspace).expect("temp dir is removed");
}
