//! LibreOffice: finding it, turning a .pptx into a PDF (once per file
//! content, cached), and installing it with one click.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const DOWNLOAD_PAGE: &str = "https://www.libreoffice.org/download/download-libreoffice/";

fn cache_root() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("lantern")
}

fn on_path(bin: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|d| d.join(bin))
        .find(|p| p.is_file())
}

/// The LibreOffice program, if installed.
pub fn find() -> Option<PathBuf> {
    let mut c: Vec<PathBuf> = Vec::new();
    if cfg!(target_os = "macos") {
        c.push("/Applications/LibreOffice.app/Contents/MacOS/soffice".into());
        if let Some(h) = dirs::home_dir() {
            c.push(h.join("Applications/LibreOffice.app/Contents/MacOS/soffice"));
        }
    } else if cfg!(windows) {
        for var in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"] {
            if let Some(p) = std::env::var_os(var) {
                c.push(
                    PathBuf::from(p)
                        .join("LibreOffice")
                        .join("program")
                        .join("soffice.exe"),
                );
            }
        }
        if let Some(p) = on_path("soffice.exe") {
            c.push(p);
        }
    } else {
        for b in [
            "soffice",
            "libreoffice",
            "libreoffice-fresh",
            "libreoffice-still",
        ] {
            if let Some(p) = on_path(b) {
                c.push(p);
            }
        }
        if let Ok(rd) = std::fs::read_dir("/opt") {
            for e in rd.flatten() {
                c.push(e.path().join("program/soffice"));
            }
        }
    }
    c.into_iter().find(|p| p.is_file())
}

/// `file:///...` URL of a path (for LibreOffice's -env: option).
fn file_url(p: &Path) -> String {
    let s = p.display().to_string().replace('\\', "/");
    let mut out = String::from("file://");
    if !s.starts_with('/') {
        out.push('/');
    }
    for ch in s.chars() {
        match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '/' | '-' | '_' | '.' | '~' | ':' => out.push(ch),
            c => {
                let mut b = [0u8; 4];
                for byte in c.encode_utf8(&mut b).bytes() {
                    out.push_str(&format!("%{byte:02X}"));
                }
            }
        }
    }
    out
}

pub fn hide_window(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let _ = cmd;
}

/// Short fingerprint of a file's content.
pub fn fingerprint(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let h = Sha256::digest(&bytes);
    Ok(h.iter().take(10).map(|b| format!("{b:02x}")).collect())
}

/// The PDF of a presentation: from the cache, else made by LibreOffice.
/// `status` tells what is going on.
pub fn to_pdf(pptx: &Path, status: &dyn Fn(String)) -> Result<PathBuf, String> {
    let key = fingerprint(pptx)?;
    let dir = cache_root().join("decks").join(&key);
    let pdf = dir.join("deck.pdf");
    if pdf.is_file() {
        return Ok(pdf);
    }
    let soffice = find().ok_or("LibreOffice ist nicht installiert.")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let ext = pptx
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("pptx")
        .to_lowercase();
    let input = dir.join(format!("deck.{ext}"));
    std::fs::copy(pptx, &input).map_err(|e| format!("{}: {e}", pptx.display()))?;
    let profile = file_url(&cache_root().join("libreoffice-profile"));
    // Full-quality pictures; older LibreOffice versions don't know the
    // options and get a second try without them.
    let filters = [
        r#"pdf:impress_pdf_Export:{"UseLosslessCompression":{"type":"boolean","value":"true"},"ReduceImageResolution":{"type":"boolean","value":"false"},"ExportNotesPages":{"type":"boolean","value":"false"}}"#,
        "pdf",
    ];
    let mut last = String::new();
    for filter in filters {
        status("LibreOffice wandelt die Folien um…".into());
        let mut cmd = Command::new(&soffice);
        cmd.arg(format!("-env:UserInstallation={profile}"))
            .args([
                "--headless",
                "--norestore",
                "--nolockcheck",
                "--nodefault",
                "--convert-to",
                filter,
                "--outdir",
            ])
            .arg(&dir)
            .arg(&input)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        hide_window(&mut cmd);
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("LibreOffice ließ sich nicht starten: {e}"))?;
        let start = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if start.elapsed() > Duration::from_secs(300) => {
                    let _ = child.kill();
                    return Err("LibreOffice hat nach 5 Minuten nicht fertig umgewandelt.".into());
                }
                Ok(None) => {
                    status(format!(
                        "LibreOffice wandelt die Folien um… {} s",
                        start.elapsed().as_secs()
                    ));
                    std::thread::sleep(Duration::from_millis(250));
                }
                Err(e) => return Err(e.to_string()),
            }
        }
        if let Ok(out) = child.wait_with_output() {
            last = String::from_utf8_lossy(&out.stderr).trim().to_string();
        }
        if pdf.is_file() {
            let _ = std::fs::remove_file(&input);
            return Ok(pdf);
        }
    }
    Err(if last.is_empty() {
        "LibreOffice konnte die Präsentation nicht umwandeln.".into()
    } else {
        format!("LibreOffice konnte die Präsentation nicht umwandeln: {last}")
    })
}

/// Progress of the LibreOffice installation.
#[derive(Clone, Debug, Default)]
pub struct Install {
    pub text: String,
    pub progress: Option<f32>,
    pub running: bool,
    pub error: Option<String>,
}

pub type InstallState = Arc<Mutex<Install>>;

fn set(state: &InstallState, text: &str, progress: Option<f32>) {
    if let Ok(mut s) = state.lock() {
        s.text = text.to_string();
        s.progress = progress;
    }
}

/// Open a web page in the browser.
pub fn open_url(url: &str) {
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(url);
        c
    } else if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/C", "start", "", url]);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(url);
        c
    };
    hide_window(&mut cmd);
    let _ = cmd.spawn();
}

/// Install LibreOffice in the background (macOS: download and copy to
/// Applications; Windows: winget; Linux: the package manager in a terminal).
pub fn install(state: InstallState) {
    if let Ok(mut s) = state.lock() {
        *s = Install {
            text: "Starte…".into(),
            running: true,
            ..Default::default()
        };
    }
    std::thread::spawn(move || {
        let r = if cfg!(target_os = "macos") {
            install_mac(&state)
        } else if cfg!(windows) {
            install_windows(&state)
        } else {
            install_linux(&state)
        };
        if let Ok(mut s) = state.lock() {
            s.running = false;
            s.progress = None;
            match r {
                Ok(t) => s.text = t,
                Err(e) => s.error = Some(e),
            }
        }
    });
}

fn install_windows(state: &InstallState) -> Result<String, String> {
    set(
        state,
        "Installiere LibreOffice über winget (Windows fragt eventuell nach Erlaubnis)…",
        None,
    );
    let mut cmd = Command::new("winget");
    cmd.args([
        "install",
        "-e",
        "--id",
        "TheDocumentFoundation.LibreOffice",
        "--silent",
        "--accept-package-agreements",
        "--accept-source-agreements",
    ]);
    hide_window(&mut cmd);
    match cmd.status() {
        Ok(_) if find().is_some() => Ok("LibreOffice ist installiert.".into()),
        Ok(s) => Err(format!(
            "winget konnte LibreOffice nicht installieren ({s}). Die Download-Seite ist geöffnet."
        ))
        .inspect_err(|_| open_url(DOWNLOAD_PAGE)),
        Err(_) => {
            open_url(DOWNLOAD_PAGE);
            Err(
                "winget fehlt auf diesem Windows. Die Download-Seite von LibreOffice ist geöffnet."
                    .into(),
            )
        }
    }
}

/// Newest LibreOffice version on the download server.
fn latest_version() -> Result<String, String> {
    let page = ureq::get("https://download.documentfoundation.org/libreoffice/stable/")
        .call()
        .map_err(|e| format!("Download-Server nicht erreichbar: {e}"))?
        .into_string()
        .map_err(|e| e.to_string())?;
    newest_in_listing(&page)
        .ok_or_else(|| "Keine LibreOffice-Version auf dem Server gefunden.".into())
}

pub fn newest_in_listing(page: &str) -> Option<String> {
    let mut best: Option<(Vec<u32>, String)> = None;
    for part in page.split("href=\"").skip(1) {
        let v = part.split('/').next().unwrap_or_default();
        let nums: Option<Vec<u32>> = v.split('.').map(|x| x.parse().ok()).collect();
        if let Some(nums) = nums.filter(|n| n.len() >= 3) {
            if best.as_ref().is_none_or(|(b, _)| nums > *b) {
                best = Some((nums, v.to_string()));
            }
        }
    }
    best.map(|(_, v)| v)
}

fn install_mac(state: &InstallState) -> Result<String, String> {
    set(state, "Suche die neueste LibreOffice-Version…", None);
    let v = latest_version()?;
    let (dir, file) = if cfg!(target_arch = "aarch64") {
        ("aarch64", "aarch64")
    } else {
        ("x86_64", "x86-64")
    };
    let url = format!("https://download.documentfoundation.org/libreoffice/stable/{v}/mac/{dir}/LibreOffice_{v}_MacOS_{file}.dmg");
    let resp = ureq::get(&url)
        .call()
        .map_err(|e| format!("Download fehlgeschlagen: {e}"))?;
    let total: u64 = resp
        .header("Content-Length")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let tmp = std::env::temp_dir().join(format!("LibreOffice-{v}.dmg"));
    let mut out = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    let mut reader = resp.into_reader();
    let mut buf = vec![0u8; 1 << 16];
    let mut done: u64 = 0;
    let mut last = Instant::now();
    loop {
        let n = std::io::Read::read(&mut reader, &mut buf)
            .map_err(|e| format!("Download abgebrochen: {e}"))?;
        if n == 0 {
            break;
        }
        std::io::Write::write_all(&mut out, &buf[..n]).map_err(|e| e.to_string())?;
        done += n as u64;
        if last.elapsed() > Duration::from_millis(200) {
            last = Instant::now();
            let p = (total > 0).then(|| done as f32 / total as f32);
            set(
                state,
                &format!("Lade LibreOffice {v}… {} MB", done / 1_000_000),
                p,
            );
        }
    }
    drop(out);
    set(state, "Kopiere LibreOffice in Programme…", None);
    let mount = std::env::temp_dir().join("lantern-libreoffice-dmg");
    let _ = Command::new("hdiutil")
        .arg("detach")
        .arg(&mount)
        .arg("-force")
        .output();
    let st = Command::new("hdiutil")
        .args([
            "attach",
            "-nobrowse",
            "-readonly",
            "-noautoopen",
            "-mountpoint",
        ])
        .arg(&mount)
        .arg(&tmp)
        .output()
        .map_err(|e| e.to_string())?;
    if !st.status.success() {
        return Err(format!(
            "Disk-Image ließ sich nicht öffnen: {}",
            String::from_utf8_lossy(&st.stderr).trim()
        ));
    }
    let app = mount.join("LibreOffice.app");
    let mut target = PathBuf::from("/Applications/LibreOffice.app");
    let mut ok = Command::new("ditto")
        .arg(&app)
        .arg(&target)
        .status()
        .is_ok_and(|s| s.success());
    if !ok {
        if let Some(h) = dirs::home_dir() {
            let _ = std::fs::create_dir_all(h.join("Applications"));
            target = h.join("Applications/LibreOffice.app");
            ok = Command::new("ditto")
                .arg(&app)
                .arg(&target)
                .status()
                .is_ok_and(|s| s.success());
        }
    }
    let _ = Command::new("hdiutil")
        .arg("detach")
        .arg(&mount)
        .arg("-force")
        .output();
    let _ = std::fs::remove_file(&tmp);
    if ok && find().is_some() {
        Ok(format!("LibreOffice {v} ist installiert."))
    } else {
        Err("LibreOffice konnte nicht nach Programme kopiert werden.".into())
    }
}

/// The package manager command for LibreOffice on this Linux.
pub fn linux_command() -> Option<&'static str> {
    if on_path("pacman").is_some() {
        Some("sudo pacman -S --needed libreoffice-fresh")
    } else if on_path("apt-get").is_some() {
        Some("sudo apt-get install -y libreoffice-impress")
    } else if on_path("dnf").is_some() {
        Some("sudo dnf install -y libreoffice-impress")
    } else if on_path("zypper").is_some() {
        Some("sudo zypper install -y libreoffice-impress")
    } else {
        None
    }
}

fn install_linux(state: &InstallState) -> Result<String, String> {
    let Some(cmd) = linux_command() else {
        open_url(DOWNLOAD_PAGE);
        return Err("Unbekannte Paketverwaltung – die Download-Seite ist geöffnet.".into());
    };
    crate::terminal::open("LibreOffice installieren", cmd)?;
    set(state, "Im Terminal: Passwort eingeben und bestätigen. Lantern merkt es, sobald LibreOffice da ist.", None);
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(1800) {
        if find().is_some() {
            return Ok("LibreOffice ist installiert.".into());
        }
        std::thread::sleep(Duration::from_secs(2));
    }
    Err("LibreOffice wurde nicht gefunden.".into())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn file_urls() {
        assert_eq!(file_url(Path::new("/a b/c")), "file:///a%20b/c");
        assert_eq!(file_url(Path::new("C:\\Users\\x")), "file:///C:/Users/x");
    }

    #[test]
    fn picks_the_newest_version() {
        let page = r#"<a href="../">..</a><a href="24.8.7/">24.8.7/</a><a href="25.2.3/">25.2.3/</a><a href="25.2.10/">x</a>"#;
        assert_eq!(newest_in_listing(page).as_deref(), Some("25.2.10"));
    }

    #[test]
    fn converts_with_real_libreoffice() {
        if find().is_none() {
            return;
        }
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/demo.pptx");
        let pdf = to_pdf(&p, &|_| {}).unwrap();
        assert!(std::fs::metadata(pdf).unwrap().len() > 1000);
    }
}
