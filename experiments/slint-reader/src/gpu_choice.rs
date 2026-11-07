//! Which GPU draws the interface. Drawing on the GPU that the screen is wired to
//! avoids copying every frame between GPUs. On most laptops that is the
//! integrated GPU, so the dedicated one can stay asleep while reading. Slint's
//! wgpu renderer honours `WGPU_ADAPTER_NAME`, which this module sets.
use std::ffi::OsString;
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory2, DXGI_ADAPTER_FLAG_SOFTWARE, DXGI_CREATE_FACTORY_FLAGS,
    DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE, DXGI_GPU_PREFERENCE_MINIMUM_POWER, IDXGIAdapter1,
    IDXGIFactory6,
};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegGetValueW};
use windows::core::{HSTRING, w};

#[derive(Debug, PartialEq)]
pub enum Choice {
    /// The GPU the main screen is connected to.
    Display,
    PowerSaving,
    Performance,
}

/// `--gpu display`, `--gpu power-saving` or `--gpu performance`; display when absent.
pub fn requested(args: &[OsString]) -> Result<Choice, String> {
    match args.iter().position(|arg| arg == "--gpu") {
        None => Ok(Choice::Display),
        Some(i) => match args.get(i + 1).and_then(|value| value.to_str()) {
            Some("display") => Ok(Choice::Display),
            Some("power-saving") => Ok(Choice::PowerSaving),
            Some("performance") => Ok(Choice::Performance),
            _ => Err("--gpu must be display, power-saving or performance".into()),
        },
    }
}

/// Windows Settings > Display > Graphics stores a per-app choice here. A value
/// of `GpuPreference=1;` or `GpuPreference=2;` is the person's own decision.
fn windows_setting_for_this_app() -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let name = HSTRING::from(exe.as_os_str());
    let mut buffer = [0u16; 64];
    let mut size = (buffer.len() * 2) as u32;
    // SAFETY: the buffer and its byte size describe valid writable memory.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\DirectX\\UserGpuPreferences"),
            &name,
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if status.is_err() {
        return false;
    }
    let value = String::from_utf16_lossy(&buffer[..(size as usize / 2).min(buffer.len())]);
    value.contains("GpuPreference=1") || value.contains("GpuPreference=2")
}

fn adapter_name(adapter: &IDXGIAdapter1) -> Option<String> {
    // SAFETY: an owned COM adapter queried for its description.
    let desc = unsafe { adapter.GetDesc1() }.ok()?;
    if desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
        return None;
    }
    let end = desc
        .Description
        .iter()
        .position(|c| *c == 0)
        .unwrap_or(desc.Description.len());
    let name = String::from_utf16_lossy(&desc.Description[..end]);
    (!name.trim().is_empty()).then_some(name)
}

/// The hardware adapter Windows ranks first for the choice, or for `Display`
/// the adapter whose output shows the primary desktop at (0, 0).
fn adapter_for(choice: &Choice) -> Option<String> {
    // SAFETY: plain DXGI factory, adapter and output queries with owned results.
    unsafe {
        let factory: IDXGIFactory6 = CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)).ok()?;
        let preference = if *choice == Choice::Performance {
            DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE
        } else {
            DXGI_GPU_PREFERENCE_MINIMUM_POWER
        };
        let mut index = 0;
        while let Ok(adapter) =
            factory.EnumAdapterByGpuPreference::<IDXGIAdapter1>(index, preference)
        {
            index += 1;
            let Some(name) = adapter_name(&adapter) else {
                continue;
            };
            if *choice != Choice::Display {
                return Some(name);
            }
            let mut output = 0;
            while let Ok(screen) = adapter.EnumOutputs(output) {
                output += 1;
                if let Ok(desc) = screen.GetDesc() {
                    let r = desc.DesktopCoordinates;
                    if desc.AttachedToDesktop.as_bool()
                        && r.left <= 0
                        && r.top <= 0
                        && r.right > 0
                        && r.bottom > 0
                    {
                        return Some(name);
                    }
                }
            }
        }
        None
    }
}

/// Call first in `main`, before any thread exists.
pub fn apply(args: &[OsString]) -> Result<(), String> {
    let choice = requested(args)?;
    let explicit = args.iter().any(|arg| arg == "--gpu");
    if std::env::var_os("WGPU_ADAPTER_NAME").is_some()
        || std::env::var_os("WGPU_POWER_PREF").is_some()
        || (!explicit && windows_setting_for_this_app())
    {
        return Ok(());
    }
    if let Some(name) = adapter_for(&choice) {
        // SAFETY: called at the start of main, while the process has one thread.
        unsafe { std::env::set_var("WGPU_ADAPTER_NAME", name) };
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }
    #[test]
    fn the_display_gpu_is_the_default_and_every_choice_parses() {
        assert_eq!(requested(&args(&["app"])), Ok(Choice::Display));
        assert_eq!(
            requested(&args(&["app", "--gpu", "display"])),
            Ok(Choice::Display)
        );
        assert_eq!(
            requested(&args(&["app", "--gpu", "performance"])),
            Ok(Choice::Performance)
        );
        assert_eq!(
            requested(&args(&["app", "--gpu", "power-saving"])),
            Ok(Choice::PowerSaving)
        );
        assert!(requested(&args(&["app", "--gpu", "fast"])).is_err());
        assert!(requested(&args(&["app", "--gpu"])).is_err());
    }
}
