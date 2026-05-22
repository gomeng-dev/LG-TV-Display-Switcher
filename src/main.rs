#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::HashMap;
use std::io::{Read, Write};
use std::mem::size_of;
use std::net::UdpSocket;
use std::net::{IpAddr, TcpStream, ToSocketAddrs};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use native_tls::TlsConnector;
use serde_json::{json, Value};
use windows::core::{Error, Result, PCWSTR};
use windows::Win32::Devices::Display::{
    DestroyPhysicalMonitors, DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes,
    GetNumberOfPhysicalMonitorsFromHMONITOR, GetPhysicalMonitorsFromHMONITOR,
    GetVCPFeatureAndVCPFeatureReply, QueryDisplayConfig, SetDisplayConfig,
    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_MODE_INFO,
    DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE, DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SOURCE_DEVICE_NAME,
    MC_VCP_CODE_TYPE, PHYSICAL_MONITOR, QDC_ONLY_ACTIVE_PATHS, SDC_ALLOW_CHANGES, SDC_APPLY,
    SDC_PATH_PERSIST_IF_REQUIRED, SDC_SAVE_TO_DATABASE, SDC_USE_SUPPLIED_DISPLAY_CONFIG,
};
use windows::Win32::Foundation::{BOOL, HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    ChangeDisplaySettingsExW, EnumDisplayMonitors, EnumDisplaySettingsW, GetMonitorInfoW,
    CDS_NORESET, CDS_UPDATEREGISTRY, DEVMODEW, DISP_CHANGE_SUCCESSFUL, DM_PELSHEIGHT, DM_PELSWIDTH,
    DM_POSITION, ENUM_CURRENT_SETTINGS, ENUM_REGISTRY_SETTINGS, HDC, HMONITOR, MONITORINFOEXW,
};
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

const TV_DEVICE_NAME: &str = r"\\.\DISPLAY3";
const PRIMARY_DEVICE_NAME: &str = r"\\.\DISPLAY2";
const CHECK_INTERVAL_MS: u32 = 5_000;
const APP_NAME: &str = "LG-TV-Display-Switcher";
const CONFIG_FILE_NAME: &str = "LG-TV-Display-Switcher.cfg";
const LEGACY_CONFIG_FILE_NAME: &str = "TVGuardTray.cfg";
const LOG_FILE_NAME: &str = "LG-TV-Display-Switcher.log";
const DEFAULT_WEBOS_PORT: u16 = 3001;
const DEFAULT_WEBOS_TIMEOUT_MS: u64 = 1500;
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
const MENU_EXIT: usize = 1008;
const CREATE_NO_WINDOW: u32 = 0x08000000;

static APP: OnceLock<Mutex<AppState>> = OnceLock::new();

#[derive(Debug)]
struct AppState {
    config: AppConfig,
    last_status: String,
    last_tv_on: Option<bool>,
    onboarding_started: bool,
}

#[derive(Clone, Debug)]
struct AppConfig {
    webos_host: Option<String>,
    webos_port: u16,
    webos_timeout: Duration,
    auto_apply_pc_mode: bool,
    auto_switch_displays: bool,
    tv_mac: Option<[u8; 6]>,
    wake_broadcast: String,
    wake_port: u16,
    webos_client_key: Option<String>,
    auto_switch_audio: bool,
    tv_audio_device_name_contains: Option<String>,
    try_enable_dolby_atmos: bool,
}

#[derive(Debug)]
enum TvPower {
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

    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class_name = wide("LgTvDisplaySwitcherWindow");

        let window_class = WNDCLASSW {
            hInstance: HINSTANCE(instance.0),
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
                MENU_WAKE_TV => wake_tv_from_state(),
                MENU_TURN_OFF_TV => turn_off_tv_from_state(),
                MENU_TOGGLE_TV_POWER => toggle_tv_power_from_state(),
                MENU_AUTO_SWITCH_DISPLAYS => toggle_auto_switch_displays(),
                MENU_RUN_ONBOARDING => run_onboarding_from_state(),
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
    match read_tv_power() {
        Ok(TvPower::OutputInactive) => update_tv_state(
            false,
            "DISPLAY3 inactive; PC mode already likely active".to_string(),
        ),
        Ok(TvPower::On) => update_tv_state(true, "TV is on".to_string()),
        Ok(TvPower::NotOn { code, reason }) => {
            let status = match code {
                Some(value) => format!("TV is not on ({reason}, code {value})"),
                None => format!("TV is not on ({reason})"),
            };
            update_tv_state(false, status);
        }
        Err(error) => {
            set_status(format!("TV power check failed: {error}"));
        }
    }
}

fn update_tv_state(tv_on: bool, status: String) {
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

    if auto_switch && previous.is_some() && previous != Some(tv_on) {
        if tv_on {
            apply_tv_mode_from_state();
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

fn load_or_create_config(base_dir: &Path) -> AppConfig {
    let path = base_dir.join(CONFIG_FILE_NAME);
    if !path.exists() {
        let legacy_path = base_dir.join(LEGACY_CONFIG_FILE_NAME);
        if legacy_path.exists() {
            let _ = std::fs::copy(&legacy_path, &path);
        } else {
            let _ = std::fs::write(&path, default_config_template());
        }
    }

    let mut config = AppConfig {
        webos_host: Some("lgwebostv".to_string()),
        webos_port: DEFAULT_WEBOS_PORT,
        webos_timeout: Duration::from_millis(DEFAULT_WEBOS_TIMEOUT_MS),
        auto_apply_pc_mode: false,
        auto_switch_displays: false,
        tv_mac: None,
        wake_broadcast: "192.168.0.255".to_string(),
        wake_port: 9,
        webos_client_key: None,
        auto_switch_audio: true,
        tv_audio_device_name_contains: Some("NVIDIA High Definition Audio".to_string()),
        try_enable_dolby_atmos: false,
    };

    let Ok(contents) = std::fs::read_to_string(path) else {
        return config;
    };

    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };

        let key = key.trim().to_ascii_lowercase();
        let value = value.trim();

        match key.as_str() {
            "weboshost" => {
                config.webos_host = (!value.is_empty()).then(|| value.to_string());
            }
            "webosport" => {
                if let Ok(port) = value.parse::<u16>() {
                    config.webos_port = port;
                }
            }
            "webostimeoutms" => {
                if let Ok(ms) = value.parse::<u64>() {
                    config.webos_timeout = Duration::from_millis(ms);
                }
            }
            "autoapplypcmode" => {
                config.auto_apply_pc_mode = matches!(
                    value.to_ascii_lowercase().as_str(),
                    "1" | "yes" | "true" | "on"
                );
            }
            "autoswitchdisplays" => {
                config.auto_switch_displays = matches!(
                    value.to_ascii_lowercase().as_str(),
                    "1" | "yes" | "true" | "on"
                );
            }
            "tvmac" => {
                config.tv_mac = parse_mac_address(value);
            }
            "wakebroadcast" => {
                if !value.is_empty() {
                    config.wake_broadcast = value.to_string();
                }
            }
            "wakeport" => {
                if let Ok(port) = value.parse::<u16>() {
                    config.wake_port = port;
                }
            }
            "webosclientkey" => {
                config.webos_client_key = (!value.is_empty()).then(|| value.to_string());
            }
            "autoswitchaudio" => {
                config.auto_switch_audio = matches!(
                    value.to_ascii_lowercase().as_str(),
                    "1" | "yes" | "true" | "on"
                );
            }
            "tvaudiodevicenamecontains" => {
                config.tv_audio_device_name_contains =
                    (!value.is_empty()).then(|| value.to_string());
            }
            "tryenabledolbyatmos" => {
                config.try_enable_dolby_atmos = matches!(
                    value.to_ascii_lowercase().as_str(),
                    "1" | "yes" | "true" | "on"
                );
            }
            _ => {}
        }
    }

    config
}

fn default_config_template() -> String {
    [
        "# LG webOS TV address. Prefer a fixed IP from your router, for example: WebOsHost=192.168.0.50",
        "# Hostname lgwebostv sometimes works, but a fixed IP is more reliable.",
        "WebOsHost=lgwebostv",
        "WebOsPort=3001",
        "WebOsTimeoutMs=1500",
        "AutoApplyPcMode=false",
        "AutoSwitchDisplays=false",
        "TvMac=",
        "WakeBroadcast=192.168.0.255",
        "WakePort=9",
        "WebOsClientKey=",
        "AutoSwitchAudio=true",
        "TvAudioDeviceNameContains=NVIDIA High Definition Audio",
        "TryEnableDolbyAtmos=false",
        "",
    ]
    .join("\r\n")
}

fn wake_tv_from_state() {
    let config = APP.get().and_then(|state| {
        state.lock().ok().map(|state| AppConfig {
            webos_host: state.config.webos_host.clone(),
            webos_port: state.config.webos_port,
            webos_timeout: state.config.webos_timeout,
            auto_apply_pc_mode: state.config.auto_apply_pc_mode,
            auto_switch_displays: state.config.auto_switch_displays,
            tv_mac: state.config.tv_mac,
            wake_broadcast: state.config.wake_broadcast.clone(),
            wake_port: state.config.wake_port,
            webos_client_key: state.config.webos_client_key.clone(),
            auto_switch_audio: state.config.auto_switch_audio,
            tv_audio_device_name_contains: state.config.tv_audio_device_name_contains.clone(),
            try_enable_dolby_atmos: state.config.try_enable_dolby_atmos,
        })
    });

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
            Some(true) => apply_tv_mode_from_state(),
            Some(false) => apply_pc_mode_from_state(),
            None => check_and_apply_pc_mode_if_needed(),
        }
    } else {
        set_status("Auto switch displays disabled");
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
        }
        Ok(None) => set_status("Turn TV off command sent"),
        Err(error) => set_status(format!("Turn TV off failed: {error}")),
    }
}

trait ReadWrite: Read + Write {}
impl<T: Read + Write> ReadWrite for T {}

fn turn_off_webos_tv(
    host: &str,
    config: &mut AppConfig,
) -> std::result::Result<Option<String>, String> {
    let timeout = if config.webos_client_key.is_some() {
        config.webos_timeout
    } else {
        Duration::from_secs(30)
    };
    let mut stream = connect_webos_socket(host, config.webos_port, timeout)?;
    let client_key = register_webos_client(&mut *stream, config.webos_client_key.as_deref())?;

    let payload = if let Some(key) = client_key.as_deref() {
        json!({ "client-key": key })
    } else {
        json!({})
    };

    let request = json!({
        "id": "turn_off",
        "type": "request",
        "uri": "ssap://system/turnOff",
        "payload": payload
    });
    send_ws_text(&mut *stream, &request.to_string())?;

    Ok(client_key)
}

fn connect_webos_socket(
    host: &str,
    port: u16,
    timeout: Duration,
) -> std::result::Result<Box<dyn ReadWrite>, String> {
    let address = format!("{host}:{port}");
    let mut addrs = address
        .to_socket_addrs()
        .map_err(|error| format!("webOS host not found: {error}"))?;
    let socket_addr = addrs
        .next()
        .ok_or_else(|| format!("webOS host has no address: {host}"))?;

    let stream = TcpStream::connect_timeout(&socket_addr, timeout)
        .map_err(|error| format!("webOS port {port} unreachable: {error}"))?;
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));

    if port == 3001 {
        let connector = TlsConnector::builder()
            .danger_accept_invalid_certs(true)
            .danger_accept_invalid_hostnames(true)
            .build()
            .map_err(|error| format!("webOS TLS setup failed: {error}"))?;
        let tls = connector
            .connect(host, stream)
            .map_err(|error| format!("webOS TLS handshake failed: {error}"))?;
        let mut boxed: Box<dyn ReadWrite> = Box::new(tls);
        websocket_upgrade(&mut *boxed, host, port)?;
        Ok(boxed)
    } else {
        let mut boxed: Box<dyn ReadWrite> = Box::new(stream);
        websocket_upgrade(&mut *boxed, host, port)?;
        Ok(boxed)
    }
}

fn websocket_upgrade(
    stream: &mut dyn ReadWrite,
    host: &str,
    port: u16,
) -> std::result::Result<(), String> {
    let request = format!(
        "GET / HTTP/1.1\r\n\
         Host: {host}:{port}\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
         Sec-WebSocket-Version: 13\r\n\
         Origin: null\r\n\
         \r\n"
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|error| format!("webOS websocket handshake write failed: {error}"))?;

    let mut response = Vec::new();
    let mut buffer = [0u8; 256];
    while response.len() < 4096 {
        let size = stream
            .read(&mut buffer)
            .map_err(|error| format!("webOS websocket handshake failed: {error}"))?;
        if size == 0 {
            break;
        }
        response.extend_from_slice(&buffer[..size]);
        if response.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }

    let response_text = String::from_utf8_lossy(&response);
    if response_text.starts_with("HTTP/1.1 101") || response_text.starts_with("HTTP/1.0 101") {
        Ok(())
    } else {
        Err("webOS websocket upgrade rejected".to_string())
    }
}

fn register_webos_client(
    stream: &mut dyn ReadWrite,
    existing_key: Option<&str>,
) -> std::result::Result<Option<String>, String> {
    let permissions = vec![
        "CONTROL_POWER",
        "CONTROL_DISPLAY",
        "READ_POWER_STATE",
        "READ_RUNNING_APPS",
        "WRITE_NOTIFICATION_TOAST",
    ];

    let mut payload = json!({
        "pairingType": "PROMPT",
        "manifest": {
            "manifestVersion": 1,
            "appVersion": "1.0",
            "signed": {
                "created": "20260523",
                "appId": "com.local.tvguardtray",
                "vendorId": "com.local",
                "localizedAppNames": { "": APP_NAME },
                "localizedVendorNames": { "": APP_NAME },
                "permissions": permissions,
                "serial": "tvguardtray"
            },
            "permissions": permissions,
            "signatures": [
                {
                    "signatureVersion": 1,
                    "signature": "tvguardtray"
                }
            ]
        }
    });

    if let Some(key) = existing_key {
        payload["client-key"] = Value::String(key.to_string());
    }

    let request = json!({
        "id": "register_0",
        "type": "register",
        "payload": payload
    });
    send_ws_text(stream, &request.to_string())?;

    for _ in 0..20 {
        let message = read_ws_text(stream)?;
        let Ok(value) = serde_json::from_str::<Value>(&message) else {
            continue;
        };

        if value.get("type").and_then(Value::as_str) == Some("registered") {
            let key = value
                .get("payload")
                .and_then(|payload| payload.get("client-key"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| existing_key.map(str::to_string));
            return Ok(key);
        }

        if value.get("type").and_then(Value::as_str) == Some("error") {
            return Err(format!("webOS registration error: {value}"));
        }
    }

    Err("webOS registration timed out; approve the pairing prompt on the TV".to_string())
}

fn send_ws_text(stream: &mut dyn ReadWrite, text: &str) -> std::result::Result<(), String> {
    let payload = text.as_bytes();
    let mut frame = Vec::with_capacity(payload.len() + 16);
    frame.push(0x81);

    if payload.len() <= 125 {
        frame.push(0x80 | payload.len() as u8);
    } else if payload.len() <= u16::MAX as usize {
        frame.push(0x80 | 126);
        frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    } else {
        frame.push(0x80 | 127);
        frame.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    }

    let mask = websocket_mask();
    frame.extend_from_slice(&mask);
    for (idx, byte) in payload.iter().enumerate() {
        frame.push(byte ^ mask[idx % 4]);
    }

    stream
        .write_all(&frame)
        .map_err(|error| format!("webOS websocket send failed: {error}"))
}

fn read_ws_text(stream: &mut dyn ReadWrite) -> std::result::Result<String, String> {
    loop {
        let mut header = [0u8; 2];
        stream
            .read_exact(&mut header)
            .map_err(|error| format!("webOS websocket read failed: {error}"))?;

        let opcode = header[0] & 0x0f;
        let masked = header[1] & 0x80 != 0;
        let mut len = (header[1] & 0x7f) as u64;

        if len == 126 {
            let mut ext = [0u8; 2];
            stream
                .read_exact(&mut ext)
                .map_err(|error| format!("webOS websocket length read failed: {error}"))?;
            len = u16::from_be_bytes(ext) as u64;
        } else if len == 127 {
            let mut ext = [0u8; 8];
            stream
                .read_exact(&mut ext)
                .map_err(|error| format!("webOS websocket length read failed: {error}"))?;
            len = u64::from_be_bytes(ext);
        }

        let mut mask = [0u8; 4];
        if masked {
            stream
                .read_exact(&mut mask)
                .map_err(|error| format!("webOS websocket mask read failed: {error}"))?;
        }

        let mut payload = vec![0u8; len as usize];
        stream
            .read_exact(&mut payload)
            .map_err(|error| format!("webOS websocket payload read failed: {error}"))?;

        if masked {
            for (idx, byte) in payload.iter_mut().enumerate() {
                *byte ^= mask[idx % 4];
            }
        }

        match opcode {
            0x1 => {
                return String::from_utf8(payload)
                    .map_err(|error| format!("webOS websocket text invalid: {error}"))
            }
            0x8 => return Err("webOS websocket closed".to_string()),
            0x9 | 0xA => continue,
            _ => continue,
        }
    }
}

fn websocket_mask() -> [u8; 4] {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    [
        (nanos & 0xff) as u8,
        ((nanos >> 8) & 0xff) as u8,
        ((nanos >> 16) & 0xff) as u8,
        ((nanos >> 24) & 0xff) as u8,
    ]
}

fn save_webos_client_key(client_key: &str) {
    save_config_value("WebOsClientKey", client_key);
}

fn save_config_value(key: &str, value: &str) {
    let base_dir = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    let path = base_dir.join(CONFIG_FILE_NAME);

    let contents = std::fs::read_to_string(&path).unwrap_or_default();
    let mut found = false;
    let mut lines = Vec::new();
    let key_lower = key.to_ascii_lowercase();

    for line in contents.lines() {
        if line
            .trim_start()
            .to_ascii_lowercase()
            .starts_with(&format!("{key_lower}="))
        {
            lines.push(format!("{key}={value}"));
            found = true;
        } else {
            lines.push(line.to_string());
        }
    }

    if !found {
        lines.push(format!("{key}={value}"));
    }

    let _ = std::fs::write(path, format!("{}\r\n", lines.join("\r\n")));
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
                read_webos_power(host, *port, Duration::from_millis(2500)),
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
    match list_audio_output_names() {
        Ok(names) if !names.is_empty() => {
            for name in &names {
                log_message(&format!("Audio endpoint: {name}"));
            }

            if let Some(selected) =
                choose_tv_audio_endpoint(&names, config.tv_audio_device_name_contains.as_deref())
            {
                save_config_value("TvAudioDeviceNameContains", &selected);
                if let Some(state) = APP.get() {
                    if let Ok(mut state) = state.lock() {
                        state.config.tv_audio_device_name_contains = Some(selected.clone());
                    }
                }
                notes.push(format!("TV audio endpoint: {selected}"));
            } else {
                notes.push(format!("Audio endpoints found: {}", names.join(", ")));
            }
        }
        Ok(_) => notes.push("Audio endpoint scan found no active output devices".to_string()),
        Err(error) => {
            notes.push(format!("Audio endpoint scan failed: {error}"));
            log_message(&format!("Audio endpoint scan failed: {error}"));
        }
    }

    Ok(format!("Onboarding completed.\n\n{}", notes.join("\n")))
}

fn pair_webos_tv(
    host: &str,
    port: u16,
    existing_key: Option<&str>,
) -> std::result::Result<Option<String>, String> {
    let timeout = if existing_key.is_some() {
        Duration::from_secs(5)
    } else {
        Duration::from_secs(30)
    };
    let mut stream = connect_webos_socket(host, port, timeout)?;
    register_webos_client(&mut *stream, existing_key)
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

fn choose_tv_audio_endpoint(names: &[String], preferred: Option<&str>) -> Option<String> {
    if let Some(preferred) = preferred {
        if !preferred.trim().is_empty() {
            if let Some(name) = names.iter().find(|name| {
                name.to_ascii_lowercase()
                    .contains(&preferred.to_ascii_lowercase())
            }) {
                return Some(name.clone());
            }
        }
    }

    for marker in [
        "LG",
        "TV",
        "NVIDIA High Definition Audio",
        "HDMI",
        "Digital Audio",
    ] {
        let marker = marker.to_ascii_lowercase();
        if let Some(name) = names
            .iter()
            .find(|name| name.to_ascii_lowercase().contains(&marker))
        {
            return Some(name.clone());
        }
    }

    names.first().cloned()
}

fn list_audio_output_names() -> std::result::Result<Vec<String>, String> {
    let script = r#"
Add-Type -TypeDefinition @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;

public enum EDataFlow { eRender, eCapture, eAll }
[Flags] public enum DeviceState : uint { Active = 0x00000001 }

[ComImport, Guid("BCDE0395-E52F-467C-8E3D-C4579291692E")]
public class MMDeviceEnumerator {}

[ComImport, Guid("A95664D2-9614-4F35-A746-DE8DB63617E6"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IMMDeviceEnumerator {
    int EnumAudioEndpoints(EDataFlow dataFlow, DeviceState dwStateMask, out IMMDeviceCollection ppDevices);
}

[ComImport, Guid("0BD7A1BE-7A1A-44DB-8397-C0E9422C1607"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IMMDeviceCollection {
    int GetCount(out uint pcDevices);
    int Item(uint nDevice, out IMMDevice ppDevice);
}

[ComImport, Guid("D666063F-1587-4E43-81F1-B948E807363F"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IMMDevice {
    int Activate(ref Guid iid, uint dwClsCtx, IntPtr pActivationParams, out IntPtr ppInterface);
    int OpenPropertyStore(uint stgmAccess, out IPropertyStore ppProperties);
    int GetId([MarshalAs(UnmanagedType.LPWStr)] out string ppstrId);
    int GetState(out uint pdwState);
}

[ComImport, Guid("886D8EEB-8CF2-4446-8D02-CDBA1DBDCF99"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IPropertyStore {
    int GetCount(out uint cProps);
    int GetAt(uint iProp, out PROPERTYKEY pkey);
    int GetValue(ref PROPERTYKEY key, out PROPVARIANT pv);
    int SetValue(ref PROPERTYKEY key, ref PROPVARIANT propvar);
    int Commit();
}

[StructLayout(LayoutKind.Sequential)]
public struct PROPERTYKEY { public Guid fmtid; public uint pid; }

[StructLayout(LayoutKind.Sequential)]
public struct PROPVARIANT { public ushort vt; public ushort w1; public ushort w2; public ushort w3; public IntPtr p; public int p2; }

public class AudioEndpointLister {
    static PROPERTYKEY PKEY_Device_FriendlyName = new PROPERTYKEY {
        fmtid = new Guid("a45c254e-df1c-4efd-8020-67d146a850e0"),
        pid = 14
    };

    static string GetString(IPropertyStore store, PROPERTYKEY key) {
        PROPVARIANT pv;
        store.GetValue(ref key, out pv);
        if (pv.vt == 31 && pv.p != IntPtr.Zero) return Marshal.PtrToStringUni(pv.p);
        return "";
    }

    public static string[] ListRenderNames() {
        var names = new List<string>();
        var enumerator = (IMMDeviceEnumerator)new MMDeviceEnumerator();
        IMMDeviceCollection devices;
        Marshal.ThrowExceptionForHR(enumerator.EnumAudioEndpoints(EDataFlow.eRender, DeviceState.Active, out devices));

        uint count;
        Marshal.ThrowExceptionForHR(devices.GetCount(out count));

        for (uint i = 0; i < count; i++) {
            IMMDevice device;
            Marshal.ThrowExceptionForHR(devices.Item(i, out device));

            IPropertyStore store;
            Marshal.ThrowExceptionForHR(device.OpenPropertyStore(0, out store));
            string name = GetString(store, PKEY_Device_FriendlyName);
            if (!String.IsNullOrWhiteSpace(name)) names.Add(name);
        }

        return names.ToArray();
    }
}
"@

[AudioEndpointLister]::ListRenderNames()
"#;

    let output = Command::new("powershell.exe")
        .arg("-NoProfile")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-Command")
        .arg(script)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|error| format!("failed to start PowerShell: {error}"))?;

    if output.status.success() {
        let names = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect();
        Ok(names)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            Err(format!("PowerShell exited with {}", output.status))
        } else {
            Err(stderr)
        }
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

fn parse_mac_address(value: &str) -> Option<[u8; 6]> {
    let cleaned: String = value.chars().filter(|ch| ch.is_ascii_hexdigit()).collect();
    if cleaned.len() != 12 {
        return None;
    }

    let mut mac = [0u8; 6];
    for idx in 0..6 {
        let part = &cleaned[idx * 2..idx * 2 + 2];
        mac[idx] = u8::from_str_radix(part, 16).ok()?;
    }
    Some(mac)
}

fn apply_pc_mode_from_state() {
    match run_embedded_display_config_script(embedded_pc_mode_script()) {
        Ok(()) => set_status("PC mode applied"),
        Err(script_error) => {
            log_message(&format!(
                "Embedded PC mode failed, trying native fallback: {script_error}"
            ));
            match apply_pc_mode_native() {
                Ok(()) => set_status("PC mode applied with native DisplayConfig"),
                Err(error) => {
                    log_message(&format!("Native DisplayConfig apply failed, trying ChangeDisplaySettingsEx fallback: {error}"));
                    match apply_pc_mode_change_display_settings() {
                        Ok(()) => set_status("PC mode applied with ChangeDisplaySettingsEx"),
                        Err(fallback_error) => {
                            set_status(format!("Failed to apply PC mode: {fallback_error}"))
                        }
                    }
                }
            }
        }
    }
}

fn apply_tv_mode_from_state() {
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

    match set_default_audio_output_by_name(name_filter) {
        Ok(device_name) => {
            set_status(format!("Default audio output set to {device_name}"));
            if config.try_enable_dolby_atmos {
                set_status("Dolby Atmos auto-enable is not implemented; set it once in Windows spatial sound settings");
            }
        }
        Err(error) => set_status(format!("Failed to switch TV audio output: {error}")),
    }
}

fn set_default_audio_output_by_name(name_filter: &str) -> std::result::Result<String, String> {
    let script = r#"
param([string] $NameFilter)

Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;

public enum EDataFlow { eRender, eCapture, eAll }
public enum ERole { eConsole, eMultimedia, eCommunications }
[Flags] public enum DeviceState : uint { Active = 0x00000001 }

[ComImport, Guid("BCDE0395-E52F-467C-8E3D-C4579291692E")]
public class MMDeviceEnumerator {}

[ComImport, Guid("A95664D2-9614-4F35-A746-DE8DB63617E6"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IMMDeviceEnumerator {
    int EnumAudioEndpoints(EDataFlow dataFlow, DeviceState dwStateMask, out IMMDeviceCollection ppDevices);
}

[ComImport, Guid("0BD7A1BE-7A1A-44DB-8397-C0E9422C1607"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IMMDeviceCollection {
    int GetCount(out uint pcDevices);
    int Item(uint nDevice, out IMMDevice ppDevice);
}

[ComImport, Guid("D666063F-1587-4E43-81F1-B948E807363F"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IMMDevice {
    int Activate(ref Guid iid, uint dwClsCtx, IntPtr pActivationParams, out IntPtr ppInterface);
    int OpenPropertyStore(uint stgmAccess, out IPropertyStore ppProperties);
    int GetId([MarshalAs(UnmanagedType.LPWStr)] out string ppstrId);
    int GetState(out uint pdwState);
}

[ComImport, Guid("886D8EEB-8CF2-4446-8D02-CDBA1DBDCF99"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IPropertyStore {
    int GetCount(out uint cProps);
    int GetAt(uint iProp, out PROPERTYKEY pkey);
    int GetValue(ref PROPERTYKEY key, out PROPVARIANT pv);
    int SetValue(ref PROPERTYKEY key, ref PROPVARIANT propvar);
    int Commit();
}

[StructLayout(LayoutKind.Sequential)]
public struct PROPERTYKEY { public Guid fmtid; public uint pid; }

[StructLayout(LayoutKind.Sequential)]
public struct PROPVARIANT { public ushort vt; public ushort w1; public ushort w2; public ushort w3; public IntPtr p; public int p2; }

[ComImport, Guid("870AF99C-171D-4F9E-AF0D-E63DF40C2BC9")]
public class PolicyConfigClient {}

[ComImport, Guid("F8679F50-850A-41CF-9C72-430F290290C8"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IPolicyConfig {
    int GetMixFormat();
    int GetDeviceFormat();
    int SetDeviceFormat();
    int GetProcessingPeriod();
    int SetProcessingPeriod();
    int GetShareMode();
    int SetShareMode();
    int GetPropertyValue();
    int SetPropertyValue();
    int SetDefaultEndpoint([MarshalAs(UnmanagedType.LPWStr)] string wszDeviceId, ERole role);
    int SetEndpointVisibility();
}

public class AudioSwitcher {
    static PROPERTYKEY PKEY_Device_FriendlyName = new PROPERTYKEY {
        fmtid = new Guid("a45c254e-df1c-4efd-8020-67d146a850e0"),
        pid = 14
    };

    static string GetString(IPropertyStore store, PROPERTYKEY key) {
        PROPVARIANT pv;
        store.GetValue(ref key, out pv);
        if (pv.vt == 31 && pv.p != IntPtr.Zero) return Marshal.PtrToStringUni(pv.p);
        return "";
    }

    public static string SetDefaultByName(string nameFilter) {
        var enumerator = (IMMDeviceEnumerator)new MMDeviceEnumerator();
        IMMDeviceCollection devices;
        Marshal.ThrowExceptionForHR(enumerator.EnumAudioEndpoints(EDataFlow.eRender, DeviceState.Active, out devices));

        uint count;
        Marshal.ThrowExceptionForHR(devices.GetCount(out count));

        string bestId = null;
        string bestName = null;

        for (uint i = 0; i < count; i++) {
            IMMDevice device;
            Marshal.ThrowExceptionForHR(devices.Item(i, out device));

            string id;
            Marshal.ThrowExceptionForHR(device.GetId(out id));

            IPropertyStore store;
            Marshal.ThrowExceptionForHR(device.OpenPropertyStore(0, out store));
            string name = GetString(store, PKEY_Device_FriendlyName);

            if (!String.IsNullOrWhiteSpace(name) && name.IndexOf(nameFilter, StringComparison.OrdinalIgnoreCase) >= 0) {
                bestId = id;
                bestName = name;
                break;
            }
        }

        if (bestId == null) throw new InvalidOperationException("No active render endpoint matched: " + nameFilter);

        var policy = (IPolicyConfig)new PolicyConfigClient();
        Marshal.ThrowExceptionForHR(policy.SetDefaultEndpoint(bestId, ERole.eConsole));
        Marshal.ThrowExceptionForHR(policy.SetDefaultEndpoint(bestId, ERole.eMultimedia));
        Marshal.ThrowExceptionForHR(policy.SetDefaultEndpoint(bestId, ERole.eCommunications));
        return bestName;
    }
}
"@

[AudioSwitcher]::SetDefaultByName($NameFilter)
"#;

    let output = Command::new("powershell.exe")
        .arg("-NoProfile")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-Command")
        .arg(script)
        .arg("-NameFilter")
        .arg(name_filter)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|error| format!("failed to start PowerShell: {error}"))?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if stdout.is_empty() {
            Ok(name_filter.to_string())
        } else {
            Ok(stdout)
        }
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            Err(format!("PowerShell exited with {}", output.status))
        } else {
            Err(stderr)
        }
    }
}

fn embedded_tv_mode_script() -> &'static str {
    r#"
Import-Module DisplayConfig -ErrorAction Stop
Enable-Display -DisplayId 1,2,3
Get-DisplayConfig |
    Set-DisplayPrimary -DisplayId 3 |
    Use-DisplayConfig
"#
}

fn embedded_pc_mode_script() -> &'static str {
    r#"
Import-Module DisplayConfig -ErrorAction Stop
Enable-Display -DisplayId 1,2 -DisplayIdToDisable 3
Get-DisplayConfig |
    Set-DisplayPrimary -DisplayId 2 |
    Use-DisplayConfig
"#
}

fn run_embedded_display_config_script(script: &str) -> std::result::Result<(), String> {
    let output = Command::new("powershell.exe")
        .arg("-NoProfile")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-Command")
        .arg(script)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|error| format!("failed to start PowerShell: {error}"))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            format!("PowerShell exited with {}", output.status)
        };
        Err(detail)
    }
}

fn apply_pc_mode_change_display_settings() -> std::result::Result<(), String> {
    unsafe {
        set_display_position(PRIMARY_DEVICE_NAME, 0, 0)?;

        match current_devmode(r"\\.\DISPLAY1") {
            Ok(display1) => {
                if let Err(error) =
                    set_display_position(r"\\.\DISPLAY1", -(display1.dmPelsWidth as i32), 0)
                {
                    log_message(&format!("DISPLAY1 reposition skipped: {error}"));
                }
            }
            Err(error) => log_message(&format!(
                "DISPLAY1 current mode not available, skipped: {error}"
            )),
        }

        disable_display(TV_DEVICE_NAME)?;

        let apply_rc = ChangeDisplaySettingsExW(
            PCWSTR::null(),
            None,
            HWND::default(),
            CDS_UPDATEREGISTRY,
            None,
        );
        if apply_rc != DISP_CHANGE_SUCCESSFUL {
            return Err(format!(
                "final ChangeDisplaySettingsEx apply failed: {}",
                apply_rc.0
            ));
        }
    }

    Ok(())
}

unsafe fn current_devmode(device_name: &str) -> std::result::Result<DEVMODEW, String> {
    let mut mode = DEVMODEW::default();
    mode.dmSize = size_of::<DEVMODEW>() as u16;
    let device_name = wide(device_name);
    if EnumDisplaySettingsW(
        PCWSTR(device_name.as_ptr()),
        ENUM_CURRENT_SETTINGS,
        &mut mode,
    )
    .as_bool()
    {
        Ok(mode)
    } else if EnumDisplaySettingsW(
        PCWSTR(device_name.as_ptr()),
        ENUM_REGISTRY_SETTINGS,
        &mut mode,
    )
    .as_bool()
    {
        Ok(mode)
    } else {
        Err(format!("EnumDisplaySettingsW failed for {device_name:?}"))
    }
}

unsafe fn set_display_position(
    device_name: &str,
    x: i32,
    y: i32,
) -> std::result::Result<(), String> {
    let mut mode = current_devmode(device_name)?;
    mode.dmFields = DM_POSITION | DM_PELSWIDTH | DM_PELSHEIGHT;
    mode.Anonymous1.Anonymous2.dmPosition.x = x;
    mode.Anonymous1.Anonymous2.dmPosition.y = y;

    let wide_name = wide(device_name);
    let rc = ChangeDisplaySettingsExW(
        PCWSTR(wide_name.as_ptr()),
        Some(&mode),
        HWND::default(),
        CDS_UPDATEREGISTRY | CDS_NORESET,
        None,
    );
    if rc == DISP_CHANGE_SUCCESSFUL {
        Ok(())
    } else {
        Err(format!(
            "ChangeDisplaySettingsEx position failed for {device_name}: {}",
            rc.0
        ))
    }
}

unsafe fn disable_display(device_name: &str) -> std::result::Result<(), String> {
    let mut mode = DEVMODEW::default();
    mode.dmSize = size_of::<DEVMODEW>() as u16;
    mode.dmFields = DM_POSITION | DM_PELSWIDTH | DM_PELSHEIGHT;
    mode.Anonymous1.Anonymous2.dmPosition.x = 0;
    mode.Anonymous1.Anonymous2.dmPosition.y = 0;
    mode.dmPelsWidth = 0;
    mode.dmPelsHeight = 0;

    let wide_name = wide(device_name);
    let rc = ChangeDisplaySettingsExW(
        PCWSTR(wide_name.as_ptr()),
        Some(&mode),
        HWND::default(),
        CDS_UPDATEREGISTRY | CDS_NORESET,
        None,
    );
    if rc == DISP_CHANGE_SUCCESSFUL {
        Ok(())
    } else {
        Err(format!(
            "ChangeDisplaySettingsEx disable failed for {device_name}: {}",
            rc.0
        ))
    }
}

fn apply_pc_mode_native() -> Result<()> {
    unsafe {
        let (mut paths, modes) = query_active_display_config()?;

        let mut kept_paths = Vec::new();
        for path in paths.drain(..) {
            let name = source_device_name(&path)?;
            if !name.eq_ignore_ascii_case(TV_DEVICE_NAME) {
                kept_paths.push(path);
            }
        }

        if kept_paths.is_empty() {
            return Err(Error::from_win32());
        }

        let mut kept_paths = kept_paths;
        let mut modes = remap_modes_for_paths(&mut kept_paths, &modes);
        set_primary_position(&kept_paths, &mut modes, PRIMARY_DEVICE_NAME)?;

        let flags = SDC_APPLY
            | SDC_USE_SUPPLIED_DISPLAY_CONFIG
            | SDC_SAVE_TO_DATABASE
            | SDC_ALLOW_CHANGES
            | SDC_PATH_PERSIST_IF_REQUIRED;
        let rc = SetDisplayConfig(Some(&kept_paths), Some(&modes), flags);
        if rc == 0 {
            Ok(())
        } else {
            log_message(&format!("SetDisplayConfig failed with code {}", rc));
            Err(Error::from_win32())
        }
    }
}

unsafe fn query_active_display_config(
) -> Result<(Vec<DISPLAYCONFIG_PATH_INFO>, Vec<DISPLAYCONFIG_MODE_INFO>)> {
    let mut path_count = 0;
    let mut mode_count = 0;
    let size_rc =
        GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count);
    if size_rc.0 != 0 {
        return Err(Error::from_win32());
    }

    loop {
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); mode_count as usize];
        let mut actual_path_count = path_count;
        let mut actual_mode_count = mode_count;

        let query_rc = QueryDisplayConfig(
            QDC_ONLY_ACTIVE_PATHS,
            &mut actual_path_count,
            paths.as_mut_ptr(),
            &mut actual_mode_count,
            modes.as_mut_ptr(),
            None,
        );

        if query_rc.0 == 0 {
            paths.truncate(actual_path_count as usize);
            modes.truncate(actual_mode_count as usize);
            return Ok((paths, modes));
        }

        let resize_rc =
            GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count);
        if resize_rc.0 != 0 {
            return Err(Error::from_win32());
        }
    }
}

unsafe fn source_device_name(path: &DISPLAYCONFIG_PATH_INFO) -> Result<String> {
    let mut source_name = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
    source_name.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
    source_name.header.size = size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
    source_name.header.adapterId = path.sourceInfo.adapterId;
    source_name.header.id = path.sourceInfo.id;

    let rc = DisplayConfigGetDeviceInfo(&mut source_name.header);
    if rc == 0 {
        Ok(string_from_wide_until_nul(&source_name.viewGdiDeviceName))
    } else {
        Err(Error::from_win32())
    }
}

unsafe fn remap_modes_for_paths(
    paths: &mut [DISPLAYCONFIG_PATH_INFO],
    modes: &[DISPLAYCONFIG_MODE_INFO],
) -> Vec<DISPLAYCONFIG_MODE_INFO> {
    let mut remap = HashMap::<u32, u32>::new();
    let mut new_modes = Vec::<DISPLAYCONFIG_MODE_INFO>::new();

    for path in paths.iter() {
        let source_idx = path.sourceInfo.Anonymous.modeInfoIdx;
        if source_idx != u32::MAX
            && (source_idx as usize) < modes.len()
            && !remap.contains_key(&source_idx)
        {
            remap.insert(source_idx, new_modes.len() as u32);
            new_modes.push(modes[source_idx as usize]);
        }

        let target_idx = path.targetInfo.Anonymous.modeInfoIdx;
        if target_idx != u32::MAX
            && (target_idx as usize) < modes.len()
            && !remap.contains_key(&target_idx)
        {
            remap.insert(target_idx, new_modes.len() as u32);
            new_modes.push(modes[target_idx as usize]);
        }
    }

    for path in paths.iter_mut() {
        let source_idx = path.sourceInfo.Anonymous.modeInfoIdx;
        if let Some(new_idx) = remap.get(&source_idx) {
            path.sourceInfo.Anonymous.modeInfoIdx = *new_idx;
        }

        let target_idx = path.targetInfo.Anonymous.modeInfoIdx;
        if let Some(new_idx) = remap.get(&target_idx) {
            path.targetInfo.Anonymous.modeInfoIdx = *new_idx;
        }
    }

    new_modes
}

unsafe fn set_primary_position(
    paths: &[DISPLAYCONFIG_PATH_INFO],
    modes: &mut [DISPLAYCONFIG_MODE_INFO],
    primary_device_name: &str,
) -> Result<()> {
    let mut ordered: Vec<(String, usize, u32)> = Vec::new();

    for path in paths {
        let name = source_device_name(path)?;
        let mode_idx = path.sourceInfo.Anonymous.modeInfoIdx;
        if mode_idx == u32::MAX {
            continue;
        }
        ordered.push((name, mode_idx as usize, path.sourceInfo.id));
    }

    ordered.sort_by_key(|(_, _, source_id)| *source_id);

    let primary_pos = ordered
        .iter()
        .position(|(name, _, _)| name.eq_ignore_ascii_case(primary_device_name))
        .unwrap_or(0);

    let mut next_left_x = 0i32;
    for (idx, (_, mode_idx, _)) in ordered.iter().enumerate() {
        if *mode_idx >= modes.len()
            || modes[*mode_idx].infoType != DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE
        {
            continue;
        }

        let source_mode = &mut modes[*mode_idx].Anonymous.sourceMode;
        if idx == primary_pos {
            source_mode.position.x = 0;
            source_mode.position.y = 0;
        } else {
            next_left_x -= source_mode.width as i32;
            source_mode.position.x = next_left_x;
            source_mode.position.y = 0;
        }
    }

    Ok(())
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
        .and_then(|state| {
            state.lock().ok().map(|state| AppConfig {
                webos_host: state.config.webos_host.clone(),
                webos_port: state.config.webos_port,
                webos_timeout: state.config.webos_timeout,
                auto_apply_pc_mode: state.config.auto_apply_pc_mode,
                auto_switch_displays: state.config.auto_switch_displays,
                tv_mac: state.config.tv_mac,
                wake_broadcast: state.config.wake_broadcast.clone(),
                wake_port: state.config.wake_port,
                webos_client_key: state.config.webos_client_key.clone(),
                auto_switch_audio: state.config.auto_switch_audio,
                tv_audio_device_name_contains: state.config.tv_audio_device_name_contains.clone(),
                try_enable_dolby_atmos: state.config.try_enable_dolby_atmos,
            })
        })
        .unwrap_or(AppConfig {
            webos_host: Some("lgwebostv".to_string()),
            webos_port: DEFAULT_WEBOS_PORT,
            webos_timeout: Duration::from_millis(DEFAULT_WEBOS_TIMEOUT_MS),
            auto_apply_pc_mode: false,
            auto_switch_displays: false,
            tv_mac: None,
            wake_broadcast: "192.168.0.255".to_string(),
            wake_port: 9,
            webos_client_key: None,
            auto_switch_audio: true,
            tv_audio_device_name_contains: Some("NVIDIA High Definition Audio".to_string()),
            try_enable_dolby_atmos: false,
        });

    if let Some(host) = config.webos_host.as_deref() {
        return Ok(read_webos_power(
            host,
            config.webos_port,
            config.webos_timeout,
        ));
    }

    if is_display_active(TV_DEVICE_NAME) {
        read_ddc_power(TV_DEVICE_NAME)
    } else {
        Ok(TvPower::OutputInactive)
    }
}

fn is_display_active(device_name: &str) -> bool {
    let mut found = false;

    unsafe {
        let mut context = ActiveDisplayContext {
            target_device: device_name,
            found: &mut found,
        };
        let context_ptr = &mut context as *mut ActiveDisplayContext as isize;
        let _ = EnumDisplayMonitors(
            HDC::default(),
            None,
            Some(enum_active_display_proc),
            LPARAM(context_ptr),
        );
    }

    found
}

struct ActiveDisplayContext<'a> {
    target_device: &'a str,
    found: &'a mut bool,
}

unsafe extern "system" fn enum_active_display_proc(
    hmonitor: HMONITOR,
    _hdc: HDC,
    _rect: *mut windows::Win32::Foundation::RECT,
    data: LPARAM,
) -> BOOL {
    let context = &mut *(data.0 as *mut ActiveDisplayContext);

    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;

    if GetMonitorInfoW(hmonitor, &mut info as *mut MONITORINFOEXW as *mut _).as_bool() {
        let device_name = string_from_wide_until_nul(&info.szDevice);
        if device_name.eq_ignore_ascii_case(context.target_device) {
            *context.found = true;
            return false.into();
        }
    }

    true.into()
}

fn read_webos_power(host: &str, port: u16, timeout: Duration) -> TvPower {
    let address = format!("{host}:{port}");
    let Ok(mut addrs) = address.to_socket_addrs() else {
        return TvPower::NotOn {
            code: None,
            reason: format!("webOS host not found: {host}"),
        };
    };

    let Some(socket_addr) = addrs.next() else {
        return TvPower::NotOn {
            code: None,
            reason: format!("webOS host has no address: {host}"),
        };
    };

    let mut stream = match TcpStream::connect_timeout(&socket_addr, timeout) {
        Ok(stream) => {
            if port == 3001 {
                return read_webos_power_tls(host, port, timeout, stream);
            }
            stream
        }
        Err(error) => {
            return TvPower::NotOn {
                code: None,
                reason: format!("webOS port {port} unreachable: {error}"),
            };
        }
    };

    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));

    let request = format!(
        "GET / HTTP/1.1\r\n\
         Host: {host}:{port}\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
         Sec-WebSocket-Version: 13\r\n\
         Origin: null\r\n\
         \r\n"
    );

    if let Err(error) = stream.write_all(request.as_bytes()) {
        return TvPower::NotOn {
            code: None,
            reason: format!("webOS handshake write failed: {error}"),
        };
    }

    let mut response = [0u8; 512];
    match stream.read(&mut response) {
        Ok(size) if size > 0 => {
            let response_text = String::from_utf8_lossy(&response[..size]);
            if response_text.starts_with("HTTP/1.1 101")
                || response_text.starts_with("HTTP/1.0 101")
            {
                TvPower::On
            } else {
                TvPower::NotOn {
                    code: None,
                    reason: "webOS websocket upgrade rejected".to_string(),
                }
            }
        }
        Ok(_) => TvPower::NotOn {
            code: None,
            reason: "webOS handshake returned no data".to_string(),
        },
        Err(error) => TvPower::NotOn {
            code: None,
            reason: format!("webOS handshake timed out or failed: {error}"),
        },
    }
}

fn read_webos_power_tls(host: &str, port: u16, timeout: Duration, stream: TcpStream) -> TvPower {
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));

    let connector = match TlsConnector::builder()
        .danger_accept_invalid_certs(true)
        .danger_accept_invalid_hostnames(true)
        .build()
    {
        Ok(connector) => connector,
        Err(error) => {
            return TvPower::NotOn {
                code: None,
                reason: format!("webOS TLS setup failed: {error}"),
            };
        }
    };

    let mut tls = match connector.connect(host, stream) {
        Ok(tls) => tls,
        Err(error) => {
            return TvPower::NotOn {
                code: None,
                reason: format!("webOS TLS handshake failed: {error}"),
            };
        }
    };

    let request = format!(
        "GET / HTTP/1.1\r\n\
         Host: {host}:{port}\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
         Sec-WebSocket-Version: 13\r\n\
         Origin: null\r\n\
         \r\n"
    );

    if let Err(error) = tls.write_all(request.as_bytes()) {
        return TvPower::NotOn {
            code: None,
            reason: format!("webOS TLS handshake write failed: {error}"),
        };
    }

    let mut response = [0u8; 512];
    match tls.read(&mut response) {
        Ok(size) if size > 0 => {
            let response_text = String::from_utf8_lossy(&response[..size]);
            if response_text.starts_with("HTTP/1.1 101")
                || response_text.starts_with("HTTP/1.0 101")
            {
                TvPower::On
            } else {
                TvPower::NotOn {
                    code: None,
                    reason: "webOS TLS websocket upgrade rejected".to_string(),
                }
            }
        }
        Ok(_) => TvPower::NotOn {
            code: None,
            reason: "webOS TLS handshake returned no data".to_string(),
        },
        Err(error) => TvPower::NotOn {
            code: None,
            reason: format!("webOS TLS handshake timed out or failed: {error}"),
        },
    }
}

fn read_ddc_power(device_name: &str) -> Result<TvPower> {
    let mut matches = Vec::<MonitorPower>::new();

    unsafe {
        let mut context = EnumContext {
            target_device: device_name,
            matches: &mut matches,
        };
        let context_ptr = &mut context as *mut EnumContext as isize;
        let _ = EnumDisplayMonitors(
            HDC::default(),
            None,
            Some(enum_monitor_proc),
            LPARAM(context_ptr),
        );
    }

    let Some(power) = matches.into_iter().next() else {
        return Ok(TvPower::OutputInactive);
    };

    if !power.ddc_ok {
        return Ok(TvPower::NotOn {
            code: None,
            reason: power.error.unwrap_or_else(|| "no DDC response".to_string()),
        });
    }

    match power.power_code {
        1 => Ok(TvPower::On),
        code => Ok(TvPower::NotOn {
            code: Some(code),
            reason: power_state_name(code).to_string(),
        }),
    }
}

struct EnumContext<'a> {
    target_device: &'a str,
    matches: &'a mut Vec<MonitorPower>,
}

#[derive(Debug)]
struct MonitorPower {
    ddc_ok: bool,
    power_code: u32,
    error: Option<String>,
}

unsafe extern "system" fn enum_monitor_proc(
    hmonitor: HMONITOR,
    _hdc: HDC,
    _rect: *mut windows::Win32::Foundation::RECT,
    data: LPARAM,
) -> BOOL {
    let context = &mut *(data.0 as *mut EnumContext);

    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;

    if !GetMonitorInfoW(hmonitor, &mut info as *mut MONITORINFOEXW as *mut _).as_bool() {
        return true.into();
    }

    let device_name = string_from_wide_until_nul(&info.szDevice);
    if !device_name.eq_ignore_ascii_case(context.target_device) {
        return true.into();
    }

    let mut count = 0;
    if GetNumberOfPhysicalMonitorsFromHMONITOR(hmonitor, &mut count).is_err() || count == 0 {
        context.matches.push(MonitorPower {
            ddc_ok: false,
            power_code: 0,
            error: Some("no physical monitor handle".to_string()),
        });
        return true.into();
    }

    let mut physical = vec![PHYSICAL_MONITOR::default(); count as usize];
    if GetPhysicalMonitorsFromHMONITOR(hmonitor, &mut physical).is_err() {
        context.matches.push(MonitorPower {
            ddc_ok: false,
            power_code: 0,
            error: Some("failed to open physical monitor handle".to_string()),
        });
        return true.into();
    }

    for monitor in &physical {
        let mut feature_type = MC_VCP_CODE_TYPE::default();
        let mut current = 0;
        let mut maximum = 0;
        let ok = GetVCPFeatureAndVCPFeatureReply(
            monitor.hPhysicalMonitor,
            0xD6,
            Some(&mut feature_type),
            &mut current,
            Some(&mut maximum),
        ) != 0;

        context.matches.push(MonitorPower {
            ddc_ok: ok,
            power_code: current,
            error: (!ok).then(|| "VCP D6 power query failed".to_string()),
        });
    }

    let _ = DestroyPhysicalMonitors(&physical);
    true.into()
}

fn power_state_name(code: u32) -> &'static str {
    match code {
        1 => "on",
        2 => "standby",
        3 => "suspend",
        4 => "off",
        _ => "unknown power state",
    }
}

unsafe fn add_tray_icon(hwnd: HWND) -> Result<()> {
    let icon = LoadIconW(None, IDI_APPLICATION)?;
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
    let auto_switch_flag = if auto_switch_displays() {
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
        MENU_APPLY_PC,
        PCWSTR(wide("Apply PC mode").as_ptr()),
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

fn string_from_wide_until_nul(value: &[u16]) -> String {
    let len = value.iter().position(|ch| *ch == 0).unwrap_or(value.len());
    String::from_utf16_lossy(&value[..len])
}
