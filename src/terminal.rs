//! Opening a terminal window that runs a command (Linux and macOS).

use std::process::Command;

fn q(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// The script the terminal runs: title, the command, and the window stays open.
fn script(title: &str, command: &str) -> String {
    format!(
        "#!/bin/bash\nprintf '\\033[1m%s\\033[0m\\n\\n' {}\n{}\nprintf '\\nFertig. Enter schließt dieses Fenster. '\nread -r _\n",
        q(title),
        command
    )
}

/// Linux terminals and how to make them run a script.
const LINUX_TERMINALS: &[(&str, &[&str])] = &[
    ("kitty", &[]),
    ("alacritty", &["-e"]),
    ("foot", &[]),
    ("wezterm", &["start", "--"]),
    ("ghostty", &["-e"]),
    ("konsole", &["-e"]),
    ("gnome-terminal", &["--"]),
    ("kgx", &["--"]),
    ("xfce4-terminal", &["-x"]),
    ("tilix", &["-e"]),
    ("terminator", &["-x"]),
    ("x-terminal-emulator", &["-e"]),
    ("xterm", &["-e"]),
];

fn on_path(bin: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(bin).is_file()))
}

/// Open a terminal window running `steps`.
pub fn open(title: &str, command: &str) -> Result<(), String> {
    let file = std::env::temp_dir().join(format!(
        "lantern-{}-{}.sh",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    ));
    std::fs::write(&file, script(title, command)).map_err(|e| e.to_string())?;
    let path = file.display().to_string();
    if cfg!(target_os = "macos") {
        let cmd = format!("bash {}", q(&path));
        let apple = format!(
            "tell application \"Terminal\"\nactivate\ndo script \"{}\"\nend tell",
            cmd.replace('\\', "\\\\").replace('"', "\\\"")
        );
        return Command::new("osascript")
            .args(["-e", &apple])
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("Could not open Terminal: {e}"));
    }
    // $TERMINAL first (the user's choice), then the usual ones.
    if let Ok(t) = std::env::var("TERMINAL") {
        let t = t.trim().to_string();
        if !t.is_empty() && on_path(&t) {
            let flag = LINUX_TERMINALS
                .iter()
                .find(|(n, _)| *n == t)
                .map_or(&["-e"][..], |(_, f)| f);
            return Command::new(&t)
                .args(flag)
                .args(["bash", &path])
                .spawn()
                .map(|_| ())
                .map_err(|e| format!("Could not open {t}: {e}"));
        }
    }
    for (bin, flag) in LINUX_TERMINALS {
        if on_path(bin) {
            return Command::new(bin)
                .args(*flag)
                .args(["bash", &path])
                .spawn()
                .map(|_| ())
                .map_err(|e| format!("Could not open {bin}: {e}"));
        }
    }
    Err("No terminal found (e.g. kitty, alacritty, konsole, gnome-terminal). Set the TERMINAL environment variable to choose one.".into())
}
