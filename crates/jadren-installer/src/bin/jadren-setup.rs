use std::env;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(feature = "bundled-installer")]
const ARCHIVE: &[u8] = include_bytes!(env!("JADREN_INSTALLER_ARCHIVE"));

#[cfg(not(feature = "bundled-installer"))]
const ARCHIVE: &[u8] = &[];

fn quote_powershell(path: &Path) -> String {
    format!(
        "'{}'",
        path.display().to_string().replace(char::from(39), "''")
    )
}

fn default_install_root() -> Result<PathBuf, Box<dyn Error>> {
    let local_app_data = env::var_os("LOCALAPPDATA")
        .ok_or("LOCALAPPDATA is not set; pass --install-root explicitly")?;
    Ok(PathBuf::from(local_app_data)
        .join("Programs")
        .join("Jadren"))
}

fn release_label() -> &'static str {
    option_env!("JADREN_RELEASE_LABEL").unwrap_or("0.1.0-preview.1")
}

fn main() -> Result<(), Box<dyn Error>> {
    if !cfg!(windows) {
        return Err("Jadren Windows installer must run on Windows".into());
    }
    if !cfg!(feature = "bundled-installer") {
        return Err(
            "Jadren installer payload is not bundled; rebuild with --features bundled-installer"
                .into(),
        );
    }

    let mut install_root = None;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--install-root" => {
                install_root = Some(PathBuf::from(
                    args.next().ok_or("--install-root requires a path")?,
                ));
            }
            "--help" | "-h" => {
                println!(
                    "Jadren preview installer\nUsage: Jadren-Setup-<version>.exe [--install-root PATH]"
                );
                return Ok(());
            }
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }
    let install_root = install_root.unwrap_or(default_install_root()?);
    fs::create_dir_all(&install_root)?;

    let temp_name = format!("jadren-setup-{}.zip", std::process::id());
    let archive_path = env::temp_dir().join(temp_name);
    fs::write(&archive_path, ARCHIVE)?;
    let command = format!(
        "$ErrorActionPreference='Stop'; Expand-Archive -LiteralPath {} -DestinationPath {} -Force",
        quote_powershell(&archive_path),
        quote_powershell(&install_root),
    );
    let status = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            command.as_str(),
        ])
        .status()?;
    let _ = fs::remove_file(&archive_path);
    if !status.success() {
        return Err(format!("Expand-Archive failed with status {status}").into());
    }

    let marker = install_root.join("INSTALLATION.txt");
    fs::write(
        marker,
        format!(
            "Jadren Windows public preview\r\nVersion: {}\r\nUnsigned preview installer.\r\n",
            release_label()
        ),
    )?;
    println!(
        "Jadren {} installed to {}",
        release_label(),
        install_root.display()
    );
    Ok(())
}
