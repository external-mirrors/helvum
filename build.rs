fn main() {
    let blp_files = [
        "src/ui/window.blp",
        "src/ui/graph/node.blp",
        "src/ui/graph/port.blp",
        "src/ui/graph/zoomentry.blp",
    ];

    println!("cargo:warning=Helvum build script starting...");

    let root = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let out_dir = std::env::var("OUT_DIR").unwrap();

    for blp in blp_files {
        let blp_abs = std::path::Path::new(&root).join(blp);
        let ui_abs = blp_abs.with_extension("ui");

        println!("cargo:rerun-if-changed={}", blp_abs.display());

        let output = std::process::Command::new("blueprint-compiler")
            .arg("compile")
            .arg("--output")
            .arg(&ui_abs)
            .arg(&blp_abs)
            .output()
            .expect("Failed to run blueprint-compiler");

        if !output.status.success() {
            panic!(
                "Failed to compile blueprint {}: {}",
                blp,
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    let gresource_xml = std::path::Path::new(&root).join("src/ui/helvum.gresource.xml");
    println!("cargo:rerun-if-changed={}", gresource_xml.display());

    let output = std::process::Command::new("glib-compile-resources")
        .arg("--target")
        .arg(std::path::Path::new(&out_dir).join("helvum.gresource"))
        .arg("--sourcedir")
        .arg(std::path::Path::new(&root).join("src/ui"))
        .arg(&gresource_xml)
        .output()
        .expect("Failed to run glib-compile-resources");

    if !output.status.success() {
        panic!(
            "Failed to compile gresource: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
