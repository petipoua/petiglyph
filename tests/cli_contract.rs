use image::{Rgba, RgbaImage};
use serde_json::Value;
use std::env;
use std::fs;
use std::fs::{File, FileTimes};
#[cfg(target_os = "linux")]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn make_temp_dir(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time is valid")
        .as_nanos();
    let dir = env::temp_dir().join(format!("petiglyph-cli-{name}-{nonce}"));
    fs::create_dir_all(&dir).expect("temp dir is created");
    dir
}

fn fake_path_without_ffmpeg(workspace: &Path) -> String {
    let fake_bin = workspace.join("fake-bin-no-ffmpeg");
    fs::create_dir_all(&fake_bin).expect("fake path directory is created");
    fake_bin.display().to_string()
}

fn ffmpeg_prompt_state_path(home: &Path) -> PathBuf {
    if cfg!(target_os = "linux") {
        home.join(".local/share/fonts/petiglyph/.ffmpeg-setup-prompt-v1.json")
    } else if cfg!(target_os = "macos") {
        home.join("Library/Fonts/petiglyph/.ffmpeg-setup-prompt-v1.json")
    } else if cfg!(target_os = "windows") {
        home.join("AppData/Local/Microsoft/Windows/Fonts/petiglyph/.ffmpeg-setup-prompt-v1.json")
    } else {
        home.join(".ffmpeg-setup-prompt-v1.json")
    }
}

fn make_stale_file(path: &Path) {
    fs::write(path, "stale lock").expect("stale lock file is written");
    let stale_time = SystemTime::now() - Duration::from_secs(3600);
    let times = FileTimes::new().set_modified(stale_time);
    File::options()
        .write(true)
        .open(path)
        .expect("stale lock should be openable")
        .set_times(times)
        .expect("stale mtime should be set");
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

    let default_home = cwd.join(".petiglyph-test-home");
    let home = home_override.unwrap_or(default_home.as_path());
    fs::create_dir_all(home).expect("home dir is created");
    cmd.env("HOME", home);
    cmd.env("USERPROFILE", home);
    cmd.env("LOCALAPPDATA", home.join("AppData/Local"));
    cmd.env("APPDATA", home.join("AppData/Roaming"));
    cmd.env("XDG_CONFIG_HOME", home.join(".config"));
    cmd.env("XDG_DATA_HOME", home.join(".local/share"));
    cmd.env("PWD", cwd);

    if let Some(path) = path_override {
        cmd.env("PATH", path);
    }

    cmd.output().expect("petiglyph command should run")
}

fn same_path(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }

    let left_canonical = left.canonicalize().ok();
    let right_canonical = right.canonicalize().ok();
    matches!((left_canonical, right_canonical), (Some(a), Some(b)) if a == b)
}

fn parse_json_stdout(output: &Output) -> Value {
    let stdout = String::from_utf8(output.stdout.clone()).expect("stdout is utf8");
    serde_json::from_str(stdout.trim()).expect("stdout is valid json")
}

fn parse_manifest_toml(manifest_path: &Path) -> toml::Value {
    let manifest_content = fs::read_to_string(manifest_path).expect("manifest should be readable");
    toml::from_str(&manifest_content).expect("manifest should be valid toml")
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
        top_stdout.contains("set-threshold")
            && top_stdout.contains("Shortcut for `glyph set-threshold`"),
        "top-level threshold command should be described as a shortcut: {top_stdout}"
    );
    assert!(
        top_stdout.contains("sample")
            && top_stdout.contains(
                "Build, install, refresh font cache, and print the sample private-use string"
            ),
        "sample help should describe its install/cache behavior: {top_stdout}"
    );
    assert!(
        top_stdout.contains("uninstall-all-fonts")
            && top_stdout.contains(
                "Remove all managed installed petiglyph fonts and install metadata for the current user"
            ),
        "uninstall-all-fonts help should keep clear wording: {top_stdout}"
    );
    assert!(
        top_stdout.contains("--ffmpeg-auto-install"),
        "top-level help should document the ffmpeg auto-install opt-in flag: {top_stdout}"
    );

    let animation = run_petiglyph(&workspace, &["animation", "--help"], None, None);
    assert!(animation.status.success(), "animation help should succeed");
    let animation_stdout = String::from_utf8_lossy(&animation.stdout);
    for expected in ["create-standard", "create-grid", "set-fps", "delete"] {
        assert!(
            animation_stdout.contains(expected),
            "animation help should contain `{expected}`: {animation_stdout}"
        );
    }
    for expected in [
        "Import media frames and create a standard animation",
        "Import media frames and create a grid animation",
        "Update an animation's frames-per-second value",
        "Delete an animation definition from the project manifest",
    ] {
        assert!(
            animation_stdout.contains(expected),
            "animation help should contain description fragment `{expected}`: {animation_stdout}"
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
    assert!(
        projects[0]["manifest_path"]
            .as_str()
            .is_some_and(|value| same_path(Path::new(value), &manifest_path)),
        "manifest path should match"
    );
    assert!(payload["data"]["installed_fonts"].as_array().is_some());

    fs::remove_dir_all(project_dir).expect("project dir is removed");
    fs::remove_dir_all(workspace).expect("temp dir is removed");
}

#[test]
fn cli_list_json_reports_malformed_manifests_as_warnings() {
    let workspace = make_temp_dir("list-json-malformed-manifest");
    let home = workspace.join("home");
    fs::create_dir_all(&home).expect("home dir is created");

    let (project_dir, manifest_path) = create_project_with_icon(&workspace, "list-valid");
    let broken_dir = workspace.join("list-broken");
    fs::create_dir_all(&broken_dir).expect("broken project dir is created");
    let broken_manifest = broken_dir.join("petiglyph.toml");
    fs::write(&broken_manifest, "this is not valid toml = [").expect("broken manifest is written");

    let output = run_petiglyph(&workspace, &["list", "--json"], Some(&home), None);
    assert!(output.status.success(), "list --json should succeed");

    let payload = parse_json_stdout(&output);
    assert_api_envelope(&payload, "list", true);

    let projects = payload["data"]["projects"]
        .as_array()
        .expect("projects array");
    assert!(
        projects.iter().any(|project| {
            project["manifest_path"]
                .as_str()
                .is_some_and(|value| same_path(Path::new(value), &manifest_path))
        }),
        "valid project should still be listed"
    );

    let warnings = payload["data"]["warnings"]
        .as_array()
        .expect("warnings array");
    assert!(
        warnings.iter().any(|warning| {
            warning["code"].as_str() == Some("manifest_read_failed")
                && warning["manifest_path"]
                    .as_str()
                    .is_some_and(|value| same_path(Path::new(value), &broken_manifest))
                && warning["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("failed to parse"))
        }),
        "malformed manifest warning should be present"
    );

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
fn cli_delete_rejects_relative_manifest_when_called_inside_project_dir() {
    let workspace = make_temp_dir("delete-inside-project");
    let (project_dir, _) = create_project_with_icon(&workspace, "delete-guard-demo");

    let output = run_petiglyph(
        &project_dir,
        &["delete", "--manifest", "petiglyph.toml", "--json"],
        None,
        None,
    );
    assert!(
        !output.status.success(),
        "delete should fail when called from inside the project directory"
    );

    let payload = parse_json_stdout(&output);
    assert_api_envelope(&payload, "delete", false);
    let message = payload["error"]["message"].as_str().expect("error message");
    assert!(
        message.contains("refusing to delete project root from inside that project directory"),
        "unexpected error message: {message}"
    );
    assert!(
        project_dir.exists(),
        "project dir should remain after guarded delete attempt"
    );

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

    let manifest = parse_manifest_toml(&manifest_path);
    assert_eq!(
        manifest
            .get("threshold_overrides")
            .and_then(|v| v.get("alpha.png"))
            .and_then(|v| v.as_integer()),
        Some(128),
        "manifest should contain the threshold override at threshold_overrides.alpha.png"
    );

    fs::remove_dir_all(project_dir).expect("project dir is removed");
    fs::remove_dir_all(workspace).expect("temp dir is removed");
}

#[test]
fn cli_glyph_create_json_fails_when_grayscale_processing_fails() {
    let workspace = make_temp_dir("glyph-create-grayscale-failure-json");
    let (project_dir, _) = create_project_with_icon(&workspace, "glyph-gray-fail-demo");
    let invalid_png = workspace.join("invalid.png");
    fs::write(&invalid_png, b"not-a-real-image").expect("invalid png bytes are written");

    let output = run_petiglyph(
        &project_dir,
        &[
            "glyph",
            "create",
            "--input",
            invalid_png.to_str().expect("path should be utf8"),
            "--grayscale-enabled",
            "--json",
        ],
        None,
        None,
    );
    assert!(
        !output.status.success(),
        "glyph create should fail when grayscale processing fails"
    );

    let payload = parse_json_stdout(&output);
    assert_api_envelope(&payload, "glyph.create", false);
    let message = payload["error"]["message"].as_str().expect("error message");
    assert!(
        message.contains("failed to apply grayscale processing to imported file"),
        "unexpected error message: {message}"
    );
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

    let manifest = parse_manifest_toml(&manifest_path);
    assert!(
        manifest
            .get("threshold_overrides")
            .and_then(|v| v.get("alpha.png"))
            .is_none(),
        "manifest should no longer contain threshold_overrides.alpha.png"
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

    let manifest_after_set = parse_manifest_toml(&manifest_path);
    let walk_after_set = manifest_after_set
        .get("animations")
        .and_then(|v| v.as_array())
        .and_then(|animations| {
            animations
                .iter()
                .find(|entry| entry.get("name").and_then(|v| v.as_str()) == Some("walk"))
        })
        .expect("walk animation should exist after set-fps");
    assert_eq!(
        walk_after_set.get("fps").and_then(|v| v.as_integer()),
        Some(10),
        "manifest should reflect updated fps for walk"
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

    let manifest_after_delete = parse_manifest_toml(&manifest_path);
    let has_walk = manifest_after_delete
        .get("animations")
        .and_then(|v| v.as_array())
        .is_some_and(|animations| {
            animations
                .iter()
                .any(|entry| entry.get("name").and_then(|v| v.as_str()) == Some("walk"))
        });
    assert!(!has_walk, "walk animation should be removed from manifest");
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

    let manifest = parse_manifest_toml(&manifest_path);
    assert!(
        manifest
            .get("threshold_overrides")
            .and_then(|v| v.get("alpha.png"))
            .is_none(),
        "clear-threshold alias should remove threshold_overrides.alpha.png"
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
fn cli_hidden_uninstall_stub_returns_guidance_and_non_zero() {
    let workspace = make_temp_dir("hidden-uninstall");
    let output = run_petiglyph(&workspace, &["uninstall"], None, None);
    assert!(
        !output.status.success(),
        "hidden uninstall stub should exit non-zero"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("uninstall is ambiguous"),
        "stderr should explain ambiguity: {stderr}"
    );
    assert!(
        stderr.contains("uninstall-font") && stderr.contains("uninstall-all-fonts"),
        "stderr should include both guidance commands: {stderr}"
    );
}

#[test]
fn cli_tui_non_tty_without_manifest_errors_cleanly() {
    let workspace = make_temp_dir("tui-non-tty-no-manifest");
    let output = run_petiglyph(&workspace, &["tui"], None, None);
    assert!(
        !output.status.success(),
        "tui should fail in non-tty contexts"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("interactive petiglyph TUI requires a terminal"),
        "stderr should include terminal-required guidance: {stderr}"
    );
}

#[test]
fn cli_tui_non_tty_with_manifest_errors_cleanly() {
    let workspace = make_temp_dir("tui-non-tty-with-manifest");
    let (_, manifest_path) = create_project_with_icon(&workspace, "tui-demo");
    let output = run_petiglyph(
        &workspace,
        &[
            "tui",
            "--manifest",
            manifest_path.to_str().expect("manifest should be utf8"),
        ],
        None,
        None,
    );
    assert!(
        !output.status.success(),
        "tui --manifest should fail in non-tty contexts"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("requires a terminal")
            || stderr.contains("No such device or address")
            || stderr.contains("not a terminal"),
        "stderr should report terminal requirement: {stderr}"
    );
}

#[test]
fn cli_json_commands_do_not_trigger_ffmpeg_prompt_or_state() {
    let workspace = make_temp_dir("json-no-ffmpeg-prompt");
    let home = workspace.join("home");
    fs::create_dir_all(&home).expect("home dir is created");
    let (_, manifest_path) = create_project_with_icon(&workspace, "json-ffmpeg-demo");
    let fake_path = fake_path_without_ffmpeg(&workspace);
    let state_path = ffmpeg_prompt_state_path(&home);

    let list = run_petiglyph(
        &workspace,
        &["list", "--json"],
        Some(&home),
        Some(&fake_path),
    );
    assert!(list.status.success(), "list --json should succeed");
    let list_payload = parse_json_stdout(&list);
    assert_api_envelope(&list_payload, "list", true);
    let list_stdout = String::from_utf8_lossy(&list.stdout);
    assert!(
        !list_stdout.contains("FFmpeg was not found."),
        "json output must not include ffmpeg prompt text"
    );

    let build = run_petiglyph(
        manifest_path.parent().expect("project dir"),
        &["build", "--json"],
        Some(&home),
        Some(&fake_path),
    );
    assert!(build.status.success(), "build --json should succeed");
    let build_payload = parse_json_stdout(&build);
    assert_api_envelope(&build_payload, "build", true);
    let build_stdout = String::from_utf8_lossy(&build.stdout);
    assert!(
        !build_stdout.contains("FFmpeg was not found."),
        "json output must not include ffmpeg prompt text"
    );

    assert!(
        !state_path.exists(),
        "json commands should not write ffmpeg prompt state"
    );
}

#[test]
fn cli_unsupported_import_file_errors_for_create_workflows() {
    let workspace = make_temp_dir("unsupported-imports");
    let (project_dir, _) = create_project_with_icon(&workspace, "unsupported-demo");
    let unsupported = workspace.join("not-an-image.txt");
    fs::write(&unsupported, "plain text").expect("unsupported fixture is written");

    let glyph = run_petiglyph(
        &project_dir,
        &[
            "glyph",
            "create",
            "--input",
            unsupported.to_str().expect("path should be utf8"),
            "--json",
        ],
        None,
        None,
    );
    assert!(!glyph.status.success(), "glyph create should fail");
    let glyph_payload = parse_json_stdout(&glyph);
    assert_api_envelope(&glyph_payload, "glyph.create", false);
    assert!(
        glyph_payload["error"]["message"]
            .as_str()
            .expect("glyph error message")
            .contains("unsupported image type"),
        "glyph create should explain unsupported image type"
    );

    let grid = run_petiglyph(
        &project_dir,
        &[
            "grid",
            "create",
            "--input",
            unsupported.to_str().expect("path should be utf8"),
            "--rows",
            "2",
            "--cols",
            "2",
            "--json",
        ],
        None,
        None,
    );
    assert!(!grid.status.success(), "grid create should fail");
    let grid_payload = parse_json_stdout(&grid);
    assert_api_envelope(&grid_payload, "grid.create", false);
    assert!(
        grid_payload["error"]["message"]
            .as_str()
            .expect("grid error message")
            .contains("unsupported image type"),
        "grid create should explain unsupported image type"
    );

    let anim_standard = run_petiglyph(
        &project_dir,
        &[
            "animation",
            "create-standard",
            "--input",
            unsupported.to_str().expect("path should be utf8"),
            "--fps",
            "8",
            "--json",
        ],
        None,
        None,
    );
    assert!(
        !anim_standard.status.success(),
        "animation create-standard should fail"
    );
    let anim_standard_payload = parse_json_stdout(&anim_standard);
    assert_api_envelope(&anim_standard_payload, "animation.create-standard", false);
    assert!(
        anim_standard_payload["error"]["message"]
            .as_str()
            .expect("animation standard error message")
            .contains("animation import produced no frames"),
        "animation create-standard should explain empty frame import"
    );

    let anim_grid = run_petiglyph(
        &project_dir,
        &[
            "animation",
            "create-grid",
            "--input",
            unsupported.to_str().expect("path should be utf8"),
            "--fps",
            "8",
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
        !anim_grid.status.success(),
        "animation create-grid should fail"
    );
    let anim_grid_payload = parse_json_stdout(&anim_grid);
    assert_api_envelope(&anim_grid_payload, "animation.create-grid", false);
    assert!(
        anim_grid_payload["error"]["message"]
            .as_str()
            .expect("animation grid error message")
            .contains("animation import produced no frames"),
        "animation create-grid should explain empty frame import"
    );
}

#[test]
fn cli_doctor_repair_json_removes_stale_project_lock() {
    let workspace = make_temp_dir("doctor-repair-json");
    let home = workspace.join("home");
    fs::create_dir_all(&home).expect("home dir is created");
    let (project_dir, manifest_path) = create_project_with_icon(&workspace, "doctor-repair-demo");
    let project_lock = project_dir.join(".petiglyph-build.lock");
    make_stale_file(&project_lock);

    let output = run_petiglyph(
        &workspace,
        &[
            "doctor",
            "--repair",
            "--json",
            "--manifest",
            manifest_path.to_str().expect("manifest should be utf8"),
        ],
        Some(&home),
        None,
    );
    assert!(
        output.status.success(),
        "doctor --repair --json should succeed"
    );
    let payload = parse_json_stdout(&output);
    assert_api_envelope(&payload, "doctor", true);
    assert!(
        payload["data"]["repair"].as_bool() == Some(true),
        "repair flag should be true"
    );
    assert!(
        payload["data"]["repaired"].as_u64().unwrap_or(0) >= 1,
        "doctor repair should report at least one repaired item"
    );
    assert!(
        !project_lock.exists(),
        "stale project lock should be removed by doctor repair"
    );
}

#[test]
fn cli_create_non_interactive_without_no_launch_skips_tui() {
    let workspace = make_temp_dir("create-non-interactive");
    let home = workspace.join("home");
    fs::create_dir_all(&home).expect("home dir is created");
    let fake_path = fake_path_without_ffmpeg(&workspace);
    let output = run_petiglyph(
        &workspace,
        &["create", "create-no-launch-implicit"],
        Some(&home),
        Some(&fake_path),
    );
    assert!(
        output.status.success(),
        "create should succeed without --no-launch in non-tty contexts"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("non-interactive shell detected; skipping automatic TUI launch"),
        "create output should report non-interactive skip: {stdout}"
    );
    assert!(
        workspace
            .join("create-no-launch-implicit/petiglyph.toml")
            .exists(),
        "project manifest should be created"
    );
    let state_path = ffmpeg_prompt_state_path(&home);
    assert!(
        !state_path.exists(),
        "non-tty create should not write ffmpeg prompt state"
    );
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
    assert!(
        payload["data"]["manifest"]
            .as_str()
            .is_some_and(|value| same_path(Path::new(value), &manifest_path)),
        "autodetected manifest should point to nested project"
    );

    fs::remove_dir_all(project_dir).expect("project dir is removed");
    fs::remove_dir_all(workspace).expect("temp dir is removed");
}

#[test]
fn cli_nested_manifest_autodetection_fails_for_ambiguous_workspace() {
    let workspace = make_temp_dir("nested-autodetect-ambiguous");
    let (project_dir_one, _) = create_project_with_icon(&workspace, "demo-font-one");
    let (project_dir_two, _) = create_project_with_icon(&workspace, "demo-font-two");

    let output = run_petiglyph(&workspace, &["uninstall-font", "--json"], None, None);
    assert!(
        !output.status.success(),
        "uninstall-font --json should fail when multiple nested manifests exist"
    );

    let payload = parse_json_stdout(&output);
    assert_api_envelope(&payload, "uninstall-font", false);
    let message = payload["error"]["message"]
        .as_str()
        .expect("error message should be present");
    assert!(
        message.contains("multiple petiglyph projects detected")
            && message.contains("pass --manifest to choose one"),
        "ambiguous autodetection should provide manifest guidance: {message}"
    );

    fs::remove_dir_all(project_dir_one).expect("first project dir is removed");
    fs::remove_dir_all(project_dir_two).expect("second project dir is removed");
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

#[cfg(target_os = "linux")]
#[test]
fn cli_doctor_json_reports_manifest_discovery_failure() {
    let workspace = make_temp_dir("doctor-json-discovery-failure");
    let locked_dir = workspace.join("locked-cwd");
    let home = workspace.join("home");
    fs::create_dir_all(&locked_dir).expect("locked dir is created");
    fs::create_dir_all(&home).expect("home dir is created");

    let mut perms = fs::metadata(&locked_dir)
        .expect("locked dir metadata")
        .permissions();
    perms.set_mode(0o111);
    fs::set_permissions(&locked_dir, perms).expect("locked dir permissions are restricted");

    let output = run_petiglyph(&locked_dir, &["doctor", "--json"], Some(&home), None);

    let mut restore_perms = fs::metadata(&locked_dir)
        .expect("locked dir metadata for restore")
        .permissions();
    restore_perms.set_mode(0o755);
    fs::set_permissions(&locked_dir, restore_perms).expect("locked dir permissions are restored");

    assert!(
        output.status.success(),
        "doctor --json should still succeed"
    );
    let payload = parse_json_stdout(&output);
    assert_api_envelope(&payload, "doctor", true);
    let findings = payload["data"]["findings"]
        .as_array()
        .expect("findings array");
    assert!(
        findings.iter().any(|finding| {
            finding["code"].as_str() == Some("manifest_discovery_failed")
                && finding["severity"].as_str() == Some("warning")
                && finding["status"].as_str() == Some("issue")
        }),
        "doctor findings should report manifest discovery failures"
    );

    fs::remove_dir_all(workspace).expect("temp dir is removed");
}

#[test]
fn cli_tool_uninstall_json_is_idempotent() {
    let workspace = make_temp_dir("tool-uninstall-json");
    let home = workspace.join("home");
    fs::create_dir_all(&home).expect("home dir is created");

    let output = run_petiglyph(
        &workspace,
        &["uninstall-all-fonts", "--json"],
        Some(&home),
        None,
    );
    assert!(
        output.status.success(),
        "uninstall-all-fonts --json should succeed"
    );
    assert!(
        output.stderr.is_empty(),
        "uninstall-all-fonts --json should keep stderr clean on success"
    );

    let payload = parse_json_stdout(&output);
    assert_api_envelope(&payload, "uninstall-all-fonts", true);
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
        &["uninstall-all-fonts", "--json"],
        Some(&home),
        Some(&fake_path),
    );
    assert!(
        uninstall.status.success(),
        "uninstall-all-fonts --json should succeed"
    );
    assert!(
        uninstall.stderr.is_empty(),
        "uninstall-all-fonts --json should keep stderr clean on success"
    );

    let payload = parse_json_stdout(&uninstall);
    assert_api_envelope(&payload, "uninstall-all-fonts", true);
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

#[cfg(target_os = "macos")]
#[test]
fn cli_install_and_uninstall_json_lifecycle_is_idempotent_macos() {
    let workspace = make_temp_dir("install-lifecycle-macos");
    let (project_dir, _) = create_project_with_icon(&workspace, "demo-font");
    let home = workspace.join("home");
    fs::create_dir_all(&home).expect("home dir is created");

    let install_1 = run_petiglyph(&project_dir, &["install-font", "--json"], Some(&home), None);
    assert!(install_1.status.success(), "first install should succeed");
    let install_1_payload = parse_json_stdout(&install_1);
    assert_api_envelope(&install_1_payload, "install-font", true);
    assert_eq!(
        install_1_payload["data"]["platform"].as_str(),
        Some("macos")
    );
    assert_eq!(
        install_1_payload["data"]["replaced_previous_ttf_count"].as_u64(),
        Some(0)
    );

    let installed_ttf_1 = PathBuf::from(
        install_1_payload["data"]["installed_ttf"]
            .as_str()
            .expect("installed ttf"),
    );
    let expected_dir = home.join("Library").join("Fonts").join("petiglyph");
    assert!(
        installed_ttf_1.starts_with(&expected_dir)
            && installed_ttf_1
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("ttf")),
        "install should write a ttf under ~/Library/Fonts/petiglyph"
    );

    let install_2 = run_petiglyph(&project_dir, &["install-font", "--json"], Some(&home), None);
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
        None,
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
        None,
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

    let remaining_ttf_count = fs::read_dir(expected_dir)
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

#[cfg(target_os = "windows")]
#[test]
fn cli_install_and_uninstall_json_lifecycle_is_idempotent_windows() {
    let workspace = make_temp_dir("install-lifecycle-windows");
    let (project_dir, _) = create_project_with_icon(&workspace, "demo-font");
    let home = workspace.join("home");
    fs::create_dir_all(&home).expect("home dir is created");

    let install_1 = run_petiglyph(&project_dir, &["install-font", "--json"], Some(&home), None);
    assert!(install_1.status.success(), "first install should succeed");
    let install_1_payload = parse_json_stdout(&install_1);
    assert_api_envelope(&install_1_payload, "install-font", true);
    assert_eq!(
        install_1_payload["data"]["platform"].as_str(),
        Some("windows")
    );
    assert_eq!(
        install_1_payload["data"]["replaced_previous_ttf_count"].as_u64(),
        Some(0)
    );

    let installed_ttf_1 = PathBuf::from(
        install_1_payload["data"]["installed_ttf"]
            .as_str()
            .expect("installed ttf"),
    );
    let expected_dir = home
        .join("AppData")
        .join("Local")
        .join("Microsoft")
        .join("Windows")
        .join("Fonts")
        .join("petiglyph");
    assert!(
        installed_ttf_1.starts_with(&expected_dir)
            && installed_ttf_1
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("ttf")),
        "install should write a ttf under %LOCALAPPDATA%/Microsoft/Windows/Fonts/petiglyph"
    );

    let install_2 = run_petiglyph(&project_dir, &["install-font", "--json"], Some(&home), None);
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
        None,
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
        None,
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

    let remaining_ttf_count = fs::read_dir(expected_dir)
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
