fn main() {
    #[cfg(windows)]
    build_windows_icon();
}

#[cfg(windows)]
fn build_windows_icon() {
    println!("cargo:rerun-if-changed=assets/reader.rc");
    println!("cargo:rerun-if-changed=assets/reader.ico");
    embed_resource::compile("assets/reader.rc", embed_resource::NONE)
        .manifest_required()
        .expect("compile the Windows application icon");
}
