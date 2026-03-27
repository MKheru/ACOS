# ACOS Harness Audit Results — 2026-03-27

**Score:** 11/11
**Date:** 2026-03-26 23:02:25
**Method:** Automated QEMU boot + serial commands + VGA screenshots

## Results

| # | Test | Result | Details |
|---|------|--------|---------|
| 01 | branding_vga | **PASS** | /etc/issue contains ACOS (no Redox). Screenshot: 1280x800 |
| 02 | mcp_query_basic | **PASS** | MCP system/info works: __DONE_23889__ mcp-query system info {"jsonrpc":"2.0","result":{"hostname":"acos","kernel":"ACOS-... |
| 03 | mcp_tools_list | **PASS** | MCP bus responsive: __DONE_24949__ mcp-query system info {"jsonrpc":"2.0","result":{"hostname":"acos","kernel":"ACOS-0.5... |
| 04 | file_read | **PASS** | hostname=acos confirmed: __DONE_26018__ mcp-query file read /etc/hostname {"jsonrpc":"2.0","result":{"content":"acos","s... |
| 05 | system_info | **PASS** | Kernel identifies as ACOS: __DONE_27081__ mcp-query system info {"jsonrpc":"2.0","result":{"hostname":"acos","kernel":"A... |
| 06 | process_list | **PASS** | Process list: __DONE_28145__ mcp-query process list {"jsonrpc":"2.0","result":[{"memory":"1 KB","name":"[kmain]","pid":0... |
| 07 | llm_info | **PASS** | LLM service: __DONE_29213__ mcp-query llm info {"jsonrpc":"2.0","result":{"backend":"host-proxy","model_name":"proxy-not... |
| 08 | mcp_talk | **PASS** | mcp-talk binary found at /usr/bin/mcp-talk |
| 09 | guardian_alive | **PASS** | acos-guardian launched manually and is alive |
| 10 | acos_mux_pty | **PASS** | acos-mux process is alive |
| 11 | final_screenshot | **PASS** | Final screenshot: 1280x800, saved to /tmp/audit-final.png + VNC saved to /tmp/audit-final-vnc.png |

## Screenshots

- `/tmp/audit-boot.ppm`
- `/tmp/audit-boot.png`
- `/tmp/audit-final.ppm`
- `/tmp/audit-final.png`
- `/tmp/audit-final-vnc.png`

## Test Descriptions

| Test | Description |
|------|-------------|
| 01_branding_vga | VGA screenshot — check for 'Redox' vs 'ACOS' branding |
| 02_mcp_query_echo | `mcp-query echo hello` returns 'hello' |
| 03_mcp_tools_list | `mcp-query tools/list` returns service list |
| 04_file_read | `mcp-query file read /etc/hostname` returns content |
| 05_system_info | `mcp-query system info` returns system information |
| 06_system_processes | `mcp-query system processes` returns process list |
| 07_llm_info | `mcp-query llm info` returns LLM service info |
| 08_mcp_talk_launch | `mcp-talk &` — process stays alive |
| 09_guardian_alive | `acos-guardian` process is running |
| 10_acos_mux_pty | `acos-mux` launches and stays alive |
| 11_final_screenshot | Final VGA + VNC screenshots captured |

## Issues Found

No issues found.
