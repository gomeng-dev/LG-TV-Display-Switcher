use std::collections::HashMap;
use std::mem::size_of;
use std::os::windows::process::CommandExt;
use std::process::Command;

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
use windows::Win32::Foundation::{BOOL, HWND, LPARAM};
use windows::Win32::Graphics::Gdi::{
    ChangeDisplaySettingsExW, EnumDisplayMonitors, EnumDisplaySettingsW, GetMonitorInfoW,
    CDS_NORESET, CDS_UPDATEREGISTRY, DEVMODEW, DISP_CHANGE_SUCCESSFUL, DM_PELSHEIGHT, DM_PELSWIDTH,
    DM_POSITION, ENUM_CURRENT_SETTINGS, ENUM_REGISTRY_SETTINGS, HDC, HMONITOR, MONITORINFOEXW,
};

use crate::{log_message, wide, TvPower};

pub(crate) const TV_DEVICE_NAME: &str = r"\\.\DISPLAY3";
const PRIMARY_DEVICE_NAME: &str = r"\\.\DISPLAY2";
const CREATE_NO_WINDOW: u32 = 0x08000000;
pub(crate) fn embedded_tv_mode_script() -> &'static str {
    r#"
Import-Module DisplayConfig -ErrorAction Stop
Enable-Display -DisplayId 1,2,3
Get-DisplayConfig |
    Set-DisplayPrimary -DisplayId 3 |
    Use-DisplayConfig
"#
}

pub(crate) fn embedded_pc_mode_script() -> &'static str {
    r#"
Import-Module DisplayConfig -ErrorAction Stop
Enable-Display -DisplayId 1,2 -DisplayIdToDisable 3
Get-DisplayConfig |
    Set-DisplayPrimary -DisplayId 2 |
    Use-DisplayConfig
"#
}

pub(crate) fn run_embedded_display_config_script(script: &str) -> std::result::Result<(), String> {
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

pub(crate) fn apply_pc_mode_change_display_settings() -> std::result::Result<(), String> {
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

pub(crate) fn apply_pc_mode_native() -> Result<()> {
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

pub(crate) fn is_display_active(device_name: &str) -> bool {
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

pub(crate) fn read_ddc_power(device_name: &str) -> Result<TvPower> {
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

fn string_from_wide_until_nul(value: &[u16]) -> String {
    let len = value.iter().position(|ch| *ch == 0).unwrap_or(value.len());
    String::from_utf16_lossy(&value[..len])
}
