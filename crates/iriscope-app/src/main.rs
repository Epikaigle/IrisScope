fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().any(|arg| arg == "--diagnose") {
        iriscope_app::run_diagnose();
        return Ok(());
    }
    iriscope_app::run_gui()
}
