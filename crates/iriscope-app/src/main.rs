fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().any(|arg| matches!(arg.as_str(), "--version" | "-V")) {
        println!("IrisScope {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if std::env::args().any(|arg| arg == "--diagnose") {
        iriscope_app::run_diagnose();
        return Ok(());
    }
    let args: Vec<_> = std::env::args().collect();
    if let Some(index) = args
        .iter()
        .position(|arg| matches!(arg.as_str(), "--validate-hardware" | "--validate-button"))
    {
        let directory = args
            .get(index + 1)
            .ok_or("Specify an empty validation directory after the validation option")?;
        return iriscope_app::run_hardware_validation(
            std::path::Path::new(directory),
            args[index] == "--validate-button",
        );
    }
    iriscope_app::run_gui()
}
