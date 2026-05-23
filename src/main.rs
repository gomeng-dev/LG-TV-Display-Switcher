#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod config;
mod display;
mod webos;

use audio::{
    choose_tv_audio_endpoint_device, get_default_audio_output_device, list_audio_output_devices,
    set_default_audio_output,
};
use config::{load_or_create_config, save_config_value, save_webos_client_key, AppConfig};
use display::{
    apply_pc_mode_change_display_settings, apply_pc_mode_native, embedded_pc_mode_script,
    embedded_tv_mode_script, is_display_active, read_ddc_power, run_embedded_display_config_script,
    TV_DEVICE_NAME,
};
use std::io::Write;
use std::mem::size_of;
use std::net::IpAddr;
use std::net::UdpSocket;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use webos::{pair_webos_tv, read_webos_power, turn_off_webos_tv};

use serde_json::json;
use windows::core::{Error, Result, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Storage::FileSystem::WriteFile;
use windows::Win32::System::Console::{GetStdHandle, STD_OUTPUT_HANDLE};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DispatchMessageW,
    GetCursorPos, GetMessageW, LoadIconW, MessageBoxW, PostQuitMessage, RegisterClassW,
    SetForegroundWindow, SetTimer, TrackPopupMenu, TranslateMessage, HMENU, IDI_APPLICATION,
    MB_ICONINFORMATION, MB_OK, MF_CHECKED, MF_DISABLED, MF_SEPARATOR, MF_STRING, MF_UNCHECKED, MSG,
    TPM_BOTTOMALIGN, TPM_LEFTALIGN, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_COMMAND, WM_DESTROY,
    WM_LBUTTONUP, WM_RBUTTONUP, WM_TIMER, WNDCLASSW,
};

const CHECK_INTERVAL_MS: u32 = 5_000;
const APP_NAME: &str = "LG-TV-Display-Switcher";
const LOG_FILE_NAME: &str = "LG-TV-Display-Switcher.log";
const APP_ICON_RESOURCE_ID: usize = 1;
const CREATE_NO_WINDOW: u32 = 0x08000000;
const TRAY_UID: u32 = 1;
const WM_TRAY: u32 = WM_APP + 1;
const WM_CHECK_NOW: u32 = WM_APP + 2;
const TIMER_ID: usize = 1;
const MENU_STATUS: usize = 1000;
const MENU_CHECK_NOW: usize = 1001;
const MENU_APPLY_PC: usize = 1002;
const MENU_WAKE_TV: usize = 1003;
const MENU_TURN_OFF_TV: usize = 1004;
const MENU_TOGGLE_TV_POWER: usize = 1005;
const MENU_AUTO_SWITCH_DISPLAYS: usize = 1006;
const MENU_RUN_ONBOARDING: usize = 1007;
const MENU_APPLY_TV: usize = 1008;
const MENU_RUN_AS_STARTUP: usize = 1009;
const MENU_EXIT: usize = 1010;
static APP: OnceLock<Mutex<AppState>> = OnceLock::new();

#[derive(Debug)]
struct AppState {
    config: AppConfig,
    last_status: String,
    last_tv_on: Option<bool>,
    onboarding_started: bool,
}

#[derive(Debug)]
pub(crate) enum TvPower {
    OutputInactive,
    On,
    NotOn { code: Option<u32>, reason: String },
}

fn main() -> Result<()> {
    let base_dir = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    let config = load_or_create_config(&base_dir);

    APP.set(Mutex::new(AppState {
        config,
        last_status: "Starting".to_string(),
        last_tv_on: None,
        onboarding_started: false,
    }))
    .ok();

    if let Some(request) = streamdeck_cli_command() {
        let exit_code = match request {
            Ok(command) => run_streamdeck_cli(&command),
            Err(error) => {
                print_streamdeck_json(false, &error);
                2
            }
        };
        std::process::exit(exit_code);
    }

    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class_name = wide("LgTvDisplaySwitcherWindow");
        let app_icon = load_app_icon()?;

        let window_class = WNDCLASSW {
            hInstance: HINSTANCE(instance.0),
            hIcon: app_icon,
            lpszClassName: PCWSTR(class_name.as_ptr()),
            lpfnWndProc: Some(wnd_proc),
            ..Default::default()
        };

        RegisterClassW(&window_class);

        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            PCWSTR(class_name.as_ptr()),
            PCWSTR(wide(APP_NAME).as_ptr()),
            WINDOW_STYLE::default(),
            0,
            0,
            0,
            0,
            HWND::default(),
            HMENU::default(),
            HINSTANCE(instance.0),
            None,
        )?;

        add_tray_icon(hwnd)?;
        SetTimer(hwnd, TIMER_ID, CHECK_INTERVAL_MS, None);
        run_onboarding_if_needed();
        check_and_apply_pc_mode_if_needed();

        let mut message = MSG::default();
        while GetMessageW(&mut message, HWND::default(), 0, 0).into() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }

        remove_tray_icon(hwnd);
    }

    Ok(())
}

fn streamdeck_cli_command() -> Option<std::result::Result<String, String>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let index = args.iter().position(|arg| arg == "--streamdeck")?;
    let Some(command) = args.get(index + 1) else {
        return Some(Err(
            "Usage: LG-TV-Display-Switcher.exe --streamdeck <command> --json".to_string(),
        ));
    };

    if !args.iter().any(|arg| arg == "--json") {
        return Some(Err("Missing required --json flag".to_string()));
    }

    Some(Ok(command.clone()))
}

fn run_streamdeck_cli(command: &str) -> i32 {
    let result = match command {
        "status" => {
            refresh_tv_power_status(false);
            Ok(())
        }
        "apply-tv-mode" => streamdeck_apply_tv_mode(),
        "apply-pc-mode" => {
            apply_pc_mode_from_state();
            status_result()
        }
        "toggle-tv-power" => {
            toggle_tv_power_from_state();
            status_result()
        }
        "toggle-auto-switch" => {
            toggle_auto_switch_displays();
            status_result()
        }
        _ => {
            print_streamdeck_json(false, &format!("Unknown Stream Deck command: {command}"));
            return 2;
        }
    };

    match result {
        Ok(()) => {
            print_streamdeck_json(true, "");
            0
        }
        Err(error) => {
            print_streamdeck_json(false, &error);
            1
        }
    }
}

fn streamdeck_apply_tv_mode() -> std::result::Result<(), String> {
    refresh_tv_power_status(false);
    if current_tv_on() != Some(true) {
        set_status("TV mode skipped: TV is not on");
        return Err("TV is not on".to_string());
    }

    apply_tv_mode_if_tv_is_on(false);
    status_result()
}

fn status_result() -> std::result::Result<(), String> {
    let status = current_status();
    if status.to_ascii_lowercase().contains("failed") {
        Err(status)
    } else {
        Ok(())
    }
}

fn print_streamdeck_json(ok: bool, error: &str) {
    let payload = json!({
        "ok": ok,
        "status": current_status(),
        "tvOn": current_tv_on(),
        "autoSwitchDisplays": auto_switch_displays(),
        "installRequired": false,
        "error": if error.is_empty() { serde_json::Value::Null } else { json!(error) },
    });
    write_stdout_line(&payload.to_string());
}

fn write_stdout_line(line: &str) {
    let mut output = String::with_capacity(line.len() + 2);
    output.push_str(line);
    output.push_str("\r\n");

    unsafe {
        if let Ok(stdout) = GetStdHandle(STD_OUTPUT_HANDLE) {
            let mut written = 0;
            let _ = WriteFile(stdout, Some(output.as_bytes()), Some(&mut written), None);
        }
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_TIMER => {
            if wparam.0 == TIMER_ID {
                check_and_apply_pc_mode_if_needed();
            }
            LRESULT(0)
        }
        WM_CHECK_NOW => {
            check_and_apply_pc_mode_if_needed();
            LRESULT(0)
        }
        WM_TRAY => {
            match lparam.0 as u32 {
                WM_RBUTTONUP => show_tray_menu(hwnd),
                WM_LBUTTONUP => check_and_apply_pc_mode_if_needed(),
                _ => {}
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            match wparam.0 & 0xffff {
                MENU_CHECK_NOW => check_and_apply_pc_mode_if_needed(),
                MENU_APPLY_PC => apply_pc_mode_from_state(),
                MENU_APPLY_TV => apply_tv_mode_from_state(),
                MENU_WAKE_TV => wake_tv_from_state(),
                MENU_TURN_OFF_TV => turn_off_tv_from_state(),
                MENU_TOGGLE_TV_POWER => toggle_tv_power_from_state(),
                MENU_AUTO_SWITCH_DISPLAYS => toggle_auto_switch_displays(),
                MENU_RUN_ONBOARDING => run_onboarding_from_state(),
                MENU_RUN_AS_STARTUP => toggle_run_as_startup(),
                MENU_EXIT => {
                    remove_tray_icon(hwnd);
                    PostQuitMessage(0);
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            remove_tray_icon(hwnd);
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn check_and_apply_pc_mode_if_needed() {
    refresh_tv_power_status(true);
}

fn refresh_tv_power_status(apply_auto_switch: bool) {
    match read_tv_power() {
        Ok(TvPower::OutputInactive) => update_tv_state(
            false,
            "DISPLAY3 inactive; PC mode already likely active".to_string(),
            apply_auto_switch,
        ),
        Ok(TvPower::On) => update_tv_state(true, "TV is on".to_string(), apply_auto_switch),
        Ok(TvPower::NotOn { code, reason }) => {
            let status = match code {
                Some(value) => format!("TV is not on ({reason}, code {value})"),
                None => format!("TV is not on ({reason})"),
            };
            update_tv_state(false, status, apply_auto_switch);
        }
        Err(error) => {
            set_status(format!("TV power check failed: {error}"));
        }
    }
}

fn update_tv_state(tv_on: bool, status: String, apply_auto_switch: bool) {
    let (auto_switch, previous) = APP
        .get()
        .and_then(|state| {
            state.lock().ok().map(|mut state| {
                let previous = state.last_tv_on;
                state.last_tv_on = Some(tv_on);
                (state.config.auto_switch_displays, previous)
            })
        })
        .unwrap_or((false, None));

    set_status(status);

    if apply_auto_switch && auto_switch && previous.is_some() && previous != Some(tv_on) {
        if tv_on {
            apply_tv_mode_if_tv_is_on(false);
        } else {
            apply_pc_mode_from_state();
        }
    }
}

fn current_tv_on() -> Option<bool> {
    APP.get()
        .and_then(|state| state.lock().ok().and_then(|state| state.last_tv_on))
}

fn auto_switch_displays() -> bool {
    APP.get()
        .and_then(|state| {
            state
                .lock()
                .ok()
                .map(|state| state.config.auto_switch_displays)
        })
        .unwrap_or(false)
}

fn wake_tv_from_state() {
    let config = APP
        .get()
        .and_then(|state| state.lock().ok().map(|state| state.config.clone()));

    let Some(config) = config else {
        set_status("Wake TV failed: config unavailable");
        return;
    };

    let Some(mac) = config.tv_mac else {
        set_status("Wake TV failed: TvMac is not configured");
        return;
    };

    match send_wake_on_lan(mac, &config.wake_broadcast, config.wake_port) {
        Ok(()) => set_status(format!(
            "Wake TV packet sent to {}:{}",
            config.wake_broadcast, config.wake_port
        )),
        Err(error) => set_status(format!("Wake TV failed: {error}")),
    }
}

fn toggle_tv_power_from_state() {
    match current_tv_on() {
        Some(true) => turn_off_tv_from_state(),
        Some(false) => wake_tv_from_state(),
        None => {
            check_and_apply_pc_mode_if_needed();
            match current_tv_on() {
                Some(true) => turn_off_tv_from_state(),
                _ => wake_tv_from_state(),
            }
        }
    }
}

fn toggle_auto_switch_displays() {
    let (enabled, current_tv_on) = APP
        .get()
        .and_then(|state| {
            state.lock().ok().map(|mut state| {
                state.config.auto_switch_displays = !state.config.auto_switch_displays;
                (state.config.auto_switch_displays, state.last_tv_on)
            })
        })
        .unwrap_or((false, None));

    save_config_value("AutoSwitchDisplays", if enabled { "true" } else { "false" });

    if enabled {
        set_status("Auto switch displays enabled");
        match current_tv_on {
            Some(true) => apply_tv_mode_if_tv_is_on(false),
            Some(false) => apply_pc_mode_from_state(),
            None => check_and_apply_pc_mode_if_needed(),
        }
    } else {
        set_status("Auto switch displays disabled");
    }
}

fn toggle_run_as_startup() {
    if startup_enabled() {
        match disable_startup() {
            Ok(()) => set_status("Run as startup disabled"),
            Err(error) => set_status(format!("Failed to disable run as startup: {error}")),
        }
    } else {
        match enable_startup() {
            Ok(()) => set_status("Run as startup enabled"),
            Err(error) => set_status(format!("Failed to enable run as startup: {error}")),
        }
    }
}

fn turn_off_tv_from_state() {
    let config = APP
        .get()
        .and_then(|state| state.lock().ok().map(|state| state.config.clone()));

    let Some(mut config) = config else {
        set_status("Turn TV off failed: config unavailable");
        return;
    };

    let Some(host) = config.webos_host.clone() else {
        set_status("Turn TV off failed: WebOsHost is not configured");
        return;
    };

    if config.webos_client_key.is_none() {
        set_status("Turn TV off pairing requested; approve the prompt on the TV");
    }

    match turn_off_webos_tv(&host, &mut config) {
        Ok(Some(client_key)) => {
            save_webos_client_key(&client_key);
            if let Some(state) = APP.get() {
                if let Ok(mut state) = state.lock() {
                    state.config.webos_client_key = Some(client_key);
                }
            }
            set_status("Turn TV off command sent; webOS client key saved");
            apply_pc_mode_from_state();
        }
        Ok(None) => {
            set_status("Turn TV off command sent");
            apply_pc_mode_from_state();
        }
        Err(error) => set_status(format!("Turn TV off failed: {error}")),
    }
}

fn run_onboarding_if_needed() {
    let should_run = APP
        .get()
        .and_then(|state| {
            state.lock().ok().map(|mut state| {
                if state.onboarding_started || !onboarding_needed(&state.config) {
                    false
                } else {
                    state.onboarding_started = true;
                    true
                }
            })
        })
        .unwrap_or(false);

    if should_run {
        run_onboarding_from_state();
    }
}

fn onboarding_needed(config: &AppConfig) -> bool {
    let host_missing = config
        .webos_host
        .as_deref()
        .map(|host| host.trim().is_empty() || host.eq_ignore_ascii_case("lgwebostv"))
        .unwrap_or(true);

    host_missing || config.webos_client_key.is_none()
}

fn run_onboarding_from_state() {
    set_status("Onboarding started");
    show_info(
        APP_NAME,
        "LG TV onboarding will search the local network, connect to the TV, and ask for pairing approval on the TV screen.\n\nTurn the TV on and approve the prompt when it appears.",
    );

    match run_onboarding() {
        Ok(report) => {
            set_status("Onboarding completed");
            show_info(APP_NAME, &report);
        }
        Err(error) => {
            set_status(format!("Onboarding failed: {error}"));
            show_info(APP_NAME, &format!("Onboarding failed:\n{error}"));
        }
    }
}

fn run_onboarding() -> std::result::Result<String, String> {
    let config = APP
        .get()
        .and_then(|state| state.lock().ok().map(|state| state.config.clone()))
        .ok_or_else(|| "config unavailable".to_string())?;

    let mut candidates = Vec::<String>::new();
    if let Some(host) = config.webos_host.as_deref() {
        if !host.trim().is_empty() && !host.eq_ignore_ascii_case("lgwebostv") {
            candidates.push(host.to_string());
        }
    }

    set_status("Searching for LG webOS TVs on the local network");
    for ip in discover_lg_webos_tvs(Duration::from_secs(3)) {
        push_unique(&mut candidates, ip.to_string());
    }

    if let Some(host) = config.webos_host.as_deref() {
        if !host.trim().is_empty() {
            push_unique(&mut candidates, host.to_string());
        }
    }

    if candidates.is_empty() {
        return Err("No LG webOS TV was found. Check that the TV is on and connected to the same local network.".to_string());
    }

    let mut ports = Vec::new();
    for port in [config.webos_port, 3001, 3000] {
        if !ports.contains(&port) {
            ports.push(port);
        }
    }

    let mut found = None;
    for host in &candidates {
        for port in &ports {
            set_status(format!("Checking webOS TV at {host}:{port}"));
            if matches!(
                read_webos_power(
                    host,
                    *port,
                    Duration::from_millis(2500),
                    config.webos_client_key.as_deref(),
                ),
                TvPower::On
            ) {
                found = Some((host.clone(), *port));
                break;
            }
        }
        if found.is_some() {
            break;
        }
    }

    let Some((host, port)) = found else {
        return Err(format!(
            "Found candidates, but none accepted a webOS connection: {}",
            candidates.join(", ")
        ));
    };

    save_config_value("WebOsHost", &host);
    save_config_value("WebOsPort", &port.to_string());
    if let Some(state) = APP.get() {
        if let Ok(mut state) = state.lock() {
            state.config.webos_host = Some(host.clone());
            state.config.webos_port = port;
        }
    }

    set_status("Pairing with webOS TV; approve the prompt on the TV");
    let client_key = pair_webos_tv(&host, port, config.webos_client_key.as_deref())?;
    if let Some(client_key) = client_key {
        save_webos_client_key(&client_key);
        if let Some(state) = APP.get() {
            if let Ok(mut state) = state.lock() {
                state.config.webos_client_key = Some(client_key);
            }
        }
    }

    let mut notes = Vec::new();
    notes.push(format!("TV: {host}:{port}"));

    set_status("Applying TV display mode before audio endpoint scan");
    match run_embedded_display_config_script(embedded_tv_mode_script()) {
        Ok(()) => {
            notes.push("Display mode: TV mode applied".to_string());
            std::thread::sleep(Duration::from_millis(1200));
        }
        Err(error) => {
            notes.push(format!("Display mode: skipped ({error})"));
            log_message(&format!("Onboarding TV mode apply failed: {error}"));
        }
    }

    set_status("Scanning active audio output devices");
    match list_audio_output_devices() {
        Ok(devices) if !devices.is_empty() => {
            for device in &devices {
                match device.id.as_deref() {
                    Some(id) => log_message(&format!("Audio endpoint: {} [{id}]", device.name)),
                    None => log_message(&format!("Audio endpoint: {}", device.name)),
                }
            }

            if let Some(selected) = choose_tv_audio_endpoint_device(
                &devices,
                config.tv_audio_device_name_contains.as_deref(),
            ) {
                save_config_value("TvAudioDeviceNameContains", &selected.name);
                save_config_value(
                    "TvAudioEndpointId",
                    selected.id.as_deref().unwrap_or_default(),
                );
                if let Some(state) = APP.get() {
                    if let Ok(mut state) = state.lock() {
                        state.config.tv_audio_device_name_contains = Some(selected.name.clone());
                        state.config.tv_audio_endpoint_id = selected.id.clone();
                    }
                }
                notes.push(format!("TV audio endpoint: {}", selected.name));
                if selected.id.is_some() {
                    notes.push("TV audio endpoint ID saved".to_string());
                }
            } else {
                let names = devices
                    .iter()
                    .map(|device| device.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                notes.push(format!("Audio endpoints found: {names}"));
            }
        }
        Ok(_) => notes.push("Audio endpoint scan found no active output devices".to_string()),
        Err(error) => {
            notes.push(
                "Audio endpoint scan failed; TV/audio pairing is saved, but audio device name may need manual configuration".to_string(),
            );
            log_message(&format!("Audio endpoint scan failed: {error}"));
        }
    }

    Ok(format!("Onboarding completed.\n\n{}", notes.join("\n")))
}

fn discover_lg_webos_tvs(duration: Duration) -> Vec<IpAddr> {
    let mut found = Vec::<IpAddr>::new();
    let Ok(socket) = UdpSocket::bind("0.0.0.0:0") else {
        return found;
    };

    let _ = socket.set_read_timeout(Some(Duration::from_millis(350)));
    let _ = socket.set_multicast_loop_v4(false);

    let searches = [
        "M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nMAN: \"ssdp:discover\"\r\nMX: 2\r\nST: ssdp:all\r\n\r\n",
        "M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nMAN: \"ssdp:discover\"\r\nMX: 2\r\nST: urn:lge-com:service:webos-second-screen:1\r\n\r\n",
    ];

    for search in searches {
        let _ = socket.send_to(search.as_bytes(), "239.255.255.250:1900");
    }

    let deadline = Instant::now() + duration;
    let mut buffer = [0u8; 4096];
    while Instant::now() < deadline {
        match socket.recv_from(&mut buffer) {
            Ok((size, sender)) => {
                let response = String::from_utf8_lossy(&buffer[..size]).to_ascii_lowercase();
                if response.contains("webos")
                    || response.contains("lge")
                    || response.contains("lg smart")
                {
                    let ip = sender.ip();
                    if !found.contains(&ip) {
                        found.push(ip);
                    }
                }
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.kind() == std::io::ErrorKind::TimedOut => {}
            Err(error) => {
                log_message(&format!("SSDP discovery failed: {error}"));
                break;
            }
        }
    }

    found
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(&value))
    {
        values.push(value);
    }
}

fn show_info(title: &str, message: &str) {
    let title = wide(title);
    let message = wide(message);
    unsafe {
        let _ = MessageBoxW(
            HWND::default(),
            PCWSTR(message.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_OK | MB_ICONINFORMATION,
        );
    }
}

fn send_wake_on_lan(mac: [u8; 6], broadcast: &str, port: u16) -> std::io::Result<()> {
    let mut packet = [0u8; 102];
    packet[..6].fill(0xff);
    for idx in 0..16 {
        let start = 6 + idx * 6;
        packet[start..start + 6].copy_from_slice(&mac);
    }

    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.set_broadcast(true)?;
    socket.send_to(&packet, format!("{broadcast}:{port}"))?;
    Ok(())
}

fn apply_pc_mode_from_state() {
    match run_embedded_display_config_script(embedded_pc_mode_script()) {
        Ok(()) => {
            set_status("PC mode applied");
            apply_pc_audio_from_state();
        }
        Err(script_error) => {
            log_message(&format!(
                "Embedded PC mode failed, trying native fallback: {script_error}"
            ));
            match apply_pc_mode_native() {
                Ok(()) => {
                    set_status("PC mode applied with native DisplayConfig");
                    apply_pc_audio_from_state();
                }
                Err(error) => {
                    log_message(&format!("Native DisplayConfig apply failed, trying ChangeDisplaySettingsEx fallback: {error}"));
                    match apply_pc_mode_change_display_settings() {
                        Ok(()) => {
                            set_status("PC mode applied with ChangeDisplaySettingsEx");
                            apply_pc_audio_from_state();
                        }
                        Err(fallback_error) => {
                            set_status(format!("Failed to apply PC mode: {fallback_error}"))
                        }
                    }
                }
            }
        }
    }
}

fn apply_pc_audio_from_state() {
    let config = APP
        .get()
        .and_then(|state| state.lock().ok().map(|state| state.config.clone()));

    let Some(config) = config else {
        log_message("PC audio switch skipped: config unavailable");
        return;
    };

    if !config.auto_switch_audio {
        return;
    }

    let Some(name_filter) = config.pc_audio_device_name_contains.as_deref() else {
        log_message("PC audio switch skipped: PcAudioDeviceNameContains is not configured");
        return;
    };

    match set_default_audio_output(config.pc_audio_endpoint_id.as_deref(), name_filter) {
        Ok(device_name) => set_status(format!("Default audio output restored to {device_name}")),
        Err(error) => set_status(format!("Failed to restore PC audio output: {error}")),
    }
}

fn remember_current_audio_before_tv_mode() {
    let config = APP
        .get()
        .and_then(|state| state.lock().ok().map(|state| state.config.clone()));

    let Some(config) = config else {
        log_message("Previous PC audio capture skipped: config unavailable");
        return;
    };

    if !config.auto_switch_audio {
        return;
    }

    match get_default_audio_output_device() {
        Ok(device) => {
            if is_tv_audio_device(&device, &config) {
                log_message("Previous PC audio capture skipped: current output is TV audio");
                return;
            }

            save_config_value("PcAudioDeviceNameContains", &device.name);
            save_config_value(
                "PcAudioEndpointId",
                device.id.as_deref().unwrap_or_default(),
            );

            if let Some(state) = APP.get() {
                if let Ok(mut state) = state.lock() {
                    state.config.pc_audio_device_name_contains = Some(device.name.clone());
                    state.config.pc_audio_endpoint_id = device.id.clone();
                }
            }

            log_message(&format!(
                "Previous PC audio output remembered: {}",
                device.name
            ));
        }
        Err(error) => log_message(&format!("Previous PC audio capture failed: {error}")),
    }
}

fn is_tv_audio_device(device: &audio::AudioOutputDevice, config: &AppConfig) -> bool {
    if let (Some(current_id), Some(tv_id)) =
        (device.id.as_deref(), config.tv_audio_endpoint_id.as_deref())
    {
        if !tv_id.trim().is_empty() && current_id.eq_ignore_ascii_case(tv_id) {
            return true;
        }
    }

    if let Some(tv_name) = config.tv_audio_device_name_contains.as_deref() {
        let current = device
            .name
            .chars()
            .filter(|ch| !ch.is_whitespace())
            .collect::<String>()
            .to_ascii_lowercase();
        let tv = tv_name
            .chars()
            .filter(|ch| !ch.is_whitespace())
            .collect::<String>()
            .to_ascii_lowercase();
        return !tv.is_empty() && current.contains(&tv);
    }

    false
}

fn apply_tv_mode_from_state() {
    apply_tv_mode_if_tv_is_on(true);
}

fn apply_tv_mode_if_tv_is_on(refresh_power: bool) {
    if refresh_power {
        check_and_apply_pc_mode_if_needed();
    }

    if current_tv_on() != Some(true) {
        set_status("TV mode skipped: TV is not on");
        return;
    }

    remember_current_audio_before_tv_mode();

    match run_embedded_display_config_script(embedded_tv_mode_script()) {
        Ok(()) => {
            set_status("TV mode applied");
            apply_tv_audio_from_state();
        }
        Err(error) => set_status(format!("Failed to apply TV mode: {error}")),
    }
}

fn apply_tv_audio_from_state() {
    let config = APP
        .get()
        .and_then(|state| state.lock().ok().map(|state| state.config.clone()));

    let Some(config) = config else {
        set_status("TV audio switch skipped: config unavailable");
        return;
    };

    if !config.auto_switch_audio {
        return;
    }

    let Some(name_filter) = config.tv_audio_device_name_contains.as_deref() else {
        set_status("TV audio switch skipped: TvAudioDeviceNameContains is not configured");
        return;
    };

    match set_default_audio_output(config.tv_audio_endpoint_id.as_deref(), name_filter) {
        Ok(device_name) => {
            set_status(format!("Default audio output set to {device_name}"));
            if config.try_enable_dolby_atmos {
                set_status("Dolby Atmos auto-enable is not implemented; set it once in Windows spatial sound settings");
            }
        }
        Err(error) => {
            set_status(format!("Failed to switch TV audio output: {error}"));
        }
    }
}

fn set_status(status: impl Into<String>) {
    let status = status.into();
    log_message(&status);
    if let Some(state) = APP.get() {
        if let Ok(mut state) = state.lock() {
            state.last_status = status;
        }
    }
}

fn log_message(message: &str) {
    let base_dir = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    let log_path = base_dir.join(LOG_FILE_NAME);
    let line = format!("{:?} {message}\r\n", std::time::SystemTime::now());
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .and_then(|mut file| file.write_all(line.as_bytes()));
}

fn current_status() -> String {
    APP.get()
        .and_then(|state| state.lock().ok().map(|state| state.last_status.clone()))
        .unwrap_or_else(|| "Unknown".to_string())
}

fn read_tv_power() -> Result<TvPower> {
    let config = APP
        .get()
        .and_then(|state| state.lock().ok().map(|state| state.config.clone()))
        .unwrap_or_default();

    if let Some(host) = config.webos_host.as_deref() {
        return Ok(read_webos_power(
            host,
            config.webos_port,
            config.webos_timeout,
            config.webos_client_key.as_deref(),
        ));
    }

    if is_display_active(TV_DEVICE_NAME) {
        read_ddc_power(TV_DEVICE_NAME)
    } else {
        Ok(TvPower::OutputInactive)
    }
}

fn startup_enabled() -> bool {
    startup_run_registry_enabled() || startup_shortcut_path().is_some_and(|path| path.exists())
}

fn startup_run_registry_enabled() -> bool {
    Command::new("reg.exe")
        .arg("query")
        .arg("HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run")
        .arg("/v")
        .arg(APP_NAME)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn enable_startup() -> std::result::Result<(), String> {
    let exe = std::env::current_exe()
        .map_err(|error| format!("failed to locate current executable: {error}"))?;
    let command = format!("\"{}\"", exe.display());
    let output = Command::new("reg.exe")
        .arg("add")
        .arg("HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run")
        .arg("/v")
        .arg(APP_NAME)
        .arg("/t")
        .arg("REG_SZ")
        .arg("/d")
        .arg(command)
        .arg("/f")
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|error| format!("failed to start reg.exe: {error}"))?;

    if output.status.success() {
        Ok(())
    } else {
        Err(command_error("reg add", &output))
    }
}

fn disable_startup() -> std::result::Result<(), String> {
    let output = Command::new("reg.exe")
        .arg("delete")
        .arg("HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run")
        .arg("/v")
        .arg(APP_NAME)
        .arg("/f")
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|error| format!("failed to start reg.exe: {error}"))?;

    if !output.status.success() && startup_run_registry_enabled() {
        return Err(command_error("reg delete", &output));
    }

    if let Some(path) = startup_shortcut_path() {
        if path.exists() {
            std::fs::remove_file(&path).map_err(|error| {
                format!(
                    "failed to remove startup shortcut {}: {error}",
                    path.display()
                )
            })?;
        }
    }

    Ok(())
}

fn startup_shortcut_path() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    Some(
        PathBuf::from(appdata)
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs")
            .join("Startup")
            .join(format!("{APP_NAME}.lnk")),
    )
}

fn command_error(action: &str, output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !stderr.is_empty() {
        format!("{action} failed: {stderr}")
    } else if !stdout.is_empty() {
        format!("{action} failed: {stdout}")
    } else {
        format!("{action} exited with {}", output.status)
    }
}

unsafe fn add_tray_icon(hwnd: HWND) -> Result<()> {
    let icon = load_app_icon()?;
    let mut data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_UID,
        uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
        uCallbackMessage: WM_TRAY,
        hIcon: icon,
        ..Default::default()
    };

    copy_wide_to_fixed(APP_NAME, &mut data.szTip);

    if Shell_NotifyIconW(NIM_ADD, &data).as_bool() {
        Ok(())
    } else {
        Err(Error::from_win32())
    }
}

unsafe fn load_app_icon() -> Result<windows::Win32::UI::WindowsAndMessaging::HICON> {
    let instance = GetModuleHandleW(None)?;
    let resource_name = PCWSTR(APP_ICON_RESOURCE_ID as *const u16);
    LoadIconW(HINSTANCE(instance.0), resource_name).or_else(|_| LoadIconW(None, IDI_APPLICATION))
}

unsafe fn remove_tray_icon(hwnd: HWND) {
    let data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_UID,
        ..Default::default()
    };

    let _ = Shell_NotifyIconW(NIM_DELETE, &data);
}

unsafe fn show_tray_menu(hwnd: HWND) {
    let Ok(menu) = CreatePopupMenu() else {
        return;
    };

    let status = current_status();
    let status_label = format!("Status: {status}");
    let tv_power_label = match current_tv_on() {
        Some(true) => "TV Power: Turn off",
        Some(false) => "TV Power: Turn on",
        None => "TV Power: Check and toggle",
    };
    let mode_action = match current_tv_on() {
        Some(true) => (MENU_APPLY_PC, "Apply PC mode"),
        _ => (MENU_APPLY_TV, "Apply TV mode"),
    };
    let auto_switch_flag = if auto_switch_displays() {
        MF_STRING | MF_CHECKED
    } else {
        MF_STRING | MF_UNCHECKED
    };
    let startup_flag = if startup_enabled() {
        MF_STRING | MF_CHECKED
    } else {
        MF_STRING | MF_UNCHECKED
    };

    let _ = AppendMenuW(
        menu,
        MF_STRING | MF_DISABLED,
        MENU_STATUS,
        PCWSTR(wide(&status_label).as_ptr()),
    );
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    let _ = AppendMenuW(
        menu,
        MF_STRING,
        MENU_CHECK_NOW,
        PCWSTR(wide("Check now").as_ptr()),
    );
    let _ = AppendMenuW(
        menu,
        MF_STRING,
        MENU_RUN_ONBOARDING,
        PCWSTR(wide("Run onboarding").as_ptr()),
    );
    let _ = AppendMenuW(
        menu,
        MF_STRING,
        MENU_TOGGLE_TV_POWER,
        PCWSTR(wide(tv_power_label).as_ptr()),
    );
    let _ = AppendMenuW(
        menu,
        auto_switch_flag,
        MENU_AUTO_SWITCH_DISPLAYS,
        PCWSTR(wide("Auto switch displays").as_ptr()),
    );
    let _ = AppendMenuW(
        menu,
        MF_STRING,
        mode_action.0,
        PCWSTR(wide(mode_action.1).as_ptr()),
    );
    let _ = AppendMenuW(
        menu,
        startup_flag,
        MENU_RUN_AS_STARTUP,
        PCWSTR(wide("Run as startup").as_ptr()),
    );
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    let _ = AppendMenuW(menu, MF_STRING, MENU_EXIT, PCWSTR(wide("Exit").as_ptr()));

    let mut point = POINT::default();
    let _ = GetCursorPos(&mut point);
    let _ = SetForegroundWindow(hwnd);
    let _ = TrackPopupMenu(
        menu,
        TPM_LEFTALIGN | TPM_BOTTOMALIGN,
        point.x,
        point.y,
        0,
        hwnd,
        None,
    );
    let _ = DestroyMenu(menu);
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn copy_wide_to_fixed(value: &str, target: &mut [u16]) {
    let encoded = wide(value);
    let len = encoded.len().min(target.len());
    target[..len].copy_from_slice(&encoded[..len]);
}
