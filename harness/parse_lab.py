#!/usr/bin/env python3
"""
ACOS AutoResearch Lab Config Parser

Parses YAML lab configuration files and provides CLI actions
for the AutoResearch framework.

Usage:
    python3 harness/parse_lab.py <lab_id> validate
    python3 harness/parse_lab.py <lab_id> autotest
    python3 harness/parse_lab.py <lab_id> check <value>
    python3 harness/parse_lab.py <lab_id> allowed_files
"""

import sys
import os
import re

LABS_DIR = os.path.join(os.path.dirname(__file__), "..", "evolution", "labs")


def _manual_yaml_parse(text):
    """Minimal YAML parser for our lab config subset (flat + one-level lists)."""
    result = {}
    current_key = None
    current_list = None
    current_dict_key = None
    indent_stack = []

    lines = text.splitlines()
    i = 0
    while i < len(lines):
        line = lines[i]
        stripped = line.rstrip()
        if not stripped or stripped.lstrip().startswith("#"):
            i += 1
            continue

        indent = len(line) - len(line.lstrip())
        content = stripped.lstrip()

        # List item
        if content.startswith("- "):
            value = content[2:].strip().strip('"').strip("'")
            if current_list is not None:
                current_list.append(value)
            i += 1
            continue

        # Key: value pair
        if ":" in content:
            colon_idx = content.index(":")
            key = content[:colon_idx].strip()
            value = content[colon_idx + 1:].strip().strip('"').strip("'")
            # Strip inline comment
            if " #" in value:
                value = value[:value.index(" #")].strip()

            if indent == 0:
                current_dict_key = None
                if value == "" or value is None:
                    # Could be a mapping or list block
                    # Peek ahead to determine
                    if i + 1 < len(lines):
                        next_content = lines[i + 1].lstrip()
                        if next_content.startswith("- "):
                            lst = []
                            result[key] = lst
                            current_list = lst
                            current_key = key
                        else:
                            d = {}
                            result[key] = d
                            current_list = None
                            current_dict_key = key
                    else:
                        result[key] = None
                else:
                    result[key] = value
                    current_list = None
            elif indent > 0 and current_dict_key is not None:
                parent = result.get(current_dict_key)
                if isinstance(parent, dict):
                    if value == "":
                        # Nested block — peek ahead
                        if i + 1 < len(lines):
                            next_content = lines[i + 1].lstrip()
                            if next_content.startswith("- "):
                                lst = []
                                parent[key] = lst
                                current_list = lst
                            else:
                                pass  # nested dict depth not needed
                        else:
                            parent[key] = None
                    else:
                        parent[key] = value
                        current_list = None
        i += 1

    return result


def load_lab(lab_id):
    """Load YAML from evolution/labs/{lab_id}.yaml, return dict."""
    if not re.match(r'^[a-zA-Z0-9_-]+$', lab_id):
        raise ValueError(f"Invalid lab_id: {lab_id!r}")
    lab_path = os.path.join(LABS_DIR, f"{lab_id}.yaml")
    resolved = os.path.realpath(lab_path)
    labs_real = os.path.realpath(LABS_DIR)
    if not resolved.startswith(labs_real + os.sep):
        raise ValueError(f"Path traversal detected for lab_id: {lab_id!r}")
    if not os.path.exists(lab_path):
        raise FileNotFoundError(f"Lab config not found: {lab_path}")

    with open(lab_path, "r") as f:
        content = f.read()

    try:
        import yaml
        return yaml.safe_load(content)
    except ImportError:
        return _manual_yaml_parse(content)


def generate_autotest_script(lab):
    """Generate init script content for QEMU injection."""
    metric_name = lab.get("metric", {}).get("name", "metric")
    boot_commands = lab.get("qemu_test", {}).get("boot_commands", [])

    lines = [
        "#!/bin/sh",
        "# AutoResearch generated init script",
        "requires_weak 15_mcp",
        "",
        "# Wait for mcpd to be ready",
        "sleep 2",
        "",
    ]

    shell_metacharacters = set('$`;&|><')
    for cmd in boot_commands:
        if any(c in cmd for c in shell_metacharacters):
            raise ValueError(f"Shell metacharacter in boot_command: {cmd!r}")
        lines.append(f"# Run test command")
        lines.append(f"_result=$({cmd})")
        lines.append(f'echo "AUTORESEARCH_METRIC:{metric_name}=$_result"')
        lines.append("")

    lines.append('echo "AUTORESEARCH_DONE"')

    return "\n".join(lines) + "\n"


def extract_metric(serial_log_path, lab):
    """Parse serial log file and extract numeric metric value."""
    metric_name = lab.get("metric", {}).get("name", "metric")
    marker = f"AUTORESEARCH_METRIC:{metric_name}="

    if not os.path.exists(serial_log_path):
        return None

    with open(serial_log_path, "r", errors="replace") as f:
        for line in f:
            if marker in line:
                idx = line.index(marker) + len(marker)
                remainder = line[idx:].strip()
                # Extract leading numeric value
                match = re.match(r"[-+]?\d*\.?\d+", remainder)
                if match:
                    return float(match.group())

    return None


def check_target(value, target_expr):
    """Evaluate target expression like '>= 95', '< 16', '== 100'."""
    target_expr = str(target_expr).strip()
    match = re.match(r"^(<=|>=|==|!=|<|>)\s*([\d.]+)$", target_expr)
    if not match:
        raise ValueError(f"Invalid target expression: {target_expr!r}")

    op, threshold_str = match.group(1), match.group(2)
    threshold = float(threshold_str)
    value = float(value)

    ops = {
        "<": lambda a, b: a < b,
        "<=": lambda a, b: a <= b,
        ">": lambda a, b: a > b,
        ">=": lambda a, b: a >= b,
        "==": lambda a, b: a == b,
        "!=": lambda a, b: a != b,
    }
    return ops[op](value, threshold)


def get_allowed_files(lab):
    """Return list of allowed files from config."""
    entries = lab.get("allowed_files", [])
    for entry in entries:
        if '..' in entry.split('/'):
            raise ValueError(f"Path traversal in allowed_files entry: {entry!r}")
    return entries


def _cmd_validate(lab_id):
    try:
        lab = load_lab(lab_id)
    except FileNotFoundError as e:
        print(str(e), file=sys.stderr)
        return 1

    required = ["lab_id", "description", "metric", "component"]
    missing = [k for k in required if k not in lab]

    if missing:
        print(f"ERROR: Missing required fields: {', '.join(missing)}", file=sys.stderr)
        return 1

    print(f"lab_id:      {lab.get('lab_id')}")
    print(f"description: {lab.get('description')}")
    print(f"workstream:  {lab.get('workstream', 'unset')}")
    print(f"type:        {lab.get('type', 'host')}")
    print(f"component:   {lab.get('component')}")
    metric = lab.get("metric", {})
    print(f"metric:      {metric.get('name')} ({metric.get('unit')}) target={metric.get('target')}")
    print(f"budget:      {lab.get('budget', 'unset')}")
    allowed = get_allowed_files(lab)
    print(f"allowed_files: {len(allowed)} file(s)")
    print("OK")
    return 0


def _cmd_autotest(lab_id):
    try:
        lab = load_lab(lab_id)
    except FileNotFoundError as e:
        print(str(e), file=sys.stderr)
        return 1

    print(generate_autotest_script(lab), end="")
    return 0


def _cmd_check(lab_id, value_str):
    try:
        lab = load_lab(lab_id)
    except FileNotFoundError as e:
        print(str(e), file=sys.stderr)
        return 1

    target_expr = lab.get("metric", {}).get("target")
    if not target_expr:
        print("ERROR: no metric.target in lab config", file=sys.stderr)
        return 1

    try:
        value = float(value_str)
        result = check_target(value, target_expr)
    except (ValueError, TypeError) as e:
        print(f"ERROR: {e}", file=sys.stderr)
        return 1

    print("PASS" if result else "FAIL")
    return 0 if result else 1


def _cmd_allowed_files(lab_id):
    try:
        lab = load_lab(lab_id)
    except FileNotFoundError as e:
        print(str(e), file=sys.stderr)
        return 1

    for f in get_allowed_files(lab):
        print(f)
    return 0


def main():
    if len(sys.argv) < 3:
        print(
            "Usage: parse_lab.py <lab_id> validate|autotest|check <value>|allowed_files",
            file=sys.stderr,
        )
        sys.exit(1)

    lab_id = sys.argv[1]
    action = sys.argv[2]

    if action == "validate":
        sys.exit(_cmd_validate(lab_id))
    elif action == "autotest":
        sys.exit(_cmd_autotest(lab_id))
    elif action == "check":
        if len(sys.argv) < 4:
            print("Usage: parse_lab.py <lab_id> check <value>", file=sys.stderr)
            sys.exit(1)
        sys.exit(_cmd_check(lab_id, sys.argv[3]))
    elif action == "allowed_files":
        sys.exit(_cmd_allowed_files(lab_id))
    else:
        print(f"Unknown action: {action}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
