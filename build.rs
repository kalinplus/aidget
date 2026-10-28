use std::process::Command;

/// What this build is, for the updater's "only releases replace themselves"
/// rule.
///
/// A working tree reports `git describe`, so `0.1.0-5-gabc1234` and
/// `0.1.0-dirty` are recognisable as builds from source. A release is checked
/// out at a clean tag, and CI checkouts are shallow with no tags at all, so
/// anything that is not a describe stamp falls back to the crate version, which
/// the release workflow injects.
fn build_stamp() -> String {
    let base = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    described()
        .filter(|stamp| stamp.contains("-g") || stamp.ends_with("-dirty"))
        .unwrap_or(base)
}

fn described() -> Option<String> {
    let output = Command::new("git")
        .args(["describe", "--tags", "--always", "--dirty"])
        .current_dir(std::env::var("CARGO_MANIFEST_DIR").ok()?)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stamp = String::from_utf8(output.stdout).ok()?.trim().to_string();
    (!stamp.is_empty()).then_some(stamp)
}

/// Cross-target type-check hook.
///
/// `AIDGET_CHECK_TARGET=1` skips `tauri_build`, whose Windows resource step
/// needs `llvm-rc` and whose Linux step needs GTK dev packages, so a host can
/// type-check another platform's `cfg` branches with `cargo check --lib`. Real
/// builds never set it and stay strict.
fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    println!("cargo:rerun-if-env-changed=AIDGET_CHECK_TARGET");
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rustc-env=AIDGET_BUILD_STAMP={}", build_stamp());
    if std::env::var_os("AIDGET_CHECK_TARGET").is_some() {
        return;
    }
    println!("cargo:rerun-if-changed=ui/src");
    println!("cargo:rerun-if-changed=ui/assets");
    println!("cargo:rerun-if-changed=ui/widget.html");
    println!("cargo:rerun-if-changed=ui/main.html");
    println!("cargo:rerun-if-changed=ui/vite.config.ts");

    // The UI is embedded via include_dir, so every rebuild must refresh
    // ui/dist. `.taurignore` keeps those writes from retriggering the watcher.
    // On Windows `pnpm` is a `.cmd` batch shim, which `CreateProcess` will not
    // resolve from a bare program name.
    #[cfg(windows)]
    let mut command = {
        let mut command = Command::new("cmd");
        command.args(["/C", "pnpm"]);
        command
    };
    #[cfg(not(windows))]
    let mut command = Command::new("pnpm");
    let status = command
        .args(["run", "build:ui"])
        .current_dir(&manifest)
        .status()
        .expect("failed to run `pnpm run build:ui` for the UI");
    assert!(status.success(), "pnpm run build:ui failed");
    tauri_build::build()
}
