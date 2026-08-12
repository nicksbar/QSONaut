//! Hardware discovery, backend policy, and lightweight decode telemetry.
//!
//! GPU discovery is real, but GPU decoder execution remains deliberately
//! disabled until a kernel passes the same fixtures as the CPU path and wins
//! its end-to-end benchmark (including upload/readback costs).

use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputePreference {
    #[default]
    Auto,
    Cpu,
    Gpu,
}

impl ComputePreference {
    pub const ALL: [Self; 3] = [Self::Auto, Self::Cpu, Self::Gpu];

    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "AUTO",
            Self::Cpu => "CPU",
            Self::Gpu => "GPU",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveBackend {
    CpuSimd,
    GpuCompute,
}

impl ActiveBackend {
    pub fn label(self) -> &'static str {
        match self {
            Self::CpuSimd => "CPU SIMD",
            Self::GpuCompute => "GPU COMPUTE",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuAdapterInfo {
    pub name: String,
    pub backend: String,
    pub device_type: String,
}

#[derive(Debug, Clone)]
pub struct AccelerationReport {
    pub preference: ComputePreference,
    pub active: ActiveBackend,
    pub logical_cpus: usize,
    pub simd_features: Vec<&'static str>,
    /// Hardware compute devices discovered through wgpu or a native compute
    /// runtime such as CUDA. Software rasterizers never appear in this list.
    pub gpu_adapters: Vec<GpuAdapterInfo>,
    pub software_adapters: Vec<GpuAdapterInfo>,
    pub npu_exposed: bool,
    pub gpu_kernels_validated: bool,
    pub fallback_reason: Option<String>,
}

impl AccelerationReport {
    pub fn probe(preference: ComputePreference) -> Self {
        let (mut gpu_adapters, software_adapters) = enumerate_wgpu_adapters();
        for cuda_device in enumerate_cuda_devices() {
            if !gpu_adapters
                .iter()
                .any(|adapter| adapter.name.eq_ignore_ascii_case(&cuda_device.name))
            {
                gpu_adapters.push(cuda_device);
            }
        }
        let gpu_kernels_validated = false;
        let gpu_available = !gpu_adapters.is_empty();
        let (active, fallback_reason) = match preference {
            ComputePreference::Cpu => (ActiveBackend::CpuSimd, None),
            ComputePreference::Auto if gpu_available && gpu_kernels_validated => {
                (ActiveBackend::GpuCompute, None)
            }
            ComputePreference::Gpu if gpu_available && gpu_kernels_validated => {
                (ActiveBackend::GpuCompute, None)
            }
            ComputePreference::Gpu if !gpu_available => (
                ActiveBackend::CpuSimd,
                Some("GPU requested, but no compute adapter is exposed".to_string()),
            ),
            ComputePreference::Gpu => (
                ActiveBackend::CpuSimd,
                Some("GPU detected; decoder kernels are not validated yet".to_string()),
            ),
            ComputePreference::Auto => (
                ActiveBackend::CpuSimd,
                if gpu_available {
                    Some("GPU detected; AUTO stays on CPU until kernels win validation".to_string())
                } else {
                    Some("No GPU compute adapter exposed; using CPU SIMD".to_string())
                },
            ),
        };

        Self {
            preference,
            active,
            logical_cpus: std::thread::available_parallelism().map_or(1, usize::from),
            simd_features: detected_simd_features(),
            gpu_adapters,
            software_adapters,
            npu_exposed: npu_device_exposed(),
            gpu_kernels_validated,
            fallback_reason,
        }
    }

    pub fn summary(&self) -> String {
        format!(
            "{} · {} threads · {}",
            self.active.label(),
            self.logical_cpus,
            if self.simd_features.is_empty() {
                "scalar".to_string()
            } else {
                self.simd_features.join("/")
            }
        )
    }

    pub fn hardware_detail(&self) -> String {
        let gpu = self
            .gpu_adapters
            .first()
            .map(|adapter| format!("{} ({})", adapter.name, adapter.backend))
            .or_else(|| {
                self.software_adapters.first().map(|adapter| {
                    format!("{} (software {}; CPU only)", adapter.name, adapter.backend)
                })
            })
            .unwrap_or_else(|| "GPU not exposed".to_string());
        let npu = if self.npu_exposed {
            "NPU exposed"
        } else {
            "NPU not exposed"
        };
        format!("{gpu} · {npu}")
    }
}

fn enumerate_wgpu_adapters() -> (Vec<GpuAdapterInfo>, Vec<GpuAdapterInfo>) {
    #[cfg(target_os = "linux")]
    if !Path::new("/dev/dri").exists() {
        // This crate currently enables wgpu's Vulkan backend. WSL's /dev/dxg
        // exposes D3D12/CUDA, not a Vulkan DRM device; probing Vulkan there
        // only initializes lavapipe/llvmpipe and floods debug logs. CUDA is
        // discovered independently below.
        return (Vec::new(), Vec::new());
    }

    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        flags: wgpu::InstanceFlags::empty(),
        ..Default::default()
    });
    let mut hardware = Vec::new();
    let mut software = Vec::new();
    for adapter in instance.enumerate_adapters(wgpu::Backends::VULKAN) {
        let info = adapter.get_info();
        let adapter = GpuAdapterInfo {
            name: info.name,
            backend: format!("{:?}", info.backend),
            device_type: format!("{:?}", info.device_type),
        };
        if is_software_adapter(&adapter) {
            software.push(adapter);
        } else {
            hardware.push(adapter);
        }
    }
    (hardware, software)
}

fn is_software_adapter(adapter: &GpuAdapterInfo) -> bool {
    let name = adapter.name.to_ascii_lowercase();
    adapter.device_type.eq_ignore_ascii_case("cpu")
        || ["llvmpipe", "lavapipe", "swiftshader", "software rasterizer"]
            .iter()
            .any(|marker| name.contains(marker))
}

fn enumerate_cuda_devices() -> Vec<GpuAdapterInfo> {
    #[cfg(not(target_os = "linux"))]
    let candidates = ["nvidia-smi"];
    #[cfg(target_os = "linux")]
    let candidates = ["nvidia-smi", "/usr/lib/wsl/lib/nvidia-smi"];

    let output = candidates.iter().find_map(|program| {
        Command::new(program)
            .args(["--query-gpu=name", "--format=csv,noheader,nounits"])
            .output()
            .ok()
            .filter(|output| output.status.success())
    });

    output
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .into_iter()
        .flat_map(|output| {
            output
                .lines()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(|name| GpuAdapterInfo {
                    name: name.to_string(),
                    backend: "CUDA".to_string(),
                    device_type: "DiscreteGpu".to_string(),
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn npu_device_exposed() -> bool {
    ["/dev/accel/accel0", "/dev/amdxdna", "/dev/xdna0"]
        .iter()
        .any(|path| Path::new(path).exists())
}

fn detected_simd_features() -> Vec<&'static str> {
    let mut features = Vec::new();
    #[cfg(target_arch = "x86_64")]
    {
        if std::is_x86_feature_detected!("avx2") {
            features.push("AVX2");
        }
        if std::is_x86_feature_detected!("fma") {
            features.push("FMA");
        }
        if std::is_x86_feature_detected!("avx512f") {
            features.push("AVX-512");
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        features.push("NEON");
    }
    features
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageTiming {
    pub name: &'static str,
    pub duration: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeTelemetry {
    pub mode: String,
    pub backend: ActiveBackend,
    pub samples: usize,
    pub decoded: usize,
    pub total: Duration,
    pub budget: Duration,
    pub stages: Vec<StageTiming>,
}

impl DecodeTelemetry {
    pub fn realtime_percent(&self) -> f32 {
        if self.budget.is_zero() {
            return 0.0;
        }
        self.total.as_secs_f32() / self.budget.as_secs_f32() * 100.0
    }

    pub fn concise(&self) -> String {
        format!(
            "{} ms · {:.0}% slot · {} decoded",
            self.total.as_millis(),
            self.realtime_percent(),
            self.decoded
        )
    }

    pub fn stage_detail(&self) -> String {
        self.stages
            .iter()
            .map(|stage| format!("{} {}ms", stage.name, stage.duration.as_millis()))
            .collect::<Vec<_>>()
            .join(" · ")
    }
}

pub struct DecodeTrace {
    mode: String,
    backend: ActiveBackend,
    samples: usize,
    budget: Duration,
    started: Instant,
    stages: Vec<StageTiming>,
}

/// Result of running identical decode fixtures through CPU and accelerator
/// implementations. A GPU kernel is eligible only when every output digest
/// matches and its end-to-end runtime clears the configured speedup floor.
#[derive(Debug, Clone, PartialEq)]
pub struct KernelBenchmark {
    pub fixture_count: usize,
    pub cpu_digest: u64,
    pub accelerator_digest: u64,
    pub cpu_time: Duration,
    pub accelerator_time: Duration,
}

impl KernelBenchmark {
    pub fn outputs_match(&self) -> bool {
        self.fixture_count > 0 && self.cpu_digest == self.accelerator_digest
    }

    pub fn speedup(&self) -> f32 {
        if self.accelerator_time.is_zero() {
            return f32::INFINITY;
        }
        self.cpu_time.as_secs_f32() / self.accelerator_time.as_secs_f32()
    }

    pub fn eligible(&self, minimum_speedup: f32) -> bool {
        self.outputs_match() && self.speedup() >= minimum_speedup.max(1.0)
    }
}

impl DecodeTrace {
    pub fn new(
        mode: impl Into<String>,
        backend: ActiveBackend,
        samples: usize,
        budget: Duration,
    ) -> Self {
        Self {
            mode: mode.into(),
            backend,
            samples,
            budget,
            started: Instant::now(),
            stages: Vec::new(),
        }
    }

    pub fn measure<T>(&mut self, name: &'static str, operation: impl FnOnce() -> T) -> T {
        let started = Instant::now();
        let result = operation();
        self.stages.push(StageTiming {
            name,
            duration: started.elapsed(),
        });
        result
    }

    pub fn finish(self, decoded: usize) -> DecodeTelemetry {
        DecodeTelemetry {
            mode: self.mode,
            backend: self.backend,
            samples: self.samples,
            decoded,
            total: self.started.elapsed(),
            budget: self.budget,
            stages: self.stages,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_preference_falls_back_until_kernels_are_validated() {
        let report = AccelerationReport::probe(ComputePreference::Gpu);
        assert_eq!(report.active, ActiveBackend::CpuSimd);
        assert!(report.fallback_reason.is_some());
    }

    #[test]
    fn software_vulkan_adapters_are_not_gpus() {
        assert!(is_software_adapter(&GpuAdapterInfo {
            name: "llvmpipe (LLVM 19.1.7, 256 bits)".to_string(),
            backend: "Vulkan".to_string(),
            device_type: "Cpu".to_string(),
        }));
        assert!(!is_software_adapter(&GpuAdapterInfo {
            name: "NVIDIA GeForce RTX 5080 Laptop GPU".to_string(),
            backend: "CUDA".to_string(),
            device_type: "DiscreteGpu".to_string(),
        }));
    }

    #[test]
    fn trace_reports_stages_and_realtime_budget() {
        let mut trace = DecodeTrace::new(
            "FT4",
            ActiveBackend::CpuSimd,
            90_000,
            Duration::from_millis(7_500),
        );
        let value = trace.measure("prepare", || 42);
        assert_eq!(value, 42);
        let telemetry = trace.finish(3);
        assert_eq!(telemetry.stages[0].name, "prepare");
        assert_eq!(telemetry.decoded, 3);
        assert!(telemetry.realtime_percent() < 100.0);
    }

    #[test]
    fn kernel_gate_requires_correctness_and_real_speedup() {
        let fast_but_wrong = KernelBenchmark {
            fixture_count: 12,
            cpu_digest: 7,
            accelerator_digest: 8,
            cpu_time: Duration::from_millis(120),
            accelerator_time: Duration::from_millis(40),
        };
        assert!(!fast_but_wrong.eligible(1.2));

        let correct_but_slower = KernelBenchmark {
            accelerator_digest: 7,
            accelerator_time: Duration::from_millis(130),
            ..fast_but_wrong.clone()
        };
        assert!(!correct_but_slower.eligible(1.2));

        let correct_and_faster = KernelBenchmark {
            accelerator_digest: 7,
            accelerator_time: Duration::from_millis(60),
            ..fast_but_wrong
        };
        assert!(correct_and_faster.eligible(1.2));
    }
}
