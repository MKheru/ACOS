# ACOS Horizon Desktop Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build **ACOS Horizon** — a state-of-the-art, high-performance, and immersive windowed desktop environment (similar to macOS/Windows 11) for ACOS, completely replacing the static terminal view with a multi-app window manager, file explorer, system monitor, and active AI Guardian dashboard.

**Architecture:** A lightweight React-based floating window manager built directly in `acos_ui`. It establishes a persistent WebSocket connection to the ACOS `mcpd` system services to feed live filesystem structures, system processes, and CPU/RAM metrics into floating visual windows, letting users manage Redox graphically.

**Tech Stack:** React 19, Vite, Framer Motion (for physics-based smooth window dragging, scaling, and desktop animations), Lucide React (premium vector icons), and HSL-based dynamic styling systems (Matrix, Cyberpunk, Dracula, Brutalism).

---

## 1. High-Level Visual Specifications & Workspace Layout

```
┌────────────────────────────────────────────────────────┐
│ [ACOS Horizon Desktop]                       12:20 PM  │
│                                                        │
│  [📁 Files]                                           │
│  [⚙️ Monitor]                                          │
│  [🛡️ Guardian]                                         │
│                                                        │
│         ┌────────────────────────────────────────┐     │
│         │ 📁 File Explorer (Redox /etc)  [-][⬜][x]│     │
│         ├────────────────────────────────────────┤     │
│         │ > acos/        > host.conf             │     │
│         │ > theme.toml   > hostname              │     │
│         └────────────────────────────────────────┘     │
│                                                        │
├────────────────────────────────────────────────────────┤
│ 🟣 [Orb]  📁 Files  ⚙️ Monitor  🛡️ Security   Theme: Cyber  │
└────────────────────────────────────────────────────────┘
```

---

## 2. Implementation Steps

### Task 1: Environment Upgrades & Window Manager Core

**Files:**
* Modify: `/home/ankheru/Documents/Projects/ACOS/acos_ui/src/App.jsx`
* Modify: `/home/ankheru/Documents/Projects/ACOS/acos_ui/src/index.css`

**Step 1: Define window manager state**
Define a state list of windows in `App.jsx`, supporting properties like `id`, `title`, `isOpen`, `isMinimized`, `zIndex`, `x`, `y`, and component rendering.
```javascript
const [windows, setWindows] = useState([
  { id: 'terminal', title: 'ACOS Terminal', icon: 'Terminal', isOpen: true, isMinimized: false, zIndex: 10, x: 50, y: 50, w: 600, h: 400 },
  { id: 'files', title: 'File Explorer', icon: 'FolderOpen', isOpen: false, isMinimized: false, zIndex: 1, x: 100, y: 100, w: 550, h: 380 },
  { id: 'monitor', title: 'System Monitor', icon: 'Cpu', isOpen: false, isMinimized: false, zIndex: 1, x: 150, y: 150, w: 500, h: 350 },
  { id: 'guardian', title: 'AI Guardian', icon: 'Shield', isOpen: true, isMinimized: false, zIndex: 5, x: 700, y: 50, w: 450, h: 500 }
]);
```

**Step 2: Implement focus management**
Create a function `focusWindow(id)` that increments the `zIndex` of the targeted window to bring it to the foreground.

**Step 3: Create `<AcosWindow />` floating component**
Use Framer Motion's `drag` capability to implement smooth floating window movement with title bar boundaries:
```jsx
<motion.div
  drag
  dragHandleClassName="window-titlebar"
  dragMomentum={false}
  style={{ x: win.x, y: win.y, zIndex: win.zIndex, width: win.w, height: win.h }}
  className="floating-window glass glow-border"
>
  <div className="window-titlebar">
    <span>{win.title}</span>
    <div className="window-controls">
      <button onClick={() => minimizeWindow(win.id)}>_</button>
      <button onClick={() => closeWindow(win.id)}>X</button>
    </div>
  </div>
  <div className="window-body">{renderWindowContent(win.id)}</div>
</motion.div>
```

**Step 4: Commit**
```bash
git add acos_ui/src/App.jsx acos_ui/src/index.css
git commit -m "feat: implement premium floating window manager core with Framer Motion drag"
```

---

### Task 2: Taskbar & Dynamic Start Menu (ACOS Orb)

**Files:**
* Modify: `/home/ankheru/Documents/Projects/ACOS/acos_ui/src/App.jsx`
* Modify: `/home/ankheru/Documents/Projects/ACOS/acos_ui/src/index.css`

**Step 1: Implement the Taskbar Dock**
Design a dock at the bottom of the screen featuring:
* The **ACOS Orb / Start Button** (highly visual glowing orb).
* Active apps icons with active glowing states if open, showing dynamic badges.
* System tray displaying live clock, memory/CPU micro-indicators, and WebSocket latency.

**Step 2: Build the Start Menu**
On clicking the Orb, open an interactive panel overlay with search filtering. This lists all ACOS built-in apps and dynamic shortcut commands.

**Step 3: Commit**
```bash
git add acos_ui/src/App.jsx acos_ui/src/index.css
git commit -m "feat: add ACOS Orb start menu and taskbar dock with system metrics"
```

---

### Task 3: Interactive File Explorer App (ACOS Files)

**Files:**
* Modify: `/home/ankheru/Documents/Projects/ACOS/acos_ui/src/App.jsx`

**Step 1: Write filesystem navigation logic**
Connect to ACOS filesystem over MCP via WebSocket to fetch current directory structures.
* Send `file/list` command to `mcp:system` or `mcp:files` to read folder structures.
* Support double-clicking on folders to step into them.
* Support single-clicking on files to display their details and properties.

**Step 2: Add visual explorer grid**
Create a grid of visual file folders and documents. Display file names, file sizes, and add a path breadcrumb navigation bar at the top (e.g., `root > etc > acos`).

**Step 3: Commit**
```bash
git add acos_ui/src/App.jsx
git commit -m "feat: implement visual file explorer connecting to Redox filesystem"
```

---

### Task 4: Interactive System & Process Monitor (ACOS Monitor)

**Files:**
* Modify: `/home/ankheru/Documents/Projects/ACOS/acos_ui/src/App.jsx`

**Step 1: Fetch live process and memory metrics**
Periodically query `process/list` and `system/info` via System WebSocket.
* Plot live RAM usage history in a SVG line graph.
* Render a searchable process list table with columns: `PID`, `Name`, `Memory`, `Status`.

**Step 2: Implement "Kill Process" capability**
Add a red "End Task" button. Clicking this triggers the `run` command of `mcp:command` with `kill -9 <PID>` to terminate the Redox process instantly.

**Step 3: Commit**
```bash
git add acos_ui/src/App.jsx
git commit -m "feat: add system and process monitor with graphical task ending"
```

---

## 3. Verification & Quality Assurance (Headless + Manual)

1. **Local Compilation Check:** Ensure Vite builds successfully with zero linting warnings.
2. **Headless TTY Check:** Run `node ui_headless_test.js` to ensure the new window manager renders fully with zero JavaScript exceptions on launch.
3. **VM Live Check:** Start QEMU, login, and verify that the File Explorer and Process Monitor dynamically render actual Redox data via WebSockets.
