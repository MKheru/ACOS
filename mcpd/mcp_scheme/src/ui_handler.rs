//! MCP UI Service Handler for ACOS (Agent-Centric OS)
//!
//! Exposes visual customization, theme management, and Servo browser DOM/render stubs.

use crate::handler::ServiceHandler;
use crate::protocol::{JsonRpcRequest, JsonRpcResponse, INVALID_PARAMS, METHOD_NOT_FOUND};
use crate::McpPath;
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

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct DomNode {
    pub id: String,
    pub tag: String, // "window", "button", "input", "text", "link", "container", etc.
    pub label: Option<String>,
    pub value: Option<String>,
    pub children: Vec<DomNode>,
}

pub struct UiHandler {
    current_theme: Mutex<String>,
    custom_themes: Mutex<FxHashMap<String, ThemeColors>>,
    predefined_themes: FxHashMap<String, ThemeColors>,
    virtual_dom: Mutex<DomNode>,
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

        // Initialize state by trying to read from /etc/acos/theme.toml
        let current_theme = if let Ok(content) = std::fs::read_to_string("/etc/acos/theme.toml") {
            // Basic parsing of TOML key-value for theme name
            content.lines()
                .find(|l| l.starts_with("name = "))
                .and_then(|l| l.split('"').nth(1))
                .map(|s| s.to_string())
                .unwrap_or_else(|| "dark".to_string())
        } else {
            "dark".to_string()
        };

        // Initialize a default interactive virtual DOM tree (token-efficient for agent interaction)
        let default_dom = DomNode {
            id: "root-window".to_string(),
            tag: "window".to_string(),
            label: Some("ACOS Semantic Console Shell".to_string()),
            value: None,
            children: vec![
                DomNode {
                    id: "nav-bar".to_string(),
                    tag: "container".to_string(),
                    label: Some("Navigation Controls".to_string()),
                    value: None,
                    children: vec![
                        DomNode {
                            id: "btn-back".to_string(),
                            tag: "button".to_string(),
                            label: Some("Back".to_string()),
                            value: None,
                            children: vec![],
                        },
                        DomNode {
                            id: "address-input".to_string(),
                            tag: "input".to_string(),
                            label: Some("Search or URL".to_string()),
                            value: Some("mcp://welcome".to_string()),
                            children: vec![],
                        },
                    ],
                },
                DomNode {
                    id: "main-content".to_string(),
                    tag: "container".to_string(),
                    label: Some("Primary Interactive Viewport".to_string()),
                    value: None,
                    children: vec![
                        DomNode {
                            id: "title-display".to_string(),
                            tag: "text".to_string(),
                            label: Some("ACOS Rich Interface WS10".to_string()),
                            value: None,
                            children: vec![],
                        },
                        DomNode {
                            id: "submit-action".to_string(),
                            tag: "button".to_string(),
                            label: Some("Execute Agent Routine".to_string()),
                            value: None,
                            children: vec![],
                        },
                    ],
                },
            ],
        };

        Self {
            current_theme: Mutex::new(current_theme),
            custom_themes: Mutex::new(FxHashMap::default()),
            predefined_themes: predefined,
            virtual_dom: Mutex::new(default_dom),
        }
    }

    fn save_theme_to_disk(&self, name: &str, colors: &ThemeColors) {
        let toml_content = format!(
            "[theme]\nname = \"{}\"\nbg = \"{}\"\nfg = \"{}\"\naccent = \"{}\"\nprimary = \"{}\"\nsecondary = \"{}\"\n",
            name, colors.bg, colors.fg, colors.accent, colors.primary, colors.secondary
        );
        let _ = std::fs::create_dir_all("/etc/acos");
        if let Err(e) = std::fs::write("/etc/acos/theme.toml", toml_content) {
            eprintln!("[WARN] Failed to write theme configuration: {}", e);
        }
    }

    fn render_dom_to_markdown(&self, node: &DomNode, depth: usize) -> String {
        let indent = "  ".repeat(depth);
        let mut out = format!("{}{}: [{}]", indent, node.tag.to_uppercase(), node.id);
        if let Some(lbl) = &node.label {
            out.push_str(&format!(" label=\"{}\"", lbl));
        }
        if let Some(val) = &node.value {
            out.push_str(&format!(" value=\"{}\"", val));
        }
        out.push_str("\n");
        for child in &node.children {
            out.push_str(&self.render_dom_to_markdown(child, depth + 1));
        }
        out
    }
}

impl ServiceHandler for UiHandler {
    fn handle(&self, path: &McpPath, request: &JsonRpcRequest) -> JsonRpcResponse {
        // Support both MCP path routing (e.g. mcp:ui/theme/list) and JSON-RPC method routing (e.g. mcp-query shorthand)
        let resource_type = if !path.resource.is_empty() {
            path.resource[0].as_str()
        } else {
            request.method.as_str()
        };

        match resource_type {
            "theme" => {
                let sub_action = if path.resource.len() >= 2 {
                    path.resource[1].as_str()
                } else {
                    request.params.get("action").and_then(|a| a.as_str()).unwrap_or("")
                };
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
                        let name_val = request.params.get("name");
                        if let Some(name) = name_val.and_then(|n| n.as_str()) {
                            if let Some(colors) = self.predefined_themes.get(name) {
                                *self.current_theme.lock().unwrap() = name.to_string();
                                self.save_theme_to_disk(name, colors);
                                return JsonRpcResponse::success(request.id.clone(), json!({ "status": "ok", "active_theme": name }));
                            } else if let Ok(customs) = self.custom_themes.lock() {
                                if let Some(colors) = customs.get(name) {
                                    *self.current_theme.lock().unwrap() = name.to_string();
                                    self.save_theme_to_disk(name, colors);
                                    return JsonRpcResponse::success(request.id.clone(), json!({ "status": "ok", "active_theme": name }));
                                }
                            }
                            return JsonRpcResponse::error(request.id.clone(), INVALID_PARAMS, format!("Theme '{}' not found", name));
                        }
                        JsonRpcResponse::error(request.id.clone(), INVALID_PARAMS, "Missing 'name' parameter")
                    }
                    "custom" => {
                        let name_val = request.params.get("name");
                        let colors_val = request.params.get("colors");
                        if let (Some(name), Some(colors_val)) = (name_val.and_then(|n| n.as_str()), colors_val) {
                            if let Ok(colors) = serde_json::from_value::<ThemeColors>(colors_val.clone()) {
                                if let Ok(mut customs) = self.custom_themes.lock() {
                                    customs.insert(name.to_string(), colors.clone());
                                    *self.current_theme.lock().unwrap() = name.to_string();
                                    self.save_theme_to_disk(name, &colors);
                                    return JsonRpcResponse::success(request.id.clone(), json!({ "status": "ok", "custom_theme": name }));
                                }
                            }
                        }
                        JsonRpcResponse::error(request.id.clone(), INVALID_PARAMS, "Invalid custom theme parameters")
                    }
                    _ => JsonRpcResponse::error(request.id.clone(), METHOD_NOT_FOUND, "Unknown theme action")
                }
            }
            "dom" => {
                let sub_action = if path.resource.len() >= 2 {
                    path.resource[1].as_str()
                } else {
                    request.params.get("action").and_then(|a| a.as_str()).unwrap_or("")
                };
                match sub_action {
                    "get" => {
                        let dom = self.virtual_dom.lock().unwrap().clone();
                        let format_param = request.params.get("format").and_then(|f| f.as_str()).unwrap_or("json");

                        if format_param == "markdown" {
                            let md = self.render_dom_to_markdown(&dom, 0);
                            JsonRpcResponse::success(request.id.clone(), json!({ "format": "markdown", "dom": md }))
                        } else {
                            JsonRpcResponse::success(request.id.clone(), json!({ "format": "json", "dom": dom }))
                        }
                    }
                    "set" => {
                        let new_dom_val = request.params.get("dom");
                        if let Some(new_dom_val) = new_dom_val {
                            if let Ok(new_dom) = serde_json::from_value::<DomNode>(new_dom_val.clone()) {
                                *self.virtual_dom.lock().unwrap() = new_dom;
                                return JsonRpcResponse::success(request.id.clone(), json!({ "status": "ok" }));
                            }
                        }
                        JsonRpcResponse::error(request.id.clone(), INVALID_PARAMS, "Invalid or missing 'dom' tree parameter")
                    }
                    "parse" => {
                        let html_val = request.params.get("html");
                        if let Some(html) = html_val.and_then(|h| h.as_str()) {
                            // High performance token-efficient heuristic HTML parser for micro-DOM tree creation
                            // Filters structure divs, scripts, styles, keeping only text, links, and interactive nodes.
                            let mut root = DomNode {
                                id: "parsed-root".to_string(),
                                tag: "container".to_string(),
                                label: Some("Parsed Viewport".to_string()),
                                value: None,
                                children: vec![],
                            };

                            // Simplified semantic text element extraction
                            let mut node_idx = 1;
                            if html.contains("<button") || html.contains("<input") || html.contains("<a") {
                                for word in html.split('<') {
                                    if word.starts_with("button") {
                                        if let Some(lbl) = word.split('>').nth(1).and_then(|s| s.split('<').next()) {
                                            root.children.push(DomNode {
                                                id: format!("parsed-btn-{}", node_idx),
                                                tag: "button".to_string(),
                                                label: Some(lbl.trim().to_string()),
                                                value: None,
                                                children: vec![],
                                            });
                                            node_idx += 1;
                                        }
                                    } else if word.starts_with("input") {
                                        let placeholder = word.split("placeholder=\"").nth(1)
                                            .and_then(|s| s.split('"').next())
                                            .map(|s| s.to_string());
                                        let value = word.split("value=\"").nth(1)
                                            .and_then(|s| s.split('"').next())
                                            .map(|s| s.to_string());

                                        root.children.push(DomNode {
                                            id: format!("parsed-input-{}", node_idx),
                                            tag: "input".to_string(),
                                            label: placeholder.or(Some("User Input".to_string())),
                                            value,
                                            children: vec![],
                                        });
                                        node_idx += 1;
                                    } else if word.starts_with("a ") || word.starts_with("a>") {
                                        let href = word.split("href=\"").nth(1)
                                            .and_then(|s| s.split('"').next())
                                            .map(|s| s.to_string());
                                        if let Some(lbl) = word.split('>').nth(1).and_then(|s| s.split('<').next()) {
                                            root.children.push(DomNode {
                                                id: format!("parsed-link-{}", node_idx),
                                                tag: "link".to_string(),
                                                label: Some(lbl.trim().to_string()),
                                                value: href,
                                                children: vec![],
                                            });
                                            node_idx += 1;
                                        }
                                    }
                                }
                            } else {
                                // Default semantic fallback mapping plain text
                                root.children.push(DomNode {
                                    id: "parsed-text-1".to_string(),
                                    tag: "text".to_string(),
                                    label: Some(html.trim().to_string()),
                                    value: None,
                                    children: vec![],
                                });
                            }

                            return JsonRpcResponse::success(request.id.clone(), json!({
                                "status": "parsed",
                                "node_count": node_idx - 1,
                                "dom": root
                            }));
                        }
                        JsonRpcResponse::error(request.id.clone(), INVALID_PARAMS, "Missing 'html' parameter")
                    }
                    _ => JsonRpcResponse::error(request.id.clone(), METHOD_NOT_FOUND, "Unknown DOM action")
                }
            }
            "render" => {
                // Servo page rendering integration stub
                JsonRpcResponse::success(request.id.clone(), json!({
                    "servo_render_status": "stub",
                    "info": "Performs Servo browser layout rasterization and dynamic display buffer writes.",
                    "details": "Planned in WS10 Phase C"
                }))
            }
            _ => JsonRpcResponse::error(request.id.clone(), METHOD_NOT_FOUND, "Unknown UI resource")
        }
    }

    fn list_methods(&self) -> Vec<&str> {
        vec!["list", "get", "set", "custom", "dom", "render"]
    }
}

