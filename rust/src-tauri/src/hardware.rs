//! What the machine can run: GPU and its memory, total RAM, Apple Silicon or not.

use serde::Serialize;

#[derive(Serialize, Clone, Debug)]
pub struct Hardware {
    /// e.g. "NVIDIA GeForce RTX 4060 Ti" or "Apple M2"
    pub gpu: String,
    /// Dedicated video memory in GB (on Apple Silicon: the unified memory)
    pub vram_gb: f32,
    pub ram_gb: f32,
    /// A GPU that runs Whisper large models in well under a second
    pub fast_gpu: bool,
    pub apple_silicon: bool,
}

fn ram_gb() -> f32 {
    let Ok(mut system) = sysinfo::System::new() else { return 0.0 };
    system.refresh_memory();
    system.total_memory() as f32 / 1_073_741_824.0
}

#[cfg(target_os = "windows")]
pub fn detect() -> Hardware {
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE};

    // The adapter with the most dedicated memory is the one Vulkan will use for Whisper
    let mut best: Option<(String, f32, u32)> = None;
    unsafe {
        if let Ok(factory) = CreateDXGIFactory1::<IDXGIFactory1>() {
            let mut i = 0;
            while let Ok(adapter) = factory.EnumAdapters1(i) {
                i += 1;
                let Ok(desc) = adapter.GetDesc1() else { continue };
                if desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
                    continue;
                }
                let name = String::from_utf16_lossy(&desc.Description).trim_end_matches('\0').to_string();
                let vram = desc.DedicatedVideoMemory as f32 / 1_073_741_824.0;
                if best.as_ref().is_none_or(|b| vram > b.1) {
                    best = Some((name, vram, desc.VendorId));
                }
            }
        }
    }
    let (gpu, vram_gb, vendor) = best.unwrap_or_else(|| ("No GPU found".into(), 0.0, 0));
    // NVIDIA (0x10DE) and AMD (0x1002) cards, or Intel Arc, with real video memory
    let fast_gpu = matches!(vendor, 0x10DE | 0x1002 | 0x8086) && vram_gb >= 3.5;
    Hardware { gpu, vram_gb, ram_gb: ram_gb(), fast_gpu, apple_silicon: false }
}

#[cfg(target_os = "macos")]
pub fn detect() -> Hardware {
    let ram = ram_gb();
    let apple_silicon = cfg!(target_arch = "aarch64");
    let chip = sysinfo::System::new()
        .map(|mut system| {
            system.refresh_cpu_all();
            system.cpus().first().map(|c| c.brand().to_string()).unwrap_or_default()
        })
        .unwrap_or_default();
    Hardware {
        gpu: if chip.is_empty() { "Mac".into() } else { chip },
        vram_gb: if apple_silicon { ram } else { 0.0 },
        ram_gb: ram,
        fast_gpu: apple_silicon, // Intel Macs run Whisper on the CPU
        apple_silicon,
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn detect() -> Hardware {
    Hardware { gpu: "Unknown".into(), vram_gb: 0.0, ram_gb: ram_gb(), fast_gpu: false, apple_silicon: false }
}
