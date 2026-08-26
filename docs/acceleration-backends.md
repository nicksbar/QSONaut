# Compute acceleration

QSONaut treats acceleration as a measured backend choice, not a build-time promise. CPU decoding remains the reference implementation and unconditional fallback.

## Current foundation

`qsonaut-accelerate` provides:

- `AUTO`, `CPU`, and `GPU` operator preferences;
- runtime CPU SIMD and logical-thread discovery;
- NVIDIA CUDA device discovery through `nvidia-smi`, including WSL projection;
- Linux/WSL NPU device exposure checks;
- per-stage decode timing and real-time slot-budget reporting;
- a benchmark gate that requires matching output digests and a configurable minimum end-to-end speedup.

GUI rendering adapters are not treated as proof of compute capability. On WSL,
QSONaut checks the Windows-projected NVIDIA runtime with `nvidia-smi`, so CUDA
compute discovery stays separate from the GPU that presents the desktop UI.

The GUI publishes its selected backend to decode workers. FT8 currently records PCM preparation, protocol decoding, and result-unpacking stages. Other native modes record their protocol decode stage. Station Health shows the latest timing as a percentage of that mode's slot.

## WSL GUI rendering

GUI rendering and decoder compute are separate acceleration paths. QSONaut
uses eframe's WGPU setup to create one graphics instance and select an adapter
that can present to the real application window. The session default is low
power with automatic adapter selection. **Settings > Graphics** displays the
active and available adapters and can stage another policy or GPU for a GUI
restart without persisting it. The requested adapter is validated against the
real window surface during that restart.

On WSLg with `/dev/dxg`, QSONaut defaults Mesa to its D3D12 Gallium driver and
WGPU to GL. Native Linux keeps WGPU's Vulkan and GL fallback set. Explicit
`WGPU_BACKEND`, `GALLIUM_DRIVER`, and `MESA_D3D12_DEFAULT_ADAPTER_NAME` values
are preserved. Native Windows currently defaults to WGPU GL because the WGPU
27 DX12 allocator conflicts with the Windows bindings required by the audio
stack; Vulkan remains an explicit override.

The adapter inventory is a startup snapshot. If a laptop disables a discrete
or dock GPU while QSONaut is running, restart the GUI to rebuild the device and
surface. If a previously selected GPU is absent during that restart, QSONaut
falls back to a compatible adapter using the selected power policy.

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
