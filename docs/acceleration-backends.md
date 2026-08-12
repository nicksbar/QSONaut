# Compute acceleration

RigForge treats acceleration as a measured backend choice, not a build-time promise. CPU decoding remains the reference implementation and unconditional fallback.

## Current foundation

`rigforge-accelerate` provides:

- `AUTO`, `CPU`, and `GPU` operator preferences;
- runtime CPU SIMD and logical-thread discovery;
- real `wgpu` Vulkan adapter enumeration;
- Linux/WSL GPU and NPU device exposure checks;
- per-stage decode timing and real-time slot-budget reporting;
- a benchmark gate that requires matching output digests and a configurable minimum end-to-end speedup.

Software Vulkan adapters such as llvmpipe, lavapipe, and SwiftShader are
reported as CPU fallbacks and never count as an available GPU. On WSL,
RigForge also checks the Windows-projected NVIDIA runtime with `nvidia-smi`,
so a CUDA-capable device remains visible even when Vulkan is software-only.

The GUI publishes its selected backend to decode workers. FT8 currently records PCM preparation, protocol decoding, and result-unpacking stages. Other native modes record their protocol decode stage. Station Health shows the latest timing as a percentage of that mode's slot.

## WSL GUI rendering

GUI rendering and decoder compute are separate acceleration paths. On WSL,
RigForge automatically requests Mesa's D3D12 Gallium renderer through
`/dev/dxg` and prefers an AMD adapter for the desktop UI. This avoids
llvmpipe's CPU software renderer and keeps the discrete NVIDIA GPU asleep for
ordinary display work. Explicit `GALLIUM_DRIVER` and
`MESA_D3D12_DEFAULT_ADAPTER_NAME` values are always preserved.

Automatic digital-mode waterfalls update at 10 rows per second. Texture
uploads occur only after new waterfall data arrives or its visible bandwidth
changes; audio collection and timed decoding continue independently.

## Safety policy

GPU kernels are not validated yet, so both `AUTO` and an explicit `GPU` request fall back to CPU SIMD. The UI explains why. This is intentional: detecting an adapter is not proof that a kernel is correct, faster after transfer overhead, or stable enough for a timed radio protocol.

A GPU implementation becomes eligible only when it:

1. matches the CPU output digest across the decode fixture suite;
2. preserves decoded messages, timing, frequency, and SNR within defined tolerances;
3. beats the CPU path end-to-end by the configured speedup floor;
4. stays within the mode's real-time slot budget;
5. survives device loss and cleanly falls back to CPU.

## Intended first GPU workload

The first useful kernel should batch overlapping spectrogram FFTs and coarse candidate scoring. Small display FFTs, audio resampling, and individual FEC decodes should remain on the CPU unless profiling proves otherwise. Persistent device buffers and batched dispatch are required to amortize upload and synchronization costs.

The NPU is reserved for genuinely neural workloads such as experimental denoising or signal-presence classification. It is not the default target for deterministic WSJT-family FFT/FEC processing.
