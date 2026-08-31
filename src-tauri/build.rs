fn main() {
    // TEMPORARY diagnostic for the CI-only "RC.EXE" build failure (see
    // .github/workflows/release.yml) — three prior fixes (env var, PATH,
    // step-output binding) all confirmed rc.exe's real path independently
    // yet the panic persisted, so this proves directly, from inside the
    // actual failing process, what this build script's own environment
    // looks like. Remove once root-caused.
    eprintln!("DIAG: RC = {:?}", std::env::var("RC"));
    eprintln!("DIAG: TARGET = {:?}", std::env::var("TARGET"));
    eprintln!("DIAG: PATH = {:?}", std::env::var("PATH"));
    if let Ok(rc) = std::env::var("RC") {
        eprintln!("DIAG: exists() = {:?}", std::path::Path::new(&rc).exists());
        let direct = std::process::Command::new(&rc).arg("/?").status();
        eprintln!("DIAG: direct Command::new(RC).status() = {:?}", direct);
        let bare = std::process::Command::new("rc.exe").arg("/?").status();
        eprintln!(
            "DIAG: bare Command::new(\"rc.exe\").status() (PATH lookup) = {:?}",
            bare
        );

        // Replicate embed-resource's exact invocation shape: `/fo <out>.lib
        // /I <out_dir> <resource.rc>` against a real, minimal .rc file in
        // OUT_DIR, to see whether it's specifically these extra arguments
        // (not the executable path) that break spawning.
        let out_dir = std::env::var("OUT_DIR").unwrap_or_else(|_| ".".to_string());
        let fake_rc = std::path::Path::new(&out_dir).join("diag_test.rc");
        let fake_lib = std::path::Path::new(&out_dir).join("diag_test.lib");
        let _ = std::fs::write(&fake_rc, "// empty\n");
        let replicated = std::process::Command::new(&rc)
            .args(["/fo", fake_lib.to_str().unwrap(), "/I", &out_dir])
            .arg(&fake_rc)
            .status();
        eprintln!(
            "DIAG: replicated embed-resource invocation = {:?}",
            replicated
        );
    }
    tauri_build::build()
}
