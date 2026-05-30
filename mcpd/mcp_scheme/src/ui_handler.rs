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

        Self {
            current_theme: Mutex::new(current_theme),
            custom_themes: Mutex::new(FxHashMap::default()),
            predefined_themes: predefined,
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
                // Servo DOM integration stub (Formatting aligned with token-efficient accessibility trees)
                JsonRpcResponse::success(request.id.clone(), json!({
                    "servo_dom_status": "stub",
                    "info": "Exposes simplified, semantic JSON/Markdown DOM layout trees containing only interactable components to callers.",
                    "details": "Planned in WS10 Phase B"
                }))
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
