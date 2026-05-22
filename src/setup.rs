#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

const APP_EXE: &[u8] = include_bytes!("../target/release/tv_guard_tray.exe");
const DEFAULT_CFG: &[u8] = include_bytes!("../TVGuardTray.cfg");

fn main() {
    if let Err(error) = install() {
        show_message("TV Guard Tray Setup", &format!("Install failed:\n{error}"));
    } else {
        show_message("TV Guard Tray Setup", "TV Guard Tray installed and started.");
    }
}

fn install() -> io::Result<()> {
    let install_dir = local_app_data()?.join("TVGuardTray");
    fs::create_dir_all(&install_dir)?;

    let exe_path = install_dir.join("TVGuardTray.exe");
    let cfg_path = install_dir.join("TVGuardTray.cfg");

    let _ = Command::new("taskkill")
        .args(["/IM", "TVGuardTray.exe", "/F"])
        .creation_flags(0x08000000)
        .status();

    fs::write(&exe_path, APP_EXE)?;
    if !cfg_path.exists() {
        fs::write(&cfg_path, DEFAULT_CFG)?;
    }

    ensure_display_config_module()?;

    unblock_file(&exe_path);
    unblock_file(&cfg_path);

    let start_menu = app_data()?
        .join("Microsoft")
        .join("Windows")
        .join("Start Menu")
        .join("Programs");
    let startup = start_menu.join("Startup");
    fs::create_dir_all(&start_menu)?;
    fs::create_dir_all(&startup)?;

    create_shortcut(
        &start_menu.join("TV Guard Tray.lnk"),
        &exe_path,
        &install_dir,
    );
    create_shortcut(
        &startup.join("TV Guard Tray.lnk"),
        &exe_path,
        &install_dir,
    );

    Command::new(&exe_path)
        .current_dir(&install_dir)
        .creation_flags(0x08000000)
        .spawn()?;

    Ok(())
}

fn ensure_display_config_module() -> io::Result<()> {
    let script = r#"
$ErrorActionPreference = 'Stop'
if (-not (Get-Module -ListAvailable -Name DisplayConfig)) {
    try {
        [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    } catch {}

    if (-not (Get-PackageProvider -Name NuGet -ErrorAction SilentlyContinue)) {
        Install-PackageProvider -Name NuGet -Scope CurrentUser -Force | Out-Null
    }

    Install-Module -Name DisplayConfig -Scope CurrentUser -Force -AllowClobber -Repository PSGallery
}
"#;

    let status = Command::new("powershell.exe")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", script])
        .creation_flags(0x08000000)
        .status()?;

    if status.success() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::Other,
            format!("DisplayConfig module install failed with {status}"),
        ))
    }
}

fn local_app_data() -> io::Result<PathBuf> {
    env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "LOCALAPPDATA is not set"))
}

fn app_data() -> io::Result<PathBuf> {
    env::var_os("APPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "APPDATA is not set"))
}

fn unblock_file(path: &Path) {
    let _ = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            "Unblock-File -LiteralPath $env:TVGT_PATH -ErrorAction SilentlyContinue",
        ])
        .env("TVGT_PATH", path)
        .creation_flags(0x08000000)
        .status();
}

fn create_shortcut(shortcut_path: &Path, target_path: &Path, working_dir: &Path) {
    let command = r#"
$shell = New-Object -ComObject WScript.Shell
$shortcut = $shell.CreateShortcut($env:TVGT_SHORTCUT)
$shortcut.TargetPath = $env:TVGT_TARGET
$shortcut.WorkingDirectory = $env:TVGT_WORKDIR
$shortcut.IconLocation = $env:TVGT_TARGET
$shortcut.Save()
"#;

    let _ = Command::new("powershell.exe")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", command])
        .env("TVGT_SHORTCUT", shortcut_path)
        .env("TVGT_TARGET", target_path)
        .env("TVGT_WORKDIR", working_dir)
        .creation_flags(0x08000000)
        .status();
}

fn show_message(title: &str, message: &str) {
    let script = r#"
Add-Type -AssemblyName PresentationFramework
[System.Windows.MessageBox]::Show($env:TVGT_MESSAGE, $env:TVGT_TITLE) | Out-Null
"#;

    let _ = Command::new("powershell.exe")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", script])
        .env("TVGT_TITLE", title)
        .env("TVGT_MESSAGE", message)
        .creation_flags(0x08000000)
        .status();
}

trait CommandExt {
    fn creation_flags(&mut self, flags: u32) -> &mut Self;
}

impl CommandExt for Command {
    fn creation_flags(&mut self, flags: u32) -> &mut Self {
        std::os::windows::process::CommandExt::creation_flags(self, flags)
    }
}
