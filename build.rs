fn main() {
    let blp_files = [
        "src/ui/window.blp",
        "src/ui/graph/node.blp",
        "src/ui/graph/port.blp",
        "src/ui/graph/zoomentry.blp",
    ];

    println!("cargo:warning=Helvum build script starting...");

    for blp in blp_files {
        let ui = blp.replace(".blp", ".ui");
        println!("cargo:rerun-if-changed={}", blp);

        let output = std::process::Command::new("blueprint-compiler")
            .arg("compile")
            .arg("--output")
            .arg(&ui)
            .arg(blp)
            .output();

        match output {
            Ok(output) => {
                if !output.status.success() {
                    let err = String::from_utf8_lossy(&output.stderr);
                    panic!("Failed to compile blueprint {}: {}", blp, err);
                }
            }
            Err(e) => {
                // If compiler is missing, we only fail if the .ui file doesn't exist
                if !std::path::Path::new(&ui).exists() {
                    panic!("blueprint-compiler not found and {} is missing: {}", ui, e);
                }
            }
        }
    }
}
