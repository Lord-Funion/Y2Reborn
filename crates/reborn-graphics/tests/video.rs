use std::{env, fs, path::PathBuf, process::Command};

#[test]
fn video_texture_renders_spatial_detail_and_updates_on_a_real_gles_context() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let output = env::temp_dir().join(format!("reborn-video-gles-{}", std::process::id()));
    fs::create_dir_all(&output).unwrap();
    let flags = Command::new("pkg-config")
        .args(["--cflags", "--libs", "libdrm", "gbm", "egl", "glesv2"])
        .output()
        .unwrap();
    assert!(flags.status.success());
    let binary = output.join("test");
    let compiled = Command::new("cc")
        .args([
            "-std=c11",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-fsanitize=undefined",
        ])
        .arg(root.join("tests/video.c"))
        .args(String::from_utf8(flags.stdout).unwrap().split_whitespace())
        .arg("-o")
        .arg(&binary)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    assert!(Command::new(binary)
        .env("LIBGL_ALWAYS_SOFTWARE", "1")
        .status()
        .unwrap()
        .success());
    fs::remove_dir_all(output).unwrap();
}
