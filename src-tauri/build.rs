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
    }
    tauri_build::build()
}
