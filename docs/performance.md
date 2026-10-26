# Performance

Local measurements on **19 October 2026**, using an Apple M4 Pro (14 cores, 24 GiB RAM),
Google DHU 2.1 for ARM64, and FFmpeg 9.0.2. Both head units used the same Android emulator over
ADB TCP at **800×480, 30 FPS requested, and 111 DPI**, with no media playing.

The default-viewer comparison measures the release viewer before and after limiting FFmpeg's decoder,
color-conversion and rawvideo-output workers, moving the opaque RGBA buffer into egui without
another pixel copy, and reusing texture storage between frames. The baseline is commit `9ec7d69`;
the optimized viewer includes the changes following `569544f`.

Each result averages two 30-second samples, switching between the launcher and dashboard every
two seconds. Runs used the order baseline, optimized, Google, Google, optimized, baseline.
The dashboard showed Maps' location-permission placeholder, so this measures UI transitions
rather than active navigation.

| Build | Interacting CPU | CPU range across runs | Interacting RAM |
|---|---:|---:|---:|
| Google DHU 2.1 | 4.2% | 4.2–4.3% | 91 MiB |
| openDHU release, before | 16.4% | 14.9–17.9% | 152 MiB |
| openDHU release, optimized | 11.8% | 8.8–14.7% | 141 MiB |

The optimized viewer averaged **28% less CPU and 7% less RSS** in these fresh matched runs.
Google's GUI still used less CPU and memory. Run-to-run variation was substantial; these short
local measurements are not cross-platform guarantees.

**100% CPU means one logical core.** CPU is measured from interval CPU-time deltas. RAM is summed
resident set size (RSS); openDHU totals include the daemon, native head unit, GUI, and FFmpeg decoder.
The shared emulator, ADB, WindowServer, benchmark controller, startup, and screenshot capture are excluded.
Default desktop window sizes were retained.

Both openDHU variants reported decoder progress once per second: the two baseline windows counted
1,696 decoded frames in total, versus 1,717 after optimization. These approximate counters do not
measure displayed FPS or frame drops. An offline check also decoded 60 motion frames at each of
800×480, 1920×1080, and 2560×1440 with byte-identical RGBA output before and after thread tuning.
On this Mac, each tuned decode exceeded 30 FPS; throughput on slower hardware remains unverified.

An earlier comparison established the cost of installing debug builds. Its two-sample averages
are recorded separately because timings and memory use vary between runs:

| Build | Idle CPU | Interacting CPU | Interacting RAM |
|---|---:|---:|---:|
| Google DHU 2.1 | 1.6% | 4.5% | 92 MiB |
| openDHU debug | 1.8% | 38.3% | 175 MiB |
| openDHU release, before | 0.7% | 11.1% | 172 MiB |

Release compilation reduced interaction CPU use by **71%** in that earlier comparison.
`make install` now installs release binaries; `make install-debug` is available for development.

With no viewer attached, one additional 30-second idle sample per backend gave:

| Headless backend | CPU | RAM |
|---|---:|---:|
| Google DHU 2.1 | 1.33% | 79 MiB |
| openDHU release | 0.03% | 17 MiB |

Both headless sessions retained video focus and projected the launcher; Google's headless video focus
was explicitly enabled. RSS can count shared pages more than once and does not capture compressed
memory or GPU allocations. Display latency, energy use, and GPU utilization were not measured.

## GPU color conversion

The GUI uses a YUV420 shader path by default on macOS and Linux. FFmpeg decodes H.264 on the CPU;
OpenGL converts the compact YUV planes to RGB while drawing the fitted video rectangle. At 800×480
and 30 FPS, raw pixel traffic falls from **46.1 MB/s to 17.3 MB/s**. GPU initialization or rendering
failures automatically fall back to RGBA. Use `OPENDHU_GPU_VIDEO=0 aa gui` to force the RGBA path.

A separate fresh comparison against release commit `42c3f63`, using the same hardware and workload
above, averaged two 30-second samples per mode in the order RGBA, YUV, YUV, RGBA:

| Viewer | Interacting CPU | CPU range | Interacting RSS |
|---|---:|---:|---:|
| Normal RGBA | 9.9% | 9.7–10.1% | 175 MiB |
| YUV shader (default) | 9.6% | 9.5–9.7% | 161 MiB |

The observed means were **3% lower CPU and 8% lower RSS**. The CPU change is small, and RSS varied
between runs (164–185 MiB for RGBA, 161 MiB for YUV); this is not evidence of a large speedup.
Approximate decoder counters totaled 1,715 frames for RGBA and 1,786 for YUV. The same measurement
exclusions apply; GPU memory, utilization, energy, displayed FPS and latency were not measured.
These results belong to this comparison and should not be combined with the earlier tables.

The shader uses the existing portable OpenGL renderer. Mac color-bar checks covered BT.601,
BT.709 and full-range input; average rendered RGB differences from the normal viewer were
0.4–1.1 levels out of 255. Linux ARM64 rendering was also checked with an H.264 fixture under
Xvfb/Mesa. Chroma interpolation and rounding differ, so the rendered output is not byte-identical.
See [GUI verification](gui.md#verification-and-limits) for platform coverage.
