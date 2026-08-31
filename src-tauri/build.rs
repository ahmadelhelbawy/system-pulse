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
    tauri_build::build()
}
