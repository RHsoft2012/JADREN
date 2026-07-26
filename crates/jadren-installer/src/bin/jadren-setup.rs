use std::env;
use std::error::Error;
use std::fs;
use std::io::Write;
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
    let program_files = env::var_os("PROGRAMFILES")
        .ok_or("PROGRAMFILES is not set; pass --install-root explicitly")?;
    Ok(PathBuf::from(program_files).join("Jadren"))
}

fn release_label() -> &'static str {
    option_env!("JADREN_RELEASE_LABEL").unwrap_or("0.1.0-preview.2")
}

#[cfg(windows)]
fn powershell_quote(value: &str) -> String {
    format!("'{}'", value.replace(char::from(39), "''"))
}

#[cfg(windows)]
fn is_elevated() -> Result<bool, Box<dyn Error>> {
    let output = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "([Security.Principal.WindowsPrincipal] [Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)",
        ])
        .output()?;
    if !output.status.success() {
        return Err("could not determine Windows administrator status".into());
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .trim()
        .eq_ignore_ascii_case("true"))
}

#[cfg(windows)]
fn path_is_under_program_files(path: &Path) -> bool {
    let Some(program_files) = env::var_os("PROGRAMFILES") else {
        return false;
    };
    let root = PathBuf::from(program_files);
    let path = path.to_string_lossy().to_ascii_lowercase();
    let root = root.to_string_lossy().to_ascii_lowercase();
    path == root || path.starts_with(&(root + "\\"))
}

#[cfg(windows)]
fn relaunch_elevated(args: &[String]) -> Result<i32, Box<dyn Error>> {
    let executable = env::current_exe()?;
    let argument_list = args
        .iter()
        .map(|argument| powershell_quote(argument))
        .collect::<Vec<_>>()
        .join(", ");
    let command = format!(
        "$process = Start-Process -FilePath {} -ArgumentList @({}) -Verb RunAs -Wait -PassThru; exit $process.ExitCode",
        powershell_quote(&executable.to_string_lossy()),
        argument_list
    );
    let status = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &command,
        ])
        .status()?;
    Ok(status.code().unwrap_or(1))
}

#[cfg(windows)]
fn update_user_path(bin: &Path) -> Result<(), Box<dyn Error>> {
    let bin = powershell_quote(&bin.to_string_lossy());
    let command = format!(
        "$bin={bin}; $path=[Environment]::GetEnvironmentVariable('Path','User'); $parts=@(); if ($path) {{ $parts=@($path -split ';' | Where-Object {{ $_ }}) }}; if ($parts -notcontains $bin) {{ [Environment]::SetEnvironmentVariable('Path', (($parts + $bin) -join ';'), 'User') }}"
    );
    let status = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &command,
        ])
        .status()?;
    if !status.success() {
        return Err("could not update the user PATH".into());
    }
    Ok(())
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
    let mut update_path = true;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--install-root" => {
                install_root = Some(PathBuf::from(
                    args.next().ok_or("--install-root requires a path")?,
                ));
            }
            "--no-path" => update_path = false,
            "--help" | "-h" => {
                println!(
                    "Jadren preview installer\nUsage: Jadren-Setup-<version>.exe [--install-root PATH] [--no-path]\nDefault install root: %PROGRAMFILES%\\Jadren"
                );
                return Ok(());
            }
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }
    let install_root = install_root.unwrap_or(default_install_root()?);

    #[cfg(windows)]
    if path_is_under_program_files(&install_root) && !is_elevated()? {
        let exit_code = relaunch_elevated(&env::args().skip(1).collect::<Vec<_>>())?;
        std::process::exit(exit_code);
    }

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

    let path_status = if update_path {
        #[cfg(windows)]
        {
            let bin_root = install_root.join("bin");
            match update_user_path(&bin_root) {
                Ok(()) => "updated",
                Err(error) => {
                    eprintln!("warning: PATH was not updated: {error}");
                    "not-updated"
                }
            }
        }
        #[cfg(not(windows))]
        {
            "not-applicable"
        }
    } else {
        "skipped"
    };

    let marker = install_root.join("INSTALLATION.txt");
    fs::write(
        &marker,
        format!(
            "Jadren Windows public preview\r\nVersion: {}\r\nUnsigned preview installer.\r\n",
            release_label(),
        ),
    )?;
    fs::OpenOptions::new()
        .append(true)
        .open(&marker)?
        .write_all(
            format!(
                "Install root: {}\r\nUser PATH: {}\r\n",
                install_root.display(),
                path_status
            )
            .as_bytes(),
        )?;
    println!(
        "Jadren {} installed to {}",
        release_label(),
        install_root.display()
    );
    Ok(())
}
