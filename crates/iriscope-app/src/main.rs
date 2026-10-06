fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().any(|arg| matches!(arg.as_str(), "--version" | "-V")) {
        println!("IrisScope {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if std::env::args().any(|arg| arg == "--diagnose") {
        iriscope_app::run_diagnose();
        return Ok(());
    }
    iriscope_app::run_gui()
}
