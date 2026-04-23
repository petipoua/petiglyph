use image::{Rgba, RgbaImage};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

fn make_temp_dir(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time is valid")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("petiglyph-cli-{name}-{nonce}"));
    fs::create_dir_all(&dir).expect("temp dir is created");
    dir
}

fn write_test_png(path: &Path) {
    let mut img = RgbaImage::from_pixel(8, 8, Rgba([255, 255, 255, 0]));
    img.put_pixel(2, 2, Rgba([0, 0, 0, 255]));
    img.put_pixel(5, 5, Rgba([0, 0, 0, 255]));
    img.save(path).expect("test image should be written");
}

fn run_petiglyph(cwd: &Path, args: &[&str], home_override: Option<&Path>) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_petiglyph"));
    cmd.current_dir(cwd).args(args);
    if let Some(home) = home_override {
        cmd.env("HOME", home);
    }
    cmd.output().expect("petiglyph command should run")
}

#[test]
fn cli_no_subcommand_errors_without_manifest() {
    let workspace = make_temp_dir("no-manifest");

    let output = run_petiglyph(&workspace, &[], None);

    assert!(!output.status.success(), "command should fail without manifest");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("no petiglyph project found"),
        "stderr should mention missing project manifest: {stderr}"
    );

    fs::remove_dir_all(workspace).expect("temp dir is removed");
}

#[test]
fn cli_create_build_sample_and_install_font_workflow() {
    let workspace = make_temp_dir("workflow");
    let project_name = "demo-font";
    let project_dir = workspace.join(project_name);

    let create = run_petiglyph(&workspace, &["create", project_name, "--no-launch"], None);
    assert!(create.status.success(), "create command should succeed");

    let manifest_path = project_dir.join("petiglyph.toml");
    let icons_dir = project_dir.join("icons");
    write_test_png(&icons_dir.join("alpha.png"));

    let build = run_petiglyph(&project_dir, &["build"], None);
    assert!(build.status.success(), "build command should succeed");
    assert!(project_dir.join("build/glyph-map.json").exists());
    assert!(project_dir.join("build/glyph-sample.txt").exists());
    assert!(project_dir.join("build/previews/alpha.png").exists());

    let sample = run_petiglyph(&project_dir, &["sample"], None);
    assert!(sample.status.success(), "sample command should succeed");
    let sample_stdout = String::from_utf8_lossy(&sample.stdout);
    assert!(
        sample_stdout.contains("petiglyph sample"),
        "sample output should include header: {sample_stdout}"
    );

    if Command::new("fc-cache").arg("--version").output().is_ok() {
        let home = workspace.join("home");
        fs::create_dir_all(&home).expect("home dir is created");

        let install = run_petiglyph(
            &project_dir,
            &["install-font", "--manifest", manifest_path.to_str().expect("utf8 path")],
            Some(&home),
        );
        assert!(install.status.success(), "install-font command should succeed");

        let installed_dir = home
            .join(".local")
            .join("share")
            .join("fonts")
            .join("petiglyph")
            .join("demo_font");
        let installed_ttfs = fs::read_dir(&installed_dir)
            .expect("installed font directory should exist")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("ttf"))
            .collect::<Vec<_>>();
        assert_eq!(
            installed_ttfs.len(),
            1,
            "install directory should contain exactly one ttf"
        );
    }

    fs::remove_dir_all(workspace).expect("temp dir is removed");
}
