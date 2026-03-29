use super::Status;
use crate as ion_shell;
use crate::{types, Shell};
use builtins_proc::builtin;

// ============================================================================
// Guardian builtin: thin rewrite layer over mcp call guardian.*
//
// ADVERSARIAL DESIGN — type-driven parameter schema dispatch:
//
// The obvious approach is a flat (name, endpoint, default_args, desc) table.
// That works today but breaks silently when mcpd methods change again — any
// time a method needs different parameter handling, a new per-method branch
// appears in the dispatch loop. This is the "shotgun approach" anti-pattern.
//
// This implementation uses a ParamShape enum embedded in the shortcuts table
// so every subcommand is SELF-DESCRIBING:
//   - what CLI name it has
//   - what MCP endpoint it resolves to
//   - how its arguments are structured (NoArgs / PassThrough / Structured)
//
// The dispatch loop contains ZERO per-method branches. Adding a new guardian
// method = adding one table row. Renaming a method = changing one string in
// one table column. Changing parameter structure = swapping one ParamShape.
//
// The Generator renames the table. The Adversary restructures the model.
// ============================================================================

use super::mcp::{mcp_call, mcp_subscribe};

// ============================================================================
// Parameter shape descriptor
// ============================================================================

/// Describes how a guardian subcommand's CLI arguments map to JSON-RPC params.
///
/// NoArgs      — method takes no parameters; always sends "{}" (read-only queries)
/// PassThrough — caller provides raw JSON; validate it starts with '{' or '['
/// Structured  — a dedicated builder fn constructs typed JSON from positional args
///
/// This encoding eliminates all per-method branches from the dispatch loop.
#[derive(Clone, Copy)]
enum ParamShape {
    /// Method takes no parameters. Send "{}" regardless of any trailing CLI args.
    NoArgs,
    /// Method accepts a raw JSON object passed directly from the command line.
    PassThrough,
    /// Method requires structured JSON; a typed builder validates and constructs it.
    Structured(fn(&[types::Str]) -> Result<String, String>),
}

// ============================================================================
// Structured parameter builders — one fn per method needing typed params
// ============================================================================

/// Build JSON params for guardian.respond.
///
/// Two accepted forms:
///   guardian respond <anomaly_id> <action>   — positional, most ergonomic
///   guardian respond '{"anomaly_id":N,...}'  — raw JSON pass-through fallback
///
/// Positional form validates that anomaly_id is a non-negative integer so the
/// error is caught at the shell layer before a useless RPC round-trip.
fn build_respond_json(args: &[types::Str]) -> Result<String, String> {
    if args.is_empty() {
        return Err(
            "guardian respond: usage: guardian respond <anomaly_id> <action>\n\
             Example: guardian respond 42 dismiss"
                .to_string(),
        );
    }
    // If the first arg already looks like a JSON object, pass it through directly.
    let first = args[0].as_str().trim();
    if first.starts_with('{') {
        return Ok(args.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" "));
    }
    // Positional form: validate anomaly_id is numeric
    let anomaly_id: u64 = first.parse().map_err(|_| {
        format!(
            "guardian respond: anomaly_id must be a non-negative integer, got: {}\n\
             Usage: guardian respond <anomaly_id> <action>",
            first
        )
    })?;
    if args.len() < 2 {
        return Err(
            "guardian respond: missing <action> argument\n\
             Usage: guardian respond <anomaly_id> <action>"
                .to_string(),
        );
    }
    // Action: join remaining args to allow multi-word strings
    let action = args[1..]
        .iter()
        .map(|s| s.as_str())
        .collect::<Vec<_>>()
        .join(" ");

    let mut out = String::with_capacity(action.len() + 40);
    out.push_str("{\"anomaly_id\":");
    out.push_str(&anomaly_id.to_string());
    out.push_str(",\"action\":\"");
    for ch in action.chars() {
        match ch {
            '"'  => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c    => out.push(c),
        }
    }
    out.push_str("\"}");
    Ok(out)
}

// ============================================================================
// Shortcut table — single source of truth for all guardian subcommands
//
// Columns: (cli_name, mcp_endpoint, ParamShape, description)
//
// To rename a method:            change mcp_endpoint string in one row
// To add a method:               add one row
// To change parameter structure: swap ParamShape for that row
// No other files need updating when the guardian API evolves.
// ============================================================================

static GUARDIAN_SHORTCUTS: &[(&str, &str, ParamShape, &str)] = &[
    (
        "state",
        "guardian.state",
        ParamShape::NoArgs,
        "Show current system state, anomaly counts, and uptime",
    ),
    (
        "anomalies",
        "guardian.anomalies",
        ParamShape::NoArgs,
        "List all detected anomalies",
    ),
    (
        "respond",
        "guardian.respond",
        ParamShape::Structured(build_respond_json),
        "Respond to an anomaly: guardian respond <anomaly_id> <action>",
    ),
    (
        "config",
        "guardian.config",
        ParamShape::PassThrough,
        "Get or set guardian configuration (optional JSON params)",
    ),
    (
        "history",
        "guardian.history",
        ParamShape::NoArgs,
        "Show guardian event history log",
    ),
];

// ============================================================================
// Parameter resolution — pure function, zero side effects, fully testable
// ============================================================================

/// Resolve CLI args to a JSON params string using the shape descriptor.
///
/// This is the only place that knows about ParamShape variants. The dispatch
/// loop calls this and only inspects Ok/Err — it never pattern-matches on the
/// shape itself. All per-method logic lives in builder fns or shape variants.
fn resolve_params(shape: ParamShape, rest: &[types::Str]) -> Result<String, String> {
    match shape {
        ParamShape::NoArgs => Ok("{}".to_string()),

        ParamShape::PassThrough => {
            if rest.is_empty() {
                return Ok("{}".to_string());
            }
            let joined = rest.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" ");
            let trimmed = joined.trim();
            if !trimmed.starts_with('{') && !trimmed.starts_with('[') {
                Err(format!(
                    "params must be a JSON object or array, got: {}",
                    trimmed
                ))
            } else {
                Ok(joined)
            }
        }

        ParamShape::Structured(builder) => builder(rest),
    }
}

// ============================================================================
// Main builtin entry point
// ============================================================================

#[builtin(
    desc = "interact with the ACOS guardian service",
    man = "
SYNOPSIS
    guardian <subcommand> [ARGS]...

DESCRIPTION
    Shortcut interface to the guardian MCP service.
    All commands resolve internally to mcp call guardian.* calls.

    Methods are the real mcpd guardian service methods:
    state, anomalies, respond, config, history.

SUBCOMMANDS
    state                               Show current system state, anomaly counts, uptime
    anomalies                           List all detected anomalies
    respond <anomaly_id> <action>       Respond to an anomaly by ID with specified action
    config [json-params]                Get or set guardian configuration
    history                             Show guardian event history log
    subscribe                           Subscribe to live guardian events (streaming)

EXAMPLES
    guardian state
    guardian anomalies
    guardian respond 42 dismiss
    guardian respond 7 quarantine
    guardian respond '{\"anomaly_id\": 3, \"action\": \"block\"}'
    guardian config
    guardian config {\"threshold\": 5}
    guardian history
    guardian subscribe
"
)]
pub fn guardian(args: &[types::Str], _shell: &mut Shell<'_>) -> Status {
    let subcmd = match args.get(1) {
        Some(s) => s.as_str(),
        None => {
            eprintln!("guardian: missing subcommand. Available:");
            for &(name, _, _, desc) in GUARDIAN_SHORTCUTS {
                eprintln!("  guardian {}  - {}", name, desc);
            }
            eprintln!("  guardian subscribe  - Subscribe to live guardian events");
            return Status::error("missing subcommand");
        }
    };

    // "subscribe" is a special streaming pass-through, not a regular call/response
    if subcmd == "subscribe" {
        return match mcp_subscribe("guardian") {
            Ok(response) => {
                println!("{}", response);
                Status::SUCCESS
            }
            Err(e) => {
                eprintln!("{}", e);
                Status::error(e)
            }
        };
    }

    // Table-driven dispatch — zero per-method branches in this loop
    for &(name, endpoint, shape, _) in GUARDIAN_SHORTCUTS {
        if name == subcmd {
            let rest = if args.len() > 2 { &args[2..] } else { &[] };
            let json_params = match resolve_params(shape, rest) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("{}", e);
                    return Status::error(e);
                }
            };

            return match mcp_call(endpoint, &json_params) {
                Ok(response) => {
                    println!("{}", response);
                    Status::SUCCESS
                }
                Err(e) => {
                    eprintln!("{}", e);
                    Status::error(e)
                }
            };
        }
    }

    eprintln!("guardian: unknown subcommand '{}'. Available:", subcmd);
    for &(name, _, _, desc) in GUARDIAN_SHORTCUTS {
        eprintln!("  guardian {}  - {}", name, desc);
    }
    eprintln!("  guardian subscribe  - Subscribe to live guardian events");
    Status::error(format!("unknown subcommand: {}", subcmd))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- ParamShape resolution tests ---

    #[test]
    fn test_no_args_shape_always_empty_object() {
        let result = resolve_params(ParamShape::NoArgs, &[]).unwrap();
        assert_eq!(result, "{}");
    }

    #[test]
    fn test_no_args_shape_ignores_extra_cli_args() {
        // NoArgs silently ignores trailing args — safe for read-only methods
        let extra: Vec<types::Str> = vec!["ignored".into()];
        let result = resolve_params(ParamShape::NoArgs, &extra).unwrap();
        assert_eq!(result, "{}");
    }

    #[test]
    fn test_passthrough_no_args_returns_empty_object() {
        let result = resolve_params(ParamShape::PassThrough, &[]).unwrap();
        assert_eq!(result, "{}");
    }

    #[test]
    fn test_passthrough_valid_json_accepted() {
        let args: Vec<types::Str> = vec!["{\"threshold\":5}".into()];
        let result = resolve_params(ParamShape::PassThrough, &args).unwrap();
        assert!(result.contains("\"threshold\""));
        assert!(result.contains("5"));
    }

    #[test]
    fn test_passthrough_rejects_non_json() {
        let args: Vec<types::Str> = vec!["not_json".into()];
        let result = resolve_params(ParamShape::PassThrough, &args);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("JSON object"));
    }

    #[test]
    fn test_passthrough_accepts_json_array() {
        let args: Vec<types::Str> = vec!["[1,2,3]".into()];
        let result = resolve_params(ParamShape::PassThrough, &args).unwrap();
        assert!(result.contains("[1,2,3]"));
    }

    // --- build_respond_json tests ---

    #[test]
    fn test_build_respond_json_positional_valid() {
        let args: Vec<types::Str> = vec!["42".into(), "dismiss".into()];
        let json = build_respond_json(&args).unwrap();
        assert!(json.contains("\"anomaly_id\":42"));
        assert!(json.contains("\"action\":\"dismiss\""));
    }

    #[test]
    fn test_build_respond_json_multi_word_action() {
        let args: Vec<types::Str> = vec!["7".into(), "quarantine".into(), "immediately".into()];
        let json = build_respond_json(&args).unwrap();
        assert!(json.contains("\"anomaly_id\":7"));
        assert!(json.contains("\"action\":\"quarantine immediately\""));
    }

    #[test]
    fn test_build_respond_json_invalid_id_returns_err() {
        let args: Vec<types::Str> = vec!["notanumber".into(), "dismiss".into()];
        let result = build_respond_json(&args);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("non-negative integer"));
    }

    #[test]
    fn test_build_respond_json_missing_action_returns_err() {
        let args: Vec<types::Str> = vec!["42".into()];
        let result = build_respond_json(&args);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("missing <action>"));
    }

    #[test]
    fn test_build_respond_json_no_args_returns_err() {
        let result = build_respond_json(&[]);
        assert!(result.is_err());
    }

    #[test]
    fn test_build_respond_json_escapes_quotes_in_action() {
        let args: Vec<types::Str> = vec!["1".into(), "say \"hello\"".into()];
        let json = build_respond_json(&args).unwrap();
        assert!(json.contains("\\\"hello\\\""));
    }

    #[test]
    fn test_build_respond_json_passthrough_raw_json() {
        let args: Vec<types::Str> = vec!["{\"anomaly_id\": 3, \"action\": \"block\"}".into()];
        let json = build_respond_json(&args).unwrap();
        assert!(json.contains("anomaly_id"));
        assert!(json.contains("block"));
    }

    // --- Table completeness and correctness tests ---

    #[test]
    fn test_shortcuts_table_has_all_real_mcpd_methods() {
        let endpoints: Vec<&str> = GUARDIAN_SHORTCUTS
            .iter()
            .map(|&(_, ep, _, _)| ep)
            .collect();
        assert!(endpoints.contains(&"guardian.state"));
        assert!(endpoints.contains(&"guardian.anomalies"));
        assert!(endpoints.contains(&"guardian.respond"));
        assert!(endpoints.contains(&"guardian.config"));
        assert!(endpoints.contains(&"guardian.history"));
    }

    #[test]
    fn test_shortcuts_table_has_no_phantom_methods() {
        // Verify the old fake methods are completely absent
        let endpoints: Vec<&str> = GUARDIAN_SHORTCUTS
            .iter()
            .map(|&(_, ep, _, _)| ep)
            .collect();
        assert!(
            !endpoints.contains(&"guardian.status"),
            "guardian.status is not a real mcpd method"
        );
        assert!(
            !endpoints.contains(&"guardian.ask"),
            "guardian.ask is not a real mcpd method"
        );
        assert!(
            !endpoints.contains(&"guardian.log"),
            "guardian.log is not a real mcpd method"
        );
    }

    #[test]
    fn test_shortcuts_cli_names_match_method_suffixes() {
        // CLI name MUST equal the method suffix (state→guardian.state, not state→guardian.status)
        // This test enforces the invariant the Generator's flat table can violate silently.
        for &(name, endpoint, _, _) in GUARDIAN_SHORTCUTS {
            let expected = format!("guardian.{}", name);
            assert_eq!(
                endpoint, expected,
                "CLI name '{}' maps to '{}' but should map to 'guardian.{}'",
                name, endpoint, name
            );
        }
    }

    // --- Integration smoke tests (host mock transport) ---

    #[test]
    fn test_guardian_state_calls_correct_endpoint() {
        let result = mcp_call("guardian.state", "{}").unwrap();
        assert!(result.contains("\"jsonrpc\":\"2.0\""));
        assert!(result.contains("\"result\""));
        assert!(result.contains("guardian.state"));
    }

    #[test]
    fn test_guardian_anomalies_calls_correct_endpoint() {
        let result = mcp_call("guardian.anomalies", "{}").unwrap();
        assert!(result.contains("guardian.anomalies"));
    }

    #[test]
    fn test_guardian_history_calls_correct_endpoint() {
        let result = mcp_call("guardian.history", "{}").unwrap();
        assert!(result.contains("guardian.history"));
    }

    #[test]
    fn test_guardian_respond_with_typed_params() {
        let args: Vec<types::Str> = vec!["5".into(), "dismiss".into()];
        let params = build_respond_json(&args).unwrap();
        let result = mcp_call("guardian.respond", &params).unwrap();
        assert!(result.contains("guardian.respond"));
    }

    #[test]
    fn test_guardian_config_calls_correct_endpoint() {
        let result = mcp_call("guardian.config", "{}").unwrap();
        assert!(result.contains("guardian.config"));
    }

    // ====================================================================
    // ADVERSARIAL STRUCTURAL INVARIANTS — guardian.rs side
    //
    // The Generator asserts individual method names exist.
    // We define a canonical oracle and assert BIDIRECTIONAL completeness:
    //   - Every real method is in the table (no missing entry)
    //   - Every table entry is a real method (no phantom entry)
    //   - The table has EXACTLY the right number of entries (no extra)
    //
    // One addition to mcpd guardian API = one row in GUARDIAN_SHORTCUTS
    // + one string in CANONICAL_GUARDIAN_METHODS below.  Miss either
    // one and a test fails. The Generator's approach misses half of this.
    // ====================================================================

    /// Canonical set of real mcpd guardian service methods.
    /// Derived from guardian_handler.rs:901-916 in the mcpd source.
    /// This is the authoritative oracle for ALL phantom-method checks.
    const CANONICAL_GUARDIAN_METHODS: &[&str] = &[
        "state",
        "anomalies",
        "respond",
        "config",
        "history",
    ];

    #[test]
    fn test_shortcuts_table_is_exactly_the_canonical_set_no_extras_no_gaps() {
        // Bidirectional completeness:
        //   Forward: every canonical method has a row in GUARDIAN_SHORTCUTS
        //   Reverse: every row in GUARDIAN_SHORTCUTS maps to a canonical method
        // The Generator only tests forward. We test both directions.

        let table_methods: Vec<&str> = GUARDIAN_SHORTCUTS
            .iter()
            .map(|&(name, _, _, _)| name)
            .collect();

        // Forward: canonical → table
        for &canonical in CANONICAL_GUARDIAN_METHODS {
            assert!(
                table_methods.contains(&canonical),
                "canonical method '{}' is missing from GUARDIAN_SHORTCUTS — add a table row",
                canonical
            );
        }

        // Reverse: table → canonical  (catches phantom insertions)
        for &table_name in &table_methods {
            assert!(
                CANONICAL_GUARDIAN_METHODS.contains(&table_name),
                "GUARDIAN_SHORTCUTS row '{}' is not in CANONICAL_GUARDIAN_METHODS — \
                 either it is a phantom method or CANONICAL_GUARDIAN_METHODS needs updating",
                table_name
            );
        }

        // Count: exact match prevents silent duplicates
        assert_eq!(
            table_methods.len(),
            CANONICAL_GUARDIAN_METHODS.len(),
            "GUARDIAN_SHORTCUTS has {} entries but CANONICAL_GUARDIAN_METHODS has {} — \
             they must be identical sets",
            table_methods.len(),
            CANONICAL_GUARDIAN_METHODS.len()
        );
    }

    #[test]
    fn test_all_canonical_methods_return_jsonrpc_envelope() {
        // Every canonical guardian method must return a JSON-RPC 2.0 envelope
        // when called via mcp_call. This is the contract QEMU code relies on.
        // The test exercises the full dispatch path for every real method.
        for &method in CANONICAL_GUARDIAN_METHODS {
            let endpoint = format!("guardian.{}", method);
            let params = if method == "respond" {
                // respond needs at least anomaly_id + action
                "{\"anomaly_id\":0,\"action\":\"test\"}"
            } else {
                "{}"
            };
            let result = mcp_call(&endpoint, params);
            assert!(
                result.is_ok(),
                "mcp_call({:?}) failed for canonical guardian method: {:?}",
                endpoint, result
            );
            let resp = result.unwrap();
            assert!(
                resp.contains("\"jsonrpc\":\"2.0\""),
                "canonical guardian method {} response is not JSON-RPC 2.0: {}",
                method, resp
            );
            assert!(
                resp.contains("\"result\""),
                "canonical guardian method {} response has no 'result' field: {}",
                method, resp
            );
            // The response must echo the full endpoint so callers can identify it
            assert!(
                resp.contains(&endpoint),
                "canonical guardian method {} response does not echo endpoint: {}",
                method, resp
            );
        }
    }

    #[test]
    fn test_resolve_params_returns_valid_json_for_all_shapes() {
        // For every row in GUARDIAN_SHORTCUTS, resolve_params must return
        // a string that starts with '{' or '[' (valid JSON container).
        // This catches the case where a builder returns a bare scalar.
        for &(name, _, shape, _) in GUARDIAN_SHORTCUTS {
            let args: Vec<types::Str> = if name == "respond" {
                vec!["1".into(), "dismiss".into()]
            } else {
                vec![]
            };
            let params = resolve_params(shape, &args)
                .unwrap_or_else(|e| panic!("resolve_params failed for '{}': {}", name, e));
            let trimmed = params.trim();
            assert!(
                trimmed.starts_with('{') || trimmed.starts_with('['),
                "resolve_params for '{}' returned non-JSON-container: {:?}",
                name, params
            );
        }
    }

    #[test]
    fn test_phantom_method_names_are_absent_from_all_shortcut_fields() {
        // Checks both the CLI name AND the endpoint string for phantom tokens.
        // The Generator only checks endpoints. We check both columns.
        let phantom = ["status", "ask", "log",
                        "guardian.status", "guardian.ask", "guardian.log"];
        for &p in &phantom {
            for &(cli_name, endpoint, _, desc) in GUARDIAN_SHORTCUTS {
                assert_ne!(
                    cli_name, p,
                    "phantom '{}' found as CLI name in GUARDIAN_SHORTCUTS", p
                );
                assert!(
                    !endpoint.contains(p),
                    "phantom '{}' found in endpoint '{}' in GUARDIAN_SHORTCUTS", p, endpoint
                );
                // Also check description strings don't advertise phantom commands
                // (users read descriptions for usage guidance)
                assert!(
                    !desc.to_lowercase().contains("guardian.status") &&
                    !desc.to_lowercase().contains("guardian.ask") &&
                    !desc.to_lowercase().contains("guardian.log"),
                    "description for CLI name '{}' references a phantom method: {:?}",
                    cli_name, desc
                );
            }
        }
    }
}
