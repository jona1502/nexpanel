fn main() {
    slint_build::compile("ui/main.slint").expect("Failed to compile NexPanel UI");
    // Manifest: PerMonitorV2 DPI, Common Controls v6, supportedOS, asInvoker. Linked into the
    // binaries only, so test executables are unaffected.
    if std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows") {
        embed_resource::compile("nexpanel.rc", embed_resource::NONE)
            .manifest_required()
            .expect("Failed to embed NexPanel manifest");
    }
}
