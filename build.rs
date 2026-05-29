// Composes the version string baked into the binary and exposes it as
// GOVEE2MQTT_VERSION for src/version_info.rs to read.
//
// Driven by CI env vars (raw facts in, composed version out) so the value is
// correct inside the Docker build context, which has no .git to read:
//   - GOVEE2MQTT_RELEASE_TAG set (release.yml passes the v* tag without the v):
//     embedded verbatim, e.g. 1.2.3
//   - GOVEE2MQTT_BUILD_SHA set (edge/CI builds): composed as
//     <CARGO_PKG_VERSION>-edge.<sha>, so the version derives from Cargo.toml
//     (the single source of truth) and is distinguishable from a release of the
//     same x.y.z
//   - neither (local dev builds): GOVEE2MQTT_VERSION is left empty, and version_info
//     falls back to the bare CARGO_PKG_VERSION
fn main() {
    println!("cargo:rerun-if-env-changed=GOVEE2MQTT_RELEASE_TAG");
    println!("cargo:rerun-if-env-changed=GOVEE2MQTT_BUILD_SHA");

    // rust-embed needs ui/dist to exist at compile time even if it's empty, so
    // `cargo clippy` / `cargo test` work without first running `pnpm build`.
    // Release builds populate it before this runs (Dockerfile pnpm build step).
    std::fs::create_dir_all("ui/dist").expect("create ui/dist");

    let version = std::env::var("GOVEE2MQTT_RELEASE_TAG")
        .ok()
        .filter(|t| !t.trim().is_empty())
        .map(|t| t.trim().to_string())
        .or_else(|| {
            let sha = std::env::var("GOVEE2MQTT_BUILD_SHA").ok()?;
            let sha = sha.trim();
            if sha.is_empty() {
                return None;
            }
            Some(format!("{}-edge.{sha}", env!("CARGO_PKG_VERSION")))
        })
        .unwrap_or_default();

    println!("cargo:rustc-env=GOVEE2MQTT_VERSION={version}");
}
