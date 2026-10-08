fn main() {
    // Use the same widget metrics on every desktop. Platform overlay scrollbars
    // and native field styles otherwise change the available space on macOS.
    slint_build::compile_with_config(
        "../../ui/main.slint",
        slint_build::CompilerConfiguration::new().with_style("fluent".into()),
    )
    .expect("compile the IrisScope interface");
}
