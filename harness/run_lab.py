#!/usr/bin/env python3
"""
AutoResearch Agent Launcher — generates and launches autonomous Claude Code sessions.

Usage:
  python3 harness/run_lab.py --lab <lab_id> --budget <N>
  python3 harness/run_lab.py --lab <lab_id> --budget <N> --dry-run
  python3 harness/run_lab.py --lab <lab_id> --budget <N> --resume
"""

import argparse
import glob
import os
import shutil
import subprocess
import sys

sys.path.insert(0, os.path.dirname(__file__))
from parse_lab import load_lab

BASE_DIR = os.path.join(os.path.dirname(__file__), "..")
RESULTS_DIR = os.path.join(BASE_DIR, "evolution", "results")
MEMORY_DIR = os.path.join(BASE_DIR, "evolution", "memory")


def read_last_tsv_lines(lab_id, n=5):
    """Return last N lines from the results TSV, or empty string."""
    tsv_path = os.path.join(RESULTS_DIR, f"{lab_id}.tsv")
    if not os.path.exists(tsv_path):
        return ""
    with open(tsv_path, "r") as f:
        lines = [l.rstrip() for l in f if l.strip()]
    return "\n".join(lines[-n:])


def find_last_round_memory(lab_id):
    """Return (round_number, content) of highest-numbered round memory, or (0, '')."""
    pattern = os.path.join(MEMORY_DIR, f"{lab_id}_round_*.md")
    matches = glob.glob(pattern)
    if not matches:
        return 0, ""

    def round_num(path):
        base = os.path.basename(path)
        try:
            return int(base.replace(f"{lab_id}_round_", "").replace(".md", ""))
        except ValueError:
            return 0

    last = max(matches, key=round_num)
    n = round_num(last)
    with open(last, "r") as f:
        content = f.read()
    return n, content


def determine_start_round(lab_id):
    """Determine starting round from TSV (last completed round + 1), or 1."""
    tsv_path = os.path.join(RESULTS_DIR, f"{lab_id}.tsv")
    if not os.path.exists(tsv_path):
        return 1
    last_round = 0
    with open(tsv_path, "r") as f:
        for line in f:
            parts = line.strip().split("\t")
            if len(parts) >= 2:
                try:
                    r = int(parts[1])
                    if r > last_round:
                        last_round = r
                except ValueError:
                    pass
    return last_round + 1 if last_round > 0 else 1


def generate_prompt(lab, lab_id, budget, start_round, tsv_lines, memory_content):
    """Generate the autonomous agent prompt."""
    metric = lab.get("metric", {})
    metric_name = metric.get("name", "metric")
    metric_unit = metric.get("unit", "")
    metric_target = metric.get("target", "unknown")
    description = lab.get("description", lab_id)
    component = lab.get("component", "unknown")
    allowed_files = lab.get("allowed_files", [])
    allowed_str = "\n".join(f"  - {f}" for f in allowed_files) if allowed_files else "  (see lab config)"

    if tsv_lines:
        prev_results = "```\ntimestamp\tround\tmetric\tstatus\tnotes\n" + tsv_lines + "\n```"
    else:
        prev_results = "No previous results."

    if memory_content:
        prev_memory = memory_content.strip()
    else:
        prev_memory = "No previous memory — this is the first run."

    return f"""You are an AutoResearch agent. Your mission: optimize {lab_id}.

Lab: {description}
Metric: {metric_name} ({metric_unit})
Target: {metric_target}
Budget: {budget} iterations (starting from round {start_round})
Allowed files:
{allowed_str}
Component directory: components/{component}/

## Previous Results
{prev_results}

## Previous Memory
{prev_memory}

## Your Loop (repeat until target met or budget exhausted):

1. ANALYZE previous results. What pattern do you see?
2. HYPOTHESIZE: "If I change X, metric Y should improve because Z"
3. MODIFY: Edit files in components/{component}/ (ONLY allowed files). Make ONE focused change.
4. RUN: bash harness/autoresearch.sh {lab_id} {{round}}
5. READ the output line: AUTORESEARCH_RESULT:metric=VALUE,status=...
6. RECORD: Update evolution/memory/{lab_id}_round_{{N}}.md with what you changed and why
7. DECIDE:
   - If status=target_met → STOP, write success summary
   - If status=pass (improved or stable) → keep changes, continue
   - If status=regression → changes were rolled back automatically, try different approach
   - If budget exhausted → STOP, write summary of best result

## Rules:
- ONE change per iteration. Small, measurable, reversible.
- Always read the file BEFORE editing.
- Never skip running autoresearch.sh — no guessing if it works.
- If stuck after 3 regressions in a row, step back and rethink approach.
- Never modify files outside allowed_files list.
- ACOS naming: this OS is called ACOS, never "Redox".
"""


def main():
    parser = argparse.ArgumentParser(
        description="AutoResearch Agent Launcher — generates and launches autonomous Claude Code sessions."
    )
    parser.add_argument("--lab", required=True, help="Lab config ID to run")
    parser.add_argument("--budget", type=int, default=10, help="Iteration budget (default: 10)")
    parser.add_argument("--dry-run", action="store_true", help="Print generated prompt without launching")
    parser.add_argument("--resume", action="store_true", help="Resume from last completed round")
    args = parser.parse_args()

    lab_id = args.lab
    budget = args.budget

    try:
        lab = load_lab(lab_id)
    except FileNotFoundError as e:
        print(f"ERROR: {e}", file=sys.stderr)
        sys.exit(1)

    tsv_lines = read_last_tsv_lines(lab_id)
    last_round, memory_content = find_last_round_memory(lab_id)

    if args.resume:
        start_round = determine_start_round(lab_id)
    else:
        start_round = 1

    prompt = generate_prompt(lab, lab_id, budget, start_round, tsv_lines, memory_content)

    if args.dry_run:
        print(prompt)
        sys.exit(0)

    claude_bin = shutil.which("claude")
    if claude_bin:
        print(f"Launching Claude Code session for lab: {lab_id}")
        print(f"Budget: {budget} iterations, starting from round {start_round}")
        print()
        result = subprocess.run(
            [claude_bin, "--print", "--dangerously-skip-permissions", "-p", prompt],
            cwd=os.path.join(BASE_DIR),
        )
        print()
        print(f"Session complete. Check evolution/labs/{lab_id}_summary.md for results.")
        sys.exit(result.returncode)
    else:
        print("claude CLI not found. Paste the following prompt into Claude Code:\n")
        print("=" * 72)
        print(prompt)
        print("=" * 72)
        print()
        print(f"After the session, check evolution/labs/{lab_id}_summary.md for results.")
        sys.exit(0)


if __name__ == "__main__":
    main()
