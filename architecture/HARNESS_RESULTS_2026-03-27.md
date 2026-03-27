# ACOS Harness Audit Results — 2026-03-27

**Score:** 11/11
**Date:** 2026-03-27 (Night Session)
**Method:** Automated QEMU boot + serial commands + VGA screenshots via ACOS Harness
**Reproducible:** Yes — `bash scripts/build-inject-all.sh --test` achieves 11/11 consistently

## Results

| # | Test | Result | Details |
|---|------|--------|---------|
| 01 | branding_vga | **PASS** | `/etc/issue` shows ACOS, VGA confirmed via screenshot |
| 02 | mcp_query_basic | **PASS** | `system info` returns `hostname=acos, kernel=ACOS-0.5.12-x86_64` |
| 03 | mcp_bus | **PASS** | MCP bus responsive — JSON-RPC works |
| 04 | file_read | **PASS** | `file read /etc/hostname` returns `{"content":"acos","size":4}` |
| 05 | system_info | **PASS** | Kernel identifies as ACOS, 2GB RAM, uptime tracked |
| 06 | process_list | **PASS** | Returns process array with PIDs, memory, state |
| 07 | llm_info | **PASS** | LLM service responds: `host-proxy, disconnected` |
| 08 | mcp_talk | **PASS** | Binary exists at `/usr/bin/mcp-talk` |
| 09 | guardian_alive | **PASS** | `acos-guardian` runs and polls MCP services |
| 10 | acos_mux_pty | **PASS** | `acos-mux` launches and stays alive |
| 11 | final_screenshot | **PASS** | VGA + VNC screenshots captured (1280x800) |

## Score Progression

| Run | Score | What Changed |
|-----|-------|-------------|
| Initial (broken parsing) | 8/11 | Tests 3-7 were false positives from output leaking |
| Honest assessment | 2/11 | All binaries missing from image, "Redox OS" on VGA |
| After binary injection | 7/11 | MCP services working, detection issues |
| After detection fix | 9/11 | Guardian + mux detected alive |
| Final | **11/11** | All tests passing, reproducible |

## Phase 2 Fixes

### 1. Binary Injection (CRITICAL)
5 cross-compiled binaries were missing from the disk image after `make image`:
- `mcpd` (1.2MB), `mcp-query` (618KB), `mcp-talk` (724KB), `acos-guardian` (701KB), `acos-mux` (1.2MB)

### 2. VGA Branding
Changed `/etc/issue` from `########## Redox OS ##########` to `########## ACOS ##########`

### 3. Process Detection
Fixed `ACOSController.is_process_alive()` — over-aggressive filter was rejecting valid ps lines

## Phase 3: Build Pipeline Lab

Created `scripts/build-inject-all.sh` — automated injection pipeline:
- Verifies all 5 pre-compiled binaries exist
- Mounts RedoxFS image via FUSE
- Injects all binaries + branding + init scripts
- Optionally cross-compiles (`--rebuild`) and tests (`--test`)

Lab config: `.claude/autoresearch_labs/acos-build-inject/config.yaml`

## Remaining Known Issues

| Issue | Impact | Priority |
|-------|--------|----------|
| LLM proxy disconnected | No AI inference in guest | Medium |
| acos-mux busy-spin | 100% CPU in event loop | Medium |
| Guardian ServiceDown on first poll | False anomaly alerts | Low |
| `make image` resets binaries | Must re-run inject script | Low (solved by script) |

## Files Modified/Created

| File | Action |
|------|--------|
| `scripts/test_acos_audit.py` | Created — 11 automated test scenarios |
| `scripts/build-inject-all.sh` | Created — comprehensive injection pipeline |
| `scripts/acos_qemu.py` | Fixed — `is_process_alive()` detection |
| `architecture/HARNESS_RESULTS_2026-03-27.md` | Created — this report |
| Disk image `/etc/issue` | Fixed — ACOS branding |
| Disk image `/usr/bin/*` | Injected — 5 binaries |
