# ACOS WS10: Rich Interface Foundations Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Establish the core `mcp://ui` scheme handler in ACOS to lay the architectural foundations for WS10 (Rich Interface), starting with a fully responsive dynamic Theme Engine and stub DOM/Render layers.

**Architecture:** 
We will introduce a native `UiHandler` registered under the `"ui"` service namespace. This handler will route and process requests directed at `mcp://ui/theme` (listing, setting, and custom-defining visual styles) and provide initial stubs for `mcp://ui/dom` and `mcp://ui/render` to eventually interface with the Servo browser engine.

**Tech Stack:** Rust, Redox OS MCP Bus, JSON-RPC 2.0, FxHashMap.

---

### Task 1: Create the UI Handler (`ui_handler.rs`)

**Files:**
- Create: `mcpd/mcp_scheme/src/ui_handler.rs`

**Step 1: Write the UiHandler structure and theme models**

Create the file `ui_handler.rs` defining the themes and implementing the `ServiceHandler` trait.

```rust
use crate::handler::ServiceHandler;
use crate::protocol::{JsonRpcRequest, JsonRpcResponse, McpPath};
use std::sync::Mutex;
use rustc_hash::FxHashMap;
use serde_json::json;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ThemeColors {
    pub bg: String,
    pub fg: String,
    pub accent: String,
    pub primary: String,
    pub secondary: String,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Theme {
    pub name: String,
    pub colors: ThemeColors,
}

pub struct UiHandler {
    current_theme: Mutex<String>,
    custom_themes: Mutex<FxHashMap<String, ThemeColors>>,
    predefined_themes: FxHashMap<String, ThemeColors>,
}

impl UiHandler {
    pub fn new() -> Self {
        let mut predefined = FxHashMap::default();
        predefined.insert("dark".to_string(), ThemeColors {
            bg: "#0a0a0c".to_string(), fg: "#f8f8f2".to_string(), accent: "#ff79c6".to_string(),
            primary: "#bd93f9".to_string(), secondary: "#8be9fd".to_string(),
        });
        predefined.insert("cyberpunk".to_string(), ThemeColors {
            bg: "#000b19".to_string(), fg: "#00f0ff".to_string(), accent: "#ff0055".to_string(),
            primary: "#fffb00".to_string(), secondary: "#00ff66".to_string(),
        });
        predefined.insert("matrix".to_string(), ThemeColors {
            bg: "#0d0208".to_string(), fg: "#00ff41".to_string(), accent: "#003b00".to_string(),
            primary: "#008f11".to_string(), secondary: "#00ff41".to_string(),
        });
        predefined.insert("brutalism".to_string(), ThemeColors {
            bg: "#ffffff".to_string(), fg: "#000000".to_string(), accent: "#ffff00".to_string(),
            primary: "#ff0000".to_string(), secondary: "#0000ff".to_string(),
        });

        Self {
            current_theme: Mutex::new("dark".to_string()),
            custom_themes: Mutex::new(FxHashMap::default()),
            predefined_themes: predefined,
        }
    }
}

impl ServiceHandler for UiHandler {
    fn handle(&self, path: &McpPath, request: &JsonRpcRequest) -> JsonRpcResponse {
        let resource_type = path.resource.get(0).map(|s| s.as_str()).unwrap_or("");

        match resource_type {
            "theme" => {
                let sub_action = path.resource.get(1).map(|s| s.as_str()).unwrap_or("");
                match sub_action {
                    "list" => {
                        let mut list: Vec<String> = self.predefined_themes.keys().cloned().collect();
                        if let Ok(customs) = self.custom_themes.lock() {
                            list.extend(customs.keys().cloned());
                        }
                        JsonRpcResponse::success(request.id.clone(), json!({ "themes": list }))
                    }
                    "get" => {
                        let active = self.current_theme.lock().unwrap().clone();
                        let colors = if let Some(c) = self.predefined_themes.get(&active) {
                            c.clone()
                        } else if let Ok(customs) = self.custom_themes.lock() {
                            customs.get(&active).cloned().unwrap_or_else(|| {
                                self.predefined_themes.get("dark").unwrap().clone()
                            })
                        } else {
                            self.predefined_themes.get("dark").unwrap().clone()
                        };
                        JsonRpcResponse::success(request.id.clone(), json!({ "theme": active, "colors": colors }))
                    }
                    "set" => {
                        if let Some(params) = &request.params {
                            if let Some(name) = params.get("name").and_then(|n| n.as_str()) {
                                if self.predefined_themes.contains_key(name) {
                                    *self.current_theme.lock().unwrap() = name.to_string();
                                    return JsonRpcResponse::success(request.id.clone(), json!({ "status": "ok", "active_theme": name }));
                                } else if let Ok(customs) = self.custom_themes.lock() {
                                    if customs.contains_key(name) {
                                        *self.current_theme.lock().unwrap() = name.to_string();
                                        return JsonRpcResponse::success(request.id.clone(), json!({ "status": "ok", "active_theme": name }));
                                    }
                                }
                                return JsonRpcResponse::error(request.id.clone(), -32602, format!("Theme '{}' not found", name));
                            }
                        }
                        JsonRpcResponse::error(request.id.clone(), -32602, "Missing 'name' parameter")
                    }
                    "custom" => {
                        if let Some(params) = &request.params {
                            if let Some(name) = params.get("name").and_then(|n| n.as_str()) {
                                if let Some(colors_val) = params.get("colors") {
                                    if let Ok(colors) = serde_json::from_value::<ThemeColors>(colors_val.clone()) {
                                        if let Ok(mut customs) = self.custom_themes.lock() {
                                            customs.insert(name.to_string(), colors);
                                            *self.current_theme.lock().unwrap() = name.to_string();
                                            return JsonRpcResponse::success(request.id.clone(), json!({ "status": "ok", "custom_theme": name }));
                                        }
                                    }
                                }
                            }
                        }
                        JsonRpcResponse::error(request.id.clone(), -32602, "Invalid custom theme parameters")
                    }
                    _ => JsonRpcResponse::error(request.id.clone(), -32601, "Unknown theme action")
                }
            }
            "dom" => {
                // Servo DOM integration stub
                JsonRpcResponse::success(request.id.clone(), json!({ "servo_dom_status": "stub", "info": "Servo browser integration planned in WS10 Phase B" }))
            }
            "render" => {
                // Servo page rendering integration stub
                JsonRpcResponse::success(request.id.clone(), json!({ "servo_render_status": "stub", "info": "Servo layout rasterization planned in WS10 Phase C" }))
            }
            _ => JsonRpcResponse::error(request.id.clone(), -32601, "Unknown UI resource")
        }
    }

    fn list_methods(&self) -> Vec<&str> {
        vec!["list", "get", "set", "custom", "dom", "render"]
    }
}
```

---

### Task 2: Register the UI Handler in MCP Scheme

**Files:**
- Modify: `mcpd/mcp_scheme/src/lib.rs`

**Step 1: Declare ui_handler module and import it**

Add `pub mod ui_handler;` and register it in the router.

```rust
// In lib.rs module list (around lines 25-42):
pub mod ui_handler;
use ui_handler::UiHandler;

// In lib.rs McpScheme::new() registrations (around lines 260-275):
router.register("ui", UiHandler::new());
```

**Step 2: Commit**

```bash
git add mcpd/mcp_scheme/src/ui_handler.rs mcpd/mcp_scheme/src/lib.rs
git commit -m "feat(ws10): add native ui theme handler and register mcp://ui service"
```

---

### Task 3: Interactive Design Blueprint & Alignment (Grill-Me Outcomes)

During the collaborative architectural interview (/grill-me), the following technical decisions were finalized and will guide the implementation of WS10:

1. **Motherboard & Terminal Theme Bridge:**
   - **Strategy:** Convert custom HEX colors dynamically to the closest 16 standard ANSI colors for the text-based virtual terminals (Konsole).
   - **Path:** Save visual settings globally to `/etc/acos/theme.toml` so the display manager and active PTY consoles can watch the file and reload automatically without manual session interruptions.

2. **Semantic DOM Format for AI Agents:**
   - **Format:** The `mcp://ui/dom` service will output a simplified, token-efficient JSON or Markdown tree containing exclusively semantic and interactable elements (buttons, links, inputs, headings) rather than raw HTML source code. This optimizes context usage and accuracy for LLM callers.

3. **Hybrid Voice Engine Execution:**
   - **STT/TTS Engines:** Utilize local quantized Whisper.cpp (STT) and Piper (TTS) models to guarantee robust, offline-first execution inside the QEMU environment.
   - **Network Bridge:** Implement a runtime configuration switch that automatically proxies speech transactions to fast external host-side APIs when networking is active.
