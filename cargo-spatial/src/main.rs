use std::process::Command;

fn main() {
    Command::new("cargo")
        .arg("install")
        .arg("--path")
        .arg("./workers")
        .arg("--bin")
        .arg("server")
        .arg("--root")
        .arg("./build/debug")
        .arg("--debug")
        .arg("--force")
        .status()
        .expect("Failed to build worker bin");

    Command::new("spatial")
        .arg("local")
        .arg("launch")
        .arg("--launch_config=deployment.json")
        .status()
        .expect("Failed to run spatial");
}
