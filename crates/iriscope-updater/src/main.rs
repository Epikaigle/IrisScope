fn main() {
    let args: Vec<_> = std::env::args_os().collect();
    if args.get(1).is_some_and(|arg| arg == "--version") {
        println!("IrisScope {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if args.len() == 3 && args[1] == "--check" {
        let result = (|| -> iriscope_updater::Result<()> {
            let executable = std::path::Path::new(&args[2]);
            let target = iriscope_updater::InstallTarget::from_executable(executable)?;
            let output = std::process::Command::new(executable)
                .arg("--version")
                .output()?;
            let version = String::from_utf8(output.stdout)?;
            let version = version
                .trim()
                .strip_prefix("IrisScope ")
                .ok_or("Version installée inconnue")?;
            let release = iriscope_updater::check(
                version,
                &target,
                &std::sync::atomic::AtomicBool::new(false),
            )?;
            if let Some(release) = release {
                println!("Version signée disponible : {}", release.version);
            } else {
                println!("IrisScope est à jour ; vérification réussie.");
            }
            Ok(())
        })();
        if let Err(error) = result {
            eprintln!("Vérification échouée : {error}");
            std::process::exit(1);
        }
        return;
    }
    if args.len() == 6 && args[1] == "--install-local" {
        let result = iriscope_updater::install_local_signed(
            std::path::Path::new(&args[2]),
            std::path::Path::new(&args[3]),
            std::path::Path::new(&args[4]),
            std::path::Path::new(&args[5]),
        );
        if let Err(error) = result {
            eprintln!("Mise à jour locale refusée : {error}");
            std::process::exit(1);
        }
        println!(
            "Installation signée préparée ; remplacement après fermeture de l’ancienne version."
        );
        return;
    }
    if args.len() != 3 || args[1] != "--apply" {
        eprintln!("Usage: iriscope-updater --apply plan.json");
        std::process::exit(2);
    }
    if let Err(error) = iriscope_updater::apply_plan(std::path::Path::new(&args[2])) {
        eprintln!("Mise à jour interrompue : {error}");
        #[cfg(target_os = "macos")]
        let _ = std::process::Command::new("/usr/bin/osascript")
            .args(["-e", "on run argv\ndisplay alert \"Mise à jour IrisScope\" message (item 1 of argv)\nend run"])
            .arg(error.to_string()).status();
        #[cfg(target_os = "linux")]
        let _ = std::process::Command::new("zenity")
            .args(["--error", "--title=Mise à jour IrisScope", "--text"])
            .arg(error.to_string())
            .status();
        std::process::exit(1);
    }
}
