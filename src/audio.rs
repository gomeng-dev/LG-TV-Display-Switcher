use std::os::windows::process::CommandExt;
use std::process::Command;

const CREATE_NO_WINDOW: u32 = 0x08000000;

pub(crate) fn choose_tv_audio_endpoint(
    names: &[String],
    preferred: Option<&str>,
) -> Option<String> {
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

pub(crate) fn list_audio_output_names() -> std::result::Result<Vec<String>, String> {
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

pub(crate) fn set_default_audio_output_by_name(
    name_filter: &str,
) -> std::result::Result<String, String> {
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
