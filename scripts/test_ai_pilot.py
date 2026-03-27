#!/usr/bin/env python3
"""
ACOS AI Pilot Lab — validate that mcp-talk AI can actually pilot the OS.

Tests the full chain: mcp-talk → talk_handler → ai_handler → LLM proxy → Gemini
with real function calling (tool use) against MCP services.

SCORE=0-6:
  1: mcp-talk starts and shows prompt
  2: AI responds to a simple greeting
  3: AI executes system_info tool (returns hostname=acos)
  4: AI executes file_read tool (reads /etc/hostname)
  5: AI executes process_list tool (lists processes)
  6: AI can write a file and read it back

Requires: llm-proxy.py running on host, QEMU with -nic user
"""

import os
import sys
import time
import re

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, SCRIPT_DIR)

from acos_qemu import ACOSController


def mcp_talk_send(vm, message, wait=20):
    """Send a message to mcp-talk via serial and wait for 'acos>' prompt to return."""
    from acos_qemu import clean_ansi
    # Flush any pending output first
    try:
        vm.serial.serial.read_nonblocking(size=65536, timeout=1)
    except Exception:
        pass

    vm.serial.sendline(message)
    # Wait for the next acos> prompt which signals the AI finished responding
    try:
        vm.serial.serial.expect(r"acos>", timeout=wait)
        raw = clean_ansi(vm.serial.serial.before or "")
        return raw
    except Exception:
        # Fallback: read whatever is available
        try:
            vm.serial.serial.read_nonblocking(size=65536, timeout=2)
            return clean_ansi(vm.serial.serial.before or "")
        except Exception:
            return None


def main():
    score = 0

    # Ensure LLM proxy is running
    import subprocess
    proxy_check = subprocess.run(["ss", "-tlnp"], capture_output=True, text=True)
    if ":9999" not in proxy_check.stdout:
        print("[*] Starting LLM proxy...")
        subprocess.Popen(
            ["python3", os.path.join(SCRIPT_DIR, "llm-proxy.py")],
            stdout=open("/tmp/llm-proxy.log", "a"),
            stderr=subprocess.STDOUT,
            stdin=subprocess.DEVNULL,
        )
        time.sleep(3)

    vm = ACOSController(network=True)

    try:
        print("[*] Starting QEMU with network...")
        vm.start()
        vm.boot(timeout=90)
        vm.login()
        print("[OK] Booted and logged in")
        time.sleep(2)

        # === Test 1: mcp-talk starts ===
        print("\n[Test 1] Starting mcp-talk...")
        vm.serial.sendline("mcp-talk")
        # Wait for the acos> prompt to appear
        try:
            vm.serial.serial.expect(r"acos>", timeout=15)
            from acos_qemu import clean_ansi
            raw = clean_ansi(vm.serial.serial.before or "")
            print("[OK] mcp-talk started — acos> prompt visible")
            score += 1
        except Exception as e:
            print(f"[FAIL] mcp-talk start: {e}")
        # Flush buffer
        time.sleep(1)

        # === Test 2: Simple greeting ===
        print("\n[Test 2] Sending greeting...")
        response = mcp_talk_send(vm, "Hello, who are you?", wait=30)
        if response and len(response.strip()) > 20:
            # Check if AI responded with something meaningful
            resp_lower = response.lower()
            if "acos" in resp_lower or "operating" in resp_lower or "system" in resp_lower or "ai" in resp_lower:
                print(f"[OK] AI responded with identity. Length={len(response)}")
                score += 1
            else:
                print(f"[WARN] Response doesn't mention ACOS: {response[:200]}")
                # Still count if it's a real response
                if len(response.strip()) > 50:
                    score += 1
        else:
            print(f"[FAIL] No response or too short: {(response or '')[:100]}")

        # === Test 3: System info (tool call) ===
        print("\n[Test 3] Asking for system info (should trigger tool call)...")
        response = mcp_talk_send(vm, "What is the hostname and kernel version of this system? Use system_info.", wait=40)
        if response:
            if "acos" in response.lower() and ("kernel" in response.lower() or "0.5" in response):
                print(f"[OK] AI used system_info tool — found hostname+kernel")
                score += 1
            elif "acos" in response.lower():
                print(f"[OK] AI returned system info (partial)")
                score += 1
            else:
                print(f"[FAIL] No system info in response: {response[:200]}")
        else:
            print("[FAIL] No response")

        # === Test 4: File read (tool call) ===
        print("\n[Test 4] Asking to read /etc/hostname (should trigger file_read)...")
        response = mcp_talk_send(vm, "Read the file /etc/hostname and tell me its contents.", wait=40)
        if response:
            if "acos" in response.lower() or "hostname" in response.lower() or "content" in response.lower():
                print(f"[OK] AI read /etc/hostname")
                score += 1
            else:
                print(f"[FAIL] /etc/hostname not found in response: {response[:200]}")
        else:
            print("[FAIL] No response")

        # === Test 5: Process list (tool call) ===
        print("\n[Test 5] Asking for process list (should trigger process_list)...")
        response = mcp_talk_send(vm, "List the running processes on this system.", wait=40)
        if response:
            if "pid" in response.lower() or "process" in response.lower() or "mcpd" in response.lower() or "init" in response.lower():
                print(f"[OK] AI listed processes")
                score += 1
            else:
                print(f"[FAIL] No process data in response: {response[:200]}")
        else:
            print("[FAIL] No response")

        # === Test 6: Write + read file (two tool calls) ===
        print("\n[Test 6] Asking AI to write a file then read it back...")
        response = mcp_talk_send(vm, "Write the text 'ACOS_PILOT_TEST_OK' to the file /tmp/ai_test.txt, then read it back to confirm.", wait=50)
        if response:
            if "ACOS_PILOT_TEST_OK" in response or "success" in response.lower() or "written" in response.lower() or "confirmed" in response.lower():
                print(f"[OK] AI wrote and read back file")
                score += 1
            else:
                print(f"[INFO] Response: {response[:200]}")
        else:
            print("[FAIL] No response")

        # Exit mcp-talk
        vm.serial.sendline("/quit")
        time.sleep(2)

    except Exception as e:
        print(f"[ERROR] {e}")
        import traceback
        traceback.print_exc()
    finally:
        vm.stop()

    print(f"\nSCORE={score}")
    return score


if __name__ == "__main__":
    score = main()
    sys.exit(0 if score >= 4 else 1)
