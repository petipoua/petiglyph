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
fn cli_no_subcommand_errors_without_manifest() {
    let workspace = make_temp_dir("no-manifest");

    let output = run_petiglyph(&workspace, &[], None, None);

    assert!(
        !output.status.success(),
        "command should fail without manifest"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("interactive TUI welcome requires a terminal"),
        "stderr should mention interactive welcome requirement in non-tty runs: {stderr}"
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

    let sample = run_petiglyph(&project_dir, &["sample"], None, None);
    assert!(sample.status.success(), "sample command should succeed");
    let sample_stdout = String::from_utf8_lossy(&sample.stdout);
    assert!(
        sample_stdout.contains("petiglyph sample"),
        "sample output should include header: {sample_stdout}"
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
    assert!(
        install_1_payload["data"]["installed_ttf"]
            .as_str()
            .expect("installed ttf")
            .ends_with("/.local/share/fonts/petiglyph/demo_font_demo_font.ttf"),
        "CLI install should use project-prefixed effective font name"
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
        Some(1),
        "second install should replace exactly one prior ttf"
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

    fs::remove_dir_all(workspace).expect("temp dir is removed");
}
