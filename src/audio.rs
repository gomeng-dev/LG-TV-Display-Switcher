use std::os::windows::process::CommandExt;
use std::process::Command;

const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Clone, Debug)]
pub(crate) struct AudioOutputDevice {
    pub(crate) id: Option<String>,
    pub(crate) name: String,
}

pub(crate) fn choose_tv_audio_endpoint_device(
    devices: &[AudioOutputDevice],
    preferred: Option<&str>,
) -> Option<AudioOutputDevice> {
    if let Some(preferred) = preferred.map(str::trim).filter(|value| !value.is_empty()) {
        if let Some(device) = devices
            .iter()
            .find(|device| device.name.eq_ignore_ascii_case(preferred))
        {
            return Some(device.clone());
        }

        if let Some(device) = devices
            .iter()
            .find(|device| name_matches(&device.name, preferred))
        {
            return Some(device.clone());
        }
    }

    for marker in ["LG", "TV", "HDMI", "Digital Audio"] {
        if let Some(device) = devices
            .iter()
            .find(|device| name_matches(&device.name, marker))
        {
            return Some(device.clone());
        }
    }

    for marker in ["NVIDIA High Definition Audio"] {
        if let Some(device) = devices
            .iter()
            .find(|device| name_matches(&device.name, marker))
        {
            return Some(device.clone());
        }
    }

    devices.first().cloned()
}

pub(crate) fn list_audio_output_devices() -> std::result::Result<Vec<AudioOutputDevice>, String> {
    let script = r#"
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new()

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
    [PreserveSig]
    int EnumAudioEndpoints(EDataFlow dataFlow, DeviceState dwStateMask, out IMMDeviceCollection ppDevices);
}

[ComImport, Guid("0BD7A1BE-7A1A-44DB-8397-CC5392387B5E"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IMMDeviceCollection {
    [PreserveSig]
    int GetCount(out uint pcDevices);
    [PreserveSig]
    int Item(uint nDevice, out IMMDevice ppDevice);
}

[ComImport, Guid("D666063F-1587-4E43-81F1-B948E807363F"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IMMDevice {
    [PreserveSig]
    int Activate(ref Guid iid, uint dwClsCtx, IntPtr pActivationParams, out IntPtr ppInterface);
    [PreserveSig]
    int OpenPropertyStore(uint stgmAccess, out IPropertyStore ppProperties);
    [PreserveSig]
    int GetId([MarshalAs(UnmanagedType.LPWStr)] out string ppstrId);
    [PreserveSig]
    int GetState(out uint pdwState);
}

[ComImport, Guid("886D8EEB-8CF2-4446-8D02-CDBA1DBDCF99"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IPropertyStore {
    [PreserveSig]
    int GetCount(out uint cProps);
    [PreserveSig]
    int GetAt(uint iProp, out PROPERTYKEY pkey);
    [PreserveSig]
    int GetValue(ref PROPERTYKEY key, out PROPVARIANT pv);
    [PreserveSig]
    int SetValue(ref PROPERTYKEY key, ref PROPVARIANT propvar);
    [PreserveSig]
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

    public static string[] ListRenderDevices() {
        var rows = new List<string>();
        var enumerator = (IMMDeviceEnumerator)new MMDeviceEnumerator();
        IMMDeviceCollection devices;
        Marshal.ThrowExceptionForHR(enumerator.EnumAudioEndpoints(EDataFlow.eRender, DeviceState.Active, out devices));

        uint count;
        Marshal.ThrowExceptionForHR(devices.GetCount(out count));

        for (uint i = 0; i < count; i++) {
            IMMDevice device;
            Marshal.ThrowExceptionForHR(devices.Item(i, out device));

            string id;
            Marshal.ThrowExceptionForHR(device.GetId(out id));

            IPropertyStore store;
            Marshal.ThrowExceptionForHR(device.OpenPropertyStore(0, out store));
            string name = GetString(store, PKEY_Device_FriendlyName);
            if (!String.IsNullOrWhiteSpace(name)) {
                rows.Add(id.Replace("\t", " ") + "\t" + name.Replace("\t", " "));
            }
        }

        return rows.ToArray();
    }
}
"@

[AudioEndpointLister]::ListRenderDevices()
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
        let devices = parse_audio_device_rows(&String::from_utf8_lossy(&output.stdout));
        if devices.is_empty() {
            list_sound_devices_from_wmi()
        } else {
            Ok(devices)
        }
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let error = if stderr.is_empty() {
            format!("PowerShell exited with {}", output.status)
        } else {
            stderr
        };

        match list_sound_devices_from_wmi() {
            Ok(devices) if !devices.is_empty() => Ok(devices),
            _ => Err(error),
        }
    }
}

pub(crate) fn get_default_audio_output_device() -> std::result::Result<AudioOutputDevice, String> {
    let script = r#"
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new()

Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;

public enum EDataFlow { eRender, eCapture, eAll }
public enum ERole { eConsole, eMultimedia, eCommunications }

[ComImport, Guid("BCDE0395-E52F-467C-8E3D-C4579291692E")]
public class MMDeviceEnumerator {}

[ComImport, Guid("A95664D2-9614-4F35-A746-DE8DB63617E6"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IMMDeviceEnumerator {
    [PreserveSig]
    int EnumAudioEndpoints(EDataFlow dataFlow, uint dwStateMask, out IntPtr ppDevices);
    [PreserveSig]
    int GetDefaultAudioEndpoint(EDataFlow dataFlow, ERole role, out IMMDevice ppEndpoint);
    [PreserveSig]
    int GetDevice([MarshalAs(UnmanagedType.LPWStr)] string pwstrId, out IMMDevice ppDevice);
    [PreserveSig]
    int RegisterEndpointNotificationCallback(IntPtr pClient);
    [PreserveSig]
    int UnregisterEndpointNotificationCallback(IntPtr pClient);
}

[ComImport, Guid("D666063F-1587-4E43-81F1-B948E807363F"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IMMDevice {
    [PreserveSig]
    int Activate(ref Guid iid, uint dwClsCtx, IntPtr pActivationParams, out IntPtr ppInterface);
    [PreserveSig]
    int OpenPropertyStore(uint stgmAccess, out IPropertyStore ppProperties);
    [PreserveSig]
    int GetId([MarshalAs(UnmanagedType.LPWStr)] out string ppstrId);
    [PreserveSig]
    int GetState(out uint pdwState);
}

[ComImport, Guid("886D8EEB-8CF2-4446-8D02-CDBA1DBDCF99"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IPropertyStore {
    [PreserveSig]
    int GetCount(out uint cProps);
    [PreserveSig]
    int GetAt(uint iProp, out PROPERTYKEY pkey);
    [PreserveSig]
    int GetValue(ref PROPERTYKEY key, out PROPVARIANT pv);
    [PreserveSig]
    int SetValue(ref PROPERTYKEY key, ref PROPVARIANT propvar);
    [PreserveSig]
    int Commit();
}

[StructLayout(LayoutKind.Sequential)]
public struct PROPERTYKEY { public Guid fmtid; public uint pid; }

[StructLayout(LayoutKind.Sequential)]
public struct PROPVARIANT { public ushort vt; public ushort w1; public ushort w2; public ushort w3; public IntPtr p; public int p2; }

public class AudioDefaultEndpoint {
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

    public static string GetDefaultRenderDevice() {
        var enumerator = (IMMDeviceEnumerator)new MMDeviceEnumerator();
        IMMDevice device;
        Marshal.ThrowExceptionForHR(enumerator.GetDefaultAudioEndpoint(EDataFlow.eRender, ERole.eConsole, out device));

        string id;
        Marshal.ThrowExceptionForHR(device.GetId(out id));

        IPropertyStore store;
        Marshal.ThrowExceptionForHR(device.OpenPropertyStore(0, out store));
        string name = GetString(store, PKEY_Device_FriendlyName);

        if (String.IsNullOrWhiteSpace(name)) throw new InvalidOperationException("Default render endpoint has no friendly name");
        return id.Replace("\t", " ") + "\t" + name.Replace("\t", " ");
    }
}
"@

[AudioDefaultEndpoint]::GetDefaultRenderDevice()
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
        parse_audio_device_rows(&String::from_utf8_lossy(&output.stdout))
            .into_iter()
            .next()
            .ok_or_else(|| "default audio output was empty".to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            Err(format!("PowerShell exited with {}", output.status))
        } else {
            Err(stderr)
        }
    }
}

fn parse_audio_device_rows(rows: &str) -> Vec<AudioOutputDevice> {
    let mut devices = Vec::new();
    for line in rows.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let (id, name) = line
            .split_once('\t')
            .map(|(id, name)| (Some(id.trim().to_string()), name.trim().to_string()))
            .unwrap_or_else(|| (None, line.to_string()));

        if name.is_empty() {
            continue;
        }

        if devices.iter().any(|device: &AudioOutputDevice| {
            device.name.eq_ignore_ascii_case(&name) && device.id == id
        }) {
            continue;
        }

        devices.push(AudioOutputDevice { id, name });
    }
    devices
}

fn list_sound_devices_from_wmi() -> std::result::Result<Vec<AudioOutputDevice>, String> {
    let script = r#"
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new()
Get-CimInstance Win32_SoundDevice |
    Where-Object { $_.Name } |
    Select-Object -ExpandProperty Name
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
        let mut devices = Vec::new();
        for name in String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
        {
            if !devices
                .iter()
                .any(|device: &AudioOutputDevice| device.name.eq_ignore_ascii_case(name))
            {
                devices.push(AudioOutputDevice {
                    id: None,
                    name: name.to_string(),
                });
            }
        }
        Ok(devices)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            Err(format!("PowerShell exited with {}", output.status))
        } else {
            Err(stderr)
        }
    }
}

pub(crate) fn set_default_audio_output(
    endpoint_id: Option<&str>,
    name_filter: &str,
) -> std::result::Result<String, String> {
    let script = r#"
$EndpointId = $env:LG_TV_DISPLAY_SWITCHER_AUDIO_ENDPOINT_ID
$NameFilter = $env:LG_TV_DISPLAY_SWITCHER_AUDIO_NAME_FILTER
$TimeoutMs = 8000

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
    [PreserveSig]
    int EnumAudioEndpoints(EDataFlow dataFlow, DeviceState dwStateMask, out IMMDeviceCollection ppDevices);
    [PreserveSig]
    int GetDefaultAudioEndpoint(EDataFlow dataFlow, ERole role, out IMMDevice ppEndpoint);
    [PreserveSig]
    int GetDevice([MarshalAs(UnmanagedType.LPWStr)] string pwstrId, out IMMDevice ppDevice);
    [PreserveSig]
    int RegisterEndpointNotificationCallback(IntPtr pClient);
    [PreserveSig]
    int UnregisterEndpointNotificationCallback(IntPtr pClient);
}

[ComImport, Guid("0BD7A1BE-7A1A-44DB-8397-CC5392387B5E"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IMMDeviceCollection {
    [PreserveSig]
    int GetCount(out uint pcDevices);
    [PreserveSig]
    int Item(uint nDevice, out IMMDevice ppDevice);
}

[ComImport, Guid("D666063F-1587-4E43-81F1-B948E807363F"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IMMDevice {
    [PreserveSig]
    int Activate(ref Guid iid, uint dwClsCtx, IntPtr pActivationParams, out IntPtr ppInterface);
    [PreserveSig]
    int OpenPropertyStore(uint stgmAccess, out IPropertyStore ppProperties);
    [PreserveSig]
    int GetId([MarshalAs(UnmanagedType.LPWStr)] out string ppstrId);
    [PreserveSig]
    int GetState(out uint pdwState);
}

[ComImport, Guid("886D8EEB-8CF2-4446-8D02-CDBA1DBDCF99"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IPropertyStore {
    [PreserveSig]
    int GetCount(out uint cProps);
    [PreserveSig]
    int GetAt(uint iProp, out PROPERTYKEY pkey);
    [PreserveSig]
    int GetValue(ref PROPERTYKEY key, out PROPVARIANT pv);
    [PreserveSig]
    int SetValue(ref PROPERTYKEY key, ref PROPVARIANT propvar);
    [PreserveSig]
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
    [PreserveSig]
    int GetMixFormat([MarshalAs(UnmanagedType.LPWStr)] string pszDeviceName, out IntPtr ppFormat);
    [PreserveSig]
    int GetDeviceFormat([MarshalAs(UnmanagedType.LPWStr)] string pszDeviceName, int bDefault, out IntPtr ppFormat);
    [PreserveSig]
    int ResetDeviceFormat([MarshalAs(UnmanagedType.LPWStr)] string pszDeviceName);
    [PreserveSig]
    int SetDeviceFormat([MarshalAs(UnmanagedType.LPWStr)] string pszDeviceName, IntPtr pEndpointFormat, IntPtr pMixFormat);
    [PreserveSig]
    int GetProcessingPeriod([MarshalAs(UnmanagedType.LPWStr)] string pszDeviceName, int bDefault, out long pmftDefaultPeriod, out long pmftMinimumPeriod);
    [PreserveSig]
    int SetProcessingPeriod([MarshalAs(UnmanagedType.LPWStr)] string pszDeviceName, ref long pmftPeriod);
    [PreserveSig]
    int GetShareMode([MarshalAs(UnmanagedType.LPWStr)] string pszDeviceName, IntPtr pMode);
    [PreserveSig]
    int SetShareMode([MarshalAs(UnmanagedType.LPWStr)] string pszDeviceName, IntPtr mode);
    [PreserveSig]
    int GetPropertyValue([MarshalAs(UnmanagedType.LPWStr)] string pszDeviceName, ref PROPERTYKEY key, out PROPVARIANT pv);
    [PreserveSig]
    int SetPropertyValue([MarshalAs(UnmanagedType.LPWStr)] string pszDeviceName, ref PROPERTYKEY key, ref PROPVARIANT pv);
    [PreserveSig]
    int SetDefaultEndpoint([MarshalAs(UnmanagedType.LPWStr)] string wszDeviceId, ERole role);
    [PreserveSig]
    int SetEndpointVisibility([MarshalAs(UnmanagedType.LPWStr)] string pszDeviceName, int bVisible);
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

    static string Compact(string value) {
        if (value == null) return "";
        return value.Replace(" ", "").Replace("\t", "").Replace("\r", "").Replace("\n", "");
    }

    static bool Matches(string name, string nameFilter) {
        if (String.IsNullOrWhiteSpace(name) || String.IsNullOrWhiteSpace(nameFilter)) return false;
        if (name.IndexOf(nameFilter, StringComparison.OrdinalIgnoreCase) >= 0) return true;
        return Compact(name).IndexOf(Compact(nameFilter), StringComparison.OrdinalIgnoreCase) >= 0;
    }

    static string GetFriendlyName(IMMDevice device) {
        IPropertyStore store;
        Marshal.ThrowExceptionForHR(device.OpenPropertyStore(0, out store));
        return GetString(store, PKEY_Device_FriendlyName);
    }

    static bool TryGetDeviceById(string endpointId, out string bestId, out string bestName) {
        bestId = null;
        bestName = null;
        if (String.IsNullOrWhiteSpace(endpointId)) return false;

        try {
            var enumerator = (IMMDeviceEnumerator)new MMDeviceEnumerator();
            IMMDevice device;
            Marshal.ThrowExceptionForHR(enumerator.GetDevice(endpointId, out device));

            uint state;
            Marshal.ThrowExceptionForHR(device.GetState(out state));
            if ((state & (uint)DeviceState.Active) == 0) return false;

            string id;
            Marshal.ThrowExceptionForHR(device.GetId(out id));

            bestId = id;
            bestName = GetFriendlyName(device);
            return !String.IsNullOrWhiteSpace(bestName);
        } catch {
            return false;
        }
    }

    static bool TryFindDeviceByName(string nameFilter, out string bestId, out string bestName) {
        var enumerator = (IMMDeviceEnumerator)new MMDeviceEnumerator();
        IMMDeviceCollection devices;
        Marshal.ThrowExceptionForHR(enumerator.EnumAudioEndpoints(EDataFlow.eRender, DeviceState.Active, out devices));

        uint count;
        Marshal.ThrowExceptionForHR(devices.GetCount(out count));

        bestId = null;
        bestName = null;

        for (uint i = 0; i < count; i++) {
            IMMDevice device;
            Marshal.ThrowExceptionForHR(devices.Item(i, out device));

            string id;
            Marshal.ThrowExceptionForHR(device.GetId(out id));

            string name = GetFriendlyName(device);

            if (Matches(name, nameFilter)) {
                bestId = id;
                bestName = name;
                return true;
            }
        }

        return false;
    }

    static void GetDefaultRenderIdentity(out string id, out string name) {
        var enumerator = (IMMDeviceEnumerator)new MMDeviceEnumerator();
        IMMDevice device;
        Marshal.ThrowExceptionForHR(enumerator.GetDefaultAudioEndpoint(EDataFlow.eRender, ERole.eConsole, out device));
        Marshal.ThrowExceptionForHR(device.GetId(out id));
        name = GetFriendlyName(device);
    }

    public static string SetDefault(string endpointId, string nameFilter, int timeoutMs) {
        string bestId = null;
        string bestName = null;
        bool selectedById = false;
        var deadline = DateTime.UtcNow.AddMilliseconds(timeoutMs);

        do {
            if (TryGetDeviceById(endpointId, out bestId, out bestName)) {
                selectedById = true;
                break;
            }

            if (TryFindDeviceByName(nameFilter, out bestId, out bestName)) break;
            System.Threading.Thread.Sleep(250);
        } while (DateTime.UtcNow < deadline);

        if (bestId == null) {
            throw new InvalidOperationException("No active render endpoint matched. EndpointId=" + endpointId + ", NameFilter=" + nameFilter);
        }

        var policy = (IPolicyConfig)new PolicyConfigClient();
        Marshal.ThrowExceptionForHR(policy.SetDefaultEndpoint(bestId, ERole.eConsole));
        Marshal.ThrowExceptionForHR(policy.SetDefaultEndpoint(bestId, ERole.eMultimedia));
        Marshal.ThrowExceptionForHR(policy.SetDefaultEndpoint(bestId, ERole.eCommunications));

        string defaultId;
        string defaultName;
        GetDefaultRenderIdentity(out defaultId, out defaultName);

        if (selectedById) {
            if (!String.Equals(defaultId, bestId, StringComparison.OrdinalIgnoreCase)) {
                throw new InvalidOperationException("Default render endpoint verification failed. Current default: " + defaultName);
            }
        } else if (!Matches(defaultName, bestName)) {
            throw new InvalidOperationException("Default render endpoint verification failed. Current default: " + defaultName);
        }

        return bestName;
    }
}
"@

[AudioSwitcher]::SetDefault($EndpointId, $NameFilter, $TimeoutMs)
"#;

    let output = Command::new("powershell.exe")
        .arg("-NoProfile")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-Command")
        .arg(script)
        .env(
            "LG_TV_DISPLAY_SWITCHER_AUDIO_ENDPOINT_ID",
            endpoint_id.unwrap_or_default(),
        )
        .env("LG_TV_DISPLAY_SWITCHER_AUDIO_NAME_FILTER", name_filter)
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

fn name_matches(name: &str, filter: &str) -> bool {
    if name.is_empty() || filter.trim().is_empty() {
        return false;
    }

    if name
        .to_ascii_lowercase()
        .contains(&filter.to_ascii_lowercase())
    {
        return true;
    }

    compact(name)
        .to_ascii_lowercase()
        .contains(&compact(filter).to_ascii_lowercase())
}

fn compact(value: &str) -> String {
    value.chars().filter(|ch| !ch.is_whitespace()).collect()
}
