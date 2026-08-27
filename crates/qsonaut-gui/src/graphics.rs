use std::sync::Arc;

use eframe::egui_wgpu::{WgpuConfiguration, WgpuSetup, WgpuSetupCreateNew};
use tracing::{info, warn};

pub const GRAPHICS_POWER_ENV: &str = "QSONAUT_GRAPHICS_POWER";
pub const GRAPHICS_ADAPTER_ENV: &str = "QSONAUT_GRAPHICS_ADAPTER";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphicsPowerPreference {
    Low,
    High,
}

impl GraphicsPowerPreference {
    pub const ALL: [Self; 2] = [Self::Low, Self::High];

    pub fn label(self) -> &'static str {
        match self {
            Self::Low => "Low power",
            Self::High => "High performance",
        }
    }

    pub fn env_value(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::High => "high",
        }
    }

    fn from_env() -> Self {
        let configured = std::env::var(GRAPHICS_POWER_ENV)
            .ok()
            .or_else(|| std::env::var("WGPU_POWER_PREF").ok());
        match configured
            .as_deref()
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("high" | "highperformance" | "high-performance") => Self::High,
            Some("low" | "lowpower" | "low-power") => Self::Low,
            None if is_wslg() => {
                // WSLg's translated GL path can make the nominal low-power
                // policy feel sluggish even when the selected adapter is
                // technically correct. Preserve explicit operator choices,
                // but prefer the higher-performance policy by default there.
                Self::High
            }
            _ => Self::Low,
        }
    }

    fn wgpu(self) -> wgpu::PowerPreference {
        match self {
            Self::Low => wgpu::PowerPreference::LowPower,
            Self::High => wgpu::PowerPreference::HighPerformance,
        }
    }
}

fn is_wslg() -> bool {
    cfg!(target_os = "linux")
        && std::fs::read_to_string("/proc/version")
            .map(|version| version.to_ascii_lowercase().contains("microsoft"))
            .unwrap_or(false)
        && std::path::Path::new("/dev/dxg").exists()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphicsPreferences {
    pub power: GraphicsPowerPreference,
    pub adapter: Option<String>,
}

impl GraphicsPreferences {
    pub fn from_environment() -> Self {
        Self {
            power: GraphicsPowerPreference::from_env(),
            adapter: std::env::var(GRAPHICS_ADAPTER_ENV)
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
        }
    }

    pub fn wgpu_configuration(&self) -> WgpuConfiguration {
        let mut create = WgpuSetupCreateNew {
            power_preference: self.power.wgpu(),
            ..Default::default()
        };

        if let Some(requested) = self.adapter.clone() {
            let power = self.power;
            create.native_adapter_selector = Some(Arc::new(move |adapters, surface| {
                select_adapter(adapters, surface, &requested, power)
            }));
        }

        WgpuConfiguration {
            wgpu_setup: WgpuSetup::CreateNew(create),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphicsAdapterInfo {
    pub selector: String,
    pub name: String,
    pub backend: String,
    pub device_type: String,
    pub driver: String,
    pub driver_info: String,
}

impl GraphicsAdapterInfo {
    pub fn from_wgpu(info: &wgpu::AdapterInfo) -> Self {
        let backend = format!("{:?}", info.backend);
        Self {
            selector: adapter_selector(info),
            name: info.name.clone(),
            backend,
            device_type: format!("{:?}", info.device_type),
            driver: info.driver.clone(),
            driver_info: info.driver_info.clone(),
        }
    }

    pub fn label(&self) -> String {
        format!("{} ({}, {})", self.name, self.backend, self.device_type)
    }
}

fn adapter_selector(info: &wgpu::AdapterInfo) -> String {
    format!("{:?}:{}", info.backend, info.name)
}

fn select_adapter(
    adapters: &[wgpu::Adapter],
    surface: Option<&wgpu::Surface<'_>>,
    requested: &str,
    power: GraphicsPowerPreference,
) -> Result<wgpu::Adapter, String> {
    let compatible = adapters
        .iter()
        .filter(|adapter| surface.is_none_or(|surface| adapter.is_surface_supported(surface)))
        .collect::<Vec<_>>();

    if let Some(adapter) = compatible
        .iter()
        .find(|adapter| adapter_selector(&adapter.get_info()).eq_ignore_ascii_case(requested))
    {
        let info = adapter.get_info();
        info!(
            adapter = %info.name,
            backend = ?info.backend,
            device_type = ?info.device_type,
            "Using session-selected graphics adapter"
        );
        return Ok((*adapter).clone());
    }

    let fallback = compatible.into_iter().max_by_key(|adapter| {
        let info = adapter.get_info();
        (
            device_type_score(info.device_type, power),
            backend_score(info.backend),
        )
    });

    let Some(adapter) = fallback else {
        return Err("no graphics adapter can present to the QSONaut window".to_string());
    };
    let info = adapter.get_info();
    warn!(
        requested,
        fallback = %info.name,
        backend = ?info.backend,
        device_type = ?info.device_type,
        "Requested graphics adapter is unavailable; using policy fallback"
    );
    Ok(adapter.clone())
}

fn device_type_score(device_type: wgpu::DeviceType, power: GraphicsPowerPreference) -> u8 {
    match (power, device_type) {
        (GraphicsPowerPreference::Low, wgpu::DeviceType::IntegratedGpu) => 5,
        (GraphicsPowerPreference::Low, wgpu::DeviceType::DiscreteGpu) => 4,
        (GraphicsPowerPreference::High, wgpu::DeviceType::DiscreteGpu) => 5,
        (GraphicsPowerPreference::High, wgpu::DeviceType::IntegratedGpu) => 4,
        (_, wgpu::DeviceType::VirtualGpu) => 3,
        (_, wgpu::DeviceType::Other) => 2,
        (_, wgpu::DeviceType::Cpu) => 1,
    }
}

fn backend_score(backend: wgpu::Backend) -> u8 {
    match backend {
        wgpu::Backend::Metal | wgpu::Backend::Dx12 | wgpu::Backend::Vulkan => 3,
        wgpu::Backend::Gl => 2,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn low_power_prefers_integrated_gpu() {
        assert!(
            device_type_score(
                wgpu::DeviceType::IntegratedGpu,
                GraphicsPowerPreference::Low
            ) > device_type_score(wgpu::DeviceType::DiscreteGpu, GraphicsPowerPreference::Low)
        );
    }

    #[test]
    fn high_performance_prefers_discrete_gpu() {
        assert!(
            device_type_score(wgpu::DeviceType::DiscreteGpu, GraphicsPowerPreference::High)
                > device_type_score(
                    wgpu::DeviceType::IntegratedGpu,
                    GraphicsPowerPreference::High
                )
        );
    }
}
