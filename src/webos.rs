use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use native_tls::TlsConnector;
use serde_json::{json, Value};

use crate::config::AppConfig;
use crate::{TvPower, APP_NAME};
trait ReadWrite: Read + Write {}
impl<T: Read + Write> ReadWrite for T {}

pub(crate) fn turn_off_webos_tv(
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

pub(crate) fn pair_webos_tv(
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
pub(crate) fn read_webos_power(
    host: &str,
    port: u16,
    timeout: Duration,
    client_key: Option<&str>,
) -> TvPower {
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
                return read_webos_power_tls(host, port, timeout, stream, client_key);
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
                read_webos_power_state_or_assume_on(&mut stream, client_key)
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

pub(crate) fn read_webos_power_tls(
    host: &str,
    port: u16,
    timeout: Duration,
    stream: TcpStream,
    client_key: Option<&str>,
) -> TvPower {
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
                read_webos_power_state_or_assume_on(&mut tls, client_key)
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

fn read_webos_power_state_or_assume_on(
    stream: &mut dyn ReadWrite,
    client_key: Option<&str>,
) -> TvPower {
    let Some(client_key) = client_key else {
        return TvPower::On;
    };

    if let Err(error) = register_webos_client(stream, Some(client_key)) {
        return TvPower::NotOn {
            code: None,
            reason: format!("webOS power-state registration failed: {error}"),
        };
    }

    let request = json!({
        "id": "power_state",
        "type": "request",
        "uri": "ssap://com.webos.service.tvpower/power/getPowerState",
        "payload": {}
    });

    if let Err(error) = send_ws_text(stream, &request.to_string()) {
        return TvPower::NotOn {
            code: None,
            reason: format!("webOS power-state request failed: {error}"),
        };
    }

    for _ in 0..10 {
        let message = match read_ws_text(stream) {
            Ok(message) => message,
            Err(error) => {
                return TvPower::NotOn {
                    code: None,
                    reason: format!("webOS power-state read failed: {error}"),
                };
            }
        };

        let Ok(value) = serde_json::from_str::<Value>(&message) else {
            continue;
        };

        if value.get("id").and_then(Value::as_str) != Some("power_state") {
            continue;
        }

        if value.get("type").and_then(Value::as_str) == Some("error") {
            return TvPower::NotOn {
                code: value
                    .get("error")
                    .and_then(Value::as_str)
                    .and_then(parse_webos_error_code),
                reason: format!("webOS power-state error: {value}"),
            };
        }

        let payload = value.get("payload").unwrap_or(&value);
        if let Some(state) = power_state_from_payload(payload) {
            return tv_power_from_webos_state(&state);
        }

        return TvPower::On;
    }

    TvPower::NotOn {
        code: None,
        reason: "webOS power-state response timed out".to_string(),
    }
}

fn power_state_from_payload(payload: &Value) -> Option<String> {
    for key in ["state", "powerState", "power_state"] {
        if let Some(state) = payload.get(key).and_then(Value::as_str) {
            return Some(state.to_string());
        }
    }
    None
}

fn tv_power_from_webos_state(state: &str) -> TvPower {
    let normalized = state
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase();

    if normalized.contains("active") || normalized == "on" || normalized.contains("screenon") {
        TvPower::On
    } else if normalized.contains("standby")
        || normalized.contains("suspend")
        || normalized.contains("poweroff")
        || normalized == "off"
    {
        TvPower::NotOn {
            code: None,
            reason: format!("webOS power state {state}"),
        }
    } else {
        TvPower::On
    }
}

fn parse_webos_error_code(error: &str) -> Option<u32> {
    error
        .split_whitespace()
        .next()
        .and_then(|code| code.parse::<u32>().ok())
}
