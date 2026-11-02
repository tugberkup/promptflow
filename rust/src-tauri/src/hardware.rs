//! What the machine can run: GPU and its memory, CPU, total RAM.

use serde::Serialize;

#[derive(Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
// Macs only ever report Apple or None; Discrete and Integrated come from the Windows detection
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub enum GpuKind {
    /// NVIDIA / AMD / Intel Arc card with its own memory
    Discrete,
    /// Graphics built into the processor; still faster than the CPU through Vulkan
    Integrated,
    /// Apple Silicon: Metal on unified memory
    Apple,
    /// Nothing usable: Whisper runs on the CPU
    None,
}

#[derive(Serialize, Clone, Debug)]
pub struct Hardware {
    /// e.g. "NVIDIA GeForce RTX 4060 Ti" or "Apple M2"
    pub gpu: String,
    pub gpu_kind: GpuKind,
    /// Dedicated video memory in GB (Apple Silicon: the unified memory)
    pub vram_gb: f32,
    pub ram_gb: f32,
    pub cpu: String,
    pub cpu_cores: usize,
    /// AVX2 on x86 (whisper.cpp's fast path); always true on ARM
    pub cpu_fast_math: bool,
}

fn cpu_info() -> (String, usize, f32) {
    let Ok(mut system) = sysinfo::System::new() else { return (String::new(), 4, 0.0) };
    system.refresh_cpu_all();
    system.refresh_memory();
    let name = system.cpus().first().map(|c| c.brand().trim().to_string()).unwrap_or_default();
    let logical = system.cpus().len().max(1);
    let cores = sysinfo::System::physical_core_count().unwrap_or(logical / 2).max(1);
    (name, cores, system.total_memory() as f32 / 1_073_741_824.0)
}

fn fast_math() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        std::arch::is_x86_feature_detected!("avx2") && std::arch::is_x86_feature_detected!("fma")
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        true
    }
}

#[cfg(target_os = "windows")]
fn gpu() -> (String, GpuKind, f32) {
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
    let Some((name, vram, vendor)) = best else { return (String::new(), GpuKind::None, 0.0) };
    // Integrated GPUs report a few hundred MB of "dedicated" memory
    let kind = if matches!(vendor, 0x10DE | 0x1002 | 0x8086) && vram >= 2.0 { GpuKind::Discrete } else { GpuKind::Integrated };
    (name, kind, vram)
}

#[cfg(target_os = "macos")]
fn gpu() -> (String, GpuKind, f32) {
    if cfg!(target_arch = "aarch64") {
        (String::new(), GpuKind::Apple, 0.0) // name and memory are filled in from the CPU below
    } else {
        (String::new(), GpuKind::None, 0.0) // Intel Macs run Whisper on the CPU
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn gpu() -> (String, GpuKind, f32) {
    (String::new(), GpuKind::None, 0.0)
}

pub fn detect() -> Hardware {
    let (cpu, cpu_cores, ram_gb) = cpu_info();
    let (mut gpu, gpu_kind, mut vram_gb) = gpu();
    if gpu_kind == GpuKind::Apple {
        gpu = cpu.clone();
        vram_gb = ram_gb;
    }
    Hardware { gpu, gpu_kind, vram_gb, ram_gb, cpu, cpu_cores, cpu_fast_math: fast_math() }
}

#[cfg(test)]
mod tests {
    #[test]
    fn detects_something() {
        let hw = super::detect();
        println!("{hw:?}");
        assert!(hw.ram_gb > 0.5 && hw.cpu_cores >= 1);
    }
}
