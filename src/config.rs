use std::path::Path;
use std::time::Duration;

const CONFIG_FILE_NAME: &str = "LG-TV-Display-Switcher.cfg";
const LEGACY_CONFIG_FILE_NAME: &str = "TVGuardTray.cfg";
const DEFAULT_WEBOS_PORT: u16 = 3001;
const DEFAULT_WEBOS_TIMEOUT_MS: u64 = 1500;

#[derive(Clone, Debug)]
pub(crate) struct AppConfig {
    pub(crate) webos_host: Option<String>,
    pub(crate) webos_port: u16,
    pub(crate) webos_timeout: Duration,
    pub(crate) auto_apply_pc_mode: bool,
    pub(crate) auto_switch_displays: bool,
    pub(crate) tv_mac: Option<[u8; 6]>,
    pub(crate) wake_broadcast: String,
    pub(crate) wake_port: u16,
    pub(crate) webos_client_key: Option<String>,
    pub(crate) auto_switch_audio: bool,
    pub(crate) pc_audio_endpoint_id: Option<String>,
    pub(crate) pc_audio_device_name_contains: Option<String>,
    pub(crate) tv_audio_endpoint_id: Option<String>,
    pub(crate) tv_audio_device_name_contains: Option<String>,
    pub(crate) try_enable_dolby_atmos: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
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
            pc_audio_endpoint_id: None,
            pc_audio_device_name_contains: None,
            tv_audio_endpoint_id: None,
            tv_audio_device_name_contains: Some("NVIDIA High Definition Audio".to_string()),
            try_enable_dolby_atmos: false,
        }
    }
}

pub(crate) fn load_or_create_config(base_dir: &Path) -> AppConfig {
    let path = base_dir.join(CONFIG_FILE_NAME);
    if !path.exists() {
        let legacy_path = base_dir.join(LEGACY_CONFIG_FILE_NAME);
        if legacy_path.exists() {
            let _ = std::fs::copy(&legacy_path, &path);
        } else {
            let _ = std::fs::write(&path, default_config_template());
        }
    }

    let mut config = AppConfig::default();
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
            "pcaudioendpointid" => {
                config.pc_audio_endpoint_id = (!value.is_empty()).then(|| value.to_string());
            }
            "pcaudiodevicenamecontains" => {
                config.pc_audio_device_name_contains =
                    (!value.is_empty()).then(|| value.to_string());
            }
            "tvaudioendpointid" => {
                config.tv_audio_endpoint_id = (!value.is_empty()).then(|| value.to_string());
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

pub(crate) fn save_webos_client_key(client_key: &str) {
    save_config_value("WebOsClientKey", client_key);
}

pub(crate) fn save_config_value(key: &str, value: &str) {
    let base_dir = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(std::path::Path::to_path_buf))
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| std::path::PathBuf::from("."));
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
        "PcAudioEndpointId=",
        "PcAudioDeviceNameContains=",
        "TvAudioEndpointId=",
        "TvAudioDeviceNameContains=NVIDIA High Definition Audio",
        "TryEnableDolbyAtmos=false",
        "",
    ]
    .join("\r\n")
}

pub(crate) fn parse_mac_address(value: &str) -> Option<[u8; 6]> {
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

pub(crate) fn format_mac_address(mac: [u8; 6]) -> String {
    mac.iter()
        .map(|part| format!("{part:02x}"))
        .collect::<Vec<_>>()
        .join(":")
}
