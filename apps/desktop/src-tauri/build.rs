fn main() {
    // Force rebuild to pick up latest frontend dist
    println!("cargo:rerun-if-changed=../dist/index.html");
    tauri_build::build()
}
