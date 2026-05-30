import React, { useState, useEffect, useRef } from 'react';
import { 
  Terminal, Shield, Cpu, RefreshCw, Layers, Layout, 
  Palette, FolderOpen, AlertTriangle, Send, X, Minus, 
  Square, Search, Folder, File, ArrowLeft, Activity, 
  Clock, HardDrive, Play, Ban
} from 'lucide-react';
import { motion, AnimatePresence } from 'framer-motion';

// ACOS Interactive Shell Commands Database with descriptions and usages
const ACOS_COMMANDS = [
  { cmd: 'ls', desc: 'Lists files and directories in the current folder', usage: 'ls -la /etc' },
  { cmd: 'ps', desc: 'Displays currently running system processes with PID, time, and private memory', usage: 'ps' },
  { cmd: 'cat', desc: 'Reads and displays the content of a text file on the screen', usage: 'cat /etc/hostname' },
  { cmd: 'kill', desc: 'Terminates an active system process by its unique PID number', usage: 'kill -9 <PID>' },
  { cmd: 'mcp-query system info', desc: 'Interrogates ACOS hardware statistics, uptime, and load via MCP JSON-RPC', usage: 'mcp-query system info' },
  { cmd: 'mcp-query process list', desc: 'Lists all Redox processes in raw MCP JSON format', usage: 'mcp-query process list' },
  { cmd: 'mcp-query ui dom get', desc: 'Reads the active visual semantic Virtual DOM tree representation', usage: 'mcp-query ui dom get' },
  { cmd: 'ui theme set', desc: 'Switches the visual Horizon style theme (matrix, cyberpunk, Dracula, brutalism)', usage: "ui theme set 'cyberpunk'" }
];

function App() {
  // Horizon Desktop State
  const [activeTheme, setActiveTheme] = useState('cyberpunk');
  const [isStartOpen, setIsStartOpen] = useState(false);
  const [startSearch, setStartSearch] = useState('');
  const [focusedWindowId, setFocusedWindowId] = useState('terminal');
  
  // Windows configurations
  const [windows, setWindows] = useState([
    { id: 'terminal', title: 'ACOS Terminal', icon: Terminal, isOpen: true, isMinimized: false, isMaximized: false, zIndex: 100, x: 80, y: 60, w: 640, h: 420 },
    { id: 'files', title: 'File Explorer', icon: FolderOpen, isOpen: false, isMinimized: false, isMaximized: false, zIndex: 10, x: 180, y: 120, w: 580, h: 390 },
    { id: 'monitor', title: 'System Monitor', icon: Cpu, isOpen: false, isMinimized: false, isMaximized: false, zIndex: 10, x: 260, y: 160, w: 540, h: 380 },
    { id: 'guardian', title: 'AI Guardian', icon: Shield, isOpen: true, isMinimized: false, isMaximized: false, zIndex: 50, x: 740, y: 60, w: 420, h: 480 }
  ]);

  // Terminal state
  const [terminalInput, setTerminalInput] = useState('');
  const [terminalLines, setTerminalLines] = useState([
    'Welcome to ACOS Horizon — Rich Semantic Desktop Environment v1.0.0',
    'MCP Systems active. WebSocket bridge online at ws://localhost:8000',
    'Double-click desktop icons or use the Orb to launch visual system tools',
    'Type standard shell commands in the terminal (autocomplete popup is active!)',
    ''
  ]);

  // Terminal Autocomplete Suggestion State
  const [suggestions, setSuggestions] = useState([]);
  const [selectedSuggestionIndex, setSelectedSuggestionIndex] = useState(0);

  // File Explorer state
  const [currentPath, setCurrentPath] = useState('/');
  const [filesList, setFilesList] = useState([]);
  const [selectedFile, setSelectedFile] = useState(null);
  const [loadingFiles, setLoadingFiles] = useState(false);

  // System/Process monitor state
  const [processes, setProcesses] = useState([]);
  const [selectedPid, setSelectedPid] = useState(null);
  const [systemInfo, setSystemInfo] = useState({
    kernel: 'ACOS-Redox-0.5.12',
    uptime: 'calculating...',
    memory: '2048 MB',
    memory_total: 2048 * 1024 * 1024,
    memory_free: 1024 * 1024 * 1024
  });

  // Guardian state
  const [guardianStatus, setGuardianStatus] = useState('SECURE');
  const [guardianLogs, setGuardianLogs] = useState([
    { id: 1, type: 'info', text: 'ACOS Security Guardian auto-started.' },
    { id: 2, type: 'info', text: 'Kernel capability policies loaded (deny-by-default).' },
    { id: 3, type: 'info', text: 'Active monitoring on all 16 MCP namespaces.' }
  ]);

  // Clock state
  const [currentTime, setCurrentTime] = useState('');

  const terminalEndRef = useRef(null);
  
  // WebSockets instances
  const wsCmdRef = useRef(null);
  const wsUiRef = useRef(null);
  const wsSysRef = useRef(null);
  const desktopRef = useRef(null);

  // Update clock every second
  useEffect(() => {
    const updateTime = () => {
      const date = new Date();
      setCurrentTime(date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }));
    };
    updateTime();
    const interval = setInterval(updateTime, 1000);
    return () => clearInterval(interval);
  }, []);

  // Auto-scroll terminal
  useEffect(() => {
    terminalEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [terminalLines]);

  // Focus utility
  const focusWindow = (id) => {
    setFocusedWindowId(id);
    setWindows(prev => {
      // Find max z-index
      const maxZ = Math.max(...prev.map(w => w.zIndex), 10);
      return prev.map(w => {
        if (w.id === id) {
          return { ...w, zIndex: maxZ + 1, isMinimized: false };
        }
        return w;
      });
    });
  };

  const openWindow = (id) => {
    setIsStartOpen(false);
    setWindows(prev => prev.map(w => {
      if (w.id === id) {
        return { ...w, isOpen: true, isMinimized: false };
      }
      return w;
    }));
    focusWindow(id);
  };

  const closeWindow = (id, e) => {
    e?.stopPropagation();
    setWindows(prev => prev.map(w => {
      if (w.id === id) {
        return { ...w, isOpen: false };
      }
      return w;
    }));
  };

  const minimizeWindow = (id, e) => {
    e?.stopPropagation();
    setWindows(prev => prev.map(w => {
      if (w.id === id) {
        return { ...w, isMinimized: true };
      }
      return w;
    }));
  };

  const toggleMaximizeWindow = (id, e) => {
    e?.stopPropagation();
    setWindows(prev => prev.map(w => {
      if (w.id === id) {
        return { ...w, isMaximized: !w.isMaximized };
      }
      return w;
    }));
    focusWindow(id);
  };

  // Connect to ACOS mcpd system WebSockets
  useEffect(() => {
    // 1. Command WebSocket
    wsCmdRef.current = new WebSocket('ws://localhost:8000/command');
    wsCmdRef.current.onopen = () => {
      console.log('Connected to ACOS command service');
      // Trigger initial filesystem fetch
      fetchFiles('/');
      fetchProcesses();
    };

    wsCmdRef.current.onmessage = (event) => {
      try {
        const data = JSON.parse(event.data);
        
        // Handle standard command terminal outputs
        if (data.result && (data.result.stdout !== undefined || data.result.stderr !== undefined)) {
          // If it was a system call executed from file explorer, intercept it
          if (data.id && String(data.id).startsWith('files_list_')) {
            const path = String(data.id).replace('files_list_', '');
            parseFilesOutput(data.result.stdout, path);
            return;
          }

          // If it was a process list update
          if (data.id === 'monitor_ps') {
            parseProcessesOutput(data.result.stdout);
            return;
          }

          if (data.result.stdout) {
            const lines = data.result.stdout.trim().split('\n');
            setTerminalLines(prev => [...prev, ...lines]);
          }
          if (data.result.stderr) {
            const lines = data.result.stderr.trim().split('\n');
            setTerminalLines(prev => [...prev, `[STDERR] ${lines.join('\n')}`]);
          }
          setTerminalLines(prev => [...prev, '']);
        } else if (data.result && data.result.output) {
          const lines = data.result.output.trim().split('\n');
          setTerminalLines(prev => [...prev, ...lines, '']);
        } else if (data.result && data.result.content) {
          setTerminalLines(prev => [...prev, data.result.content, '']);
        } else if (data.error) {
          // Intercept files fetch error
          if (data.id && String(data.id).startsWith('files_list_')) {
            setLoadingFiles(false);
            setFilesList([{ name: 'Error reading directory', isDir: false, size: 0 }]);
            return;
          }
          setTerminalLines(prev => [...prev, `[ERROR] ${data.error.message}`, '']);
        }
      } catch (e) {
        setTerminalLines(prev => [...prev, event.data, '']);
      }
    };

    // 2. UI Theme WebSocket
    wsUiRef.current = new WebSocket('ws://localhost:8000/ui');
    wsUiRef.current.onopen = () => {
      console.log('Connected to ACOS UI service');
      // Fetch initial theme
      wsUiRef.current.send(JSON.stringify({
        jsonrpc: '2.0',
        method: 'theme',
        params: { action: 'get' },
        id: 101
      }));
    };
    wsUiRef.current.onmessage = (event) => {
      try {
        const data = JSON.parse(event.data);
        if (data.result && data.result.theme) {
          setActiveTheme(data.result.theme);
        }
      } catch (e) {
        console.error('UI message parse error', e);
      }
    };

    // 3. System Info WebSocket
    wsSysRef.current = new WebSocket('ws://localhost:8000/system');
    wsSysRef.current.onopen = () => {
      console.log('Connected to ACOS system service');
      // Fetch system info
      wsSysRef.current.send(JSON.stringify({
        jsonrpc: '2.0',
        method: 'info',
        id: 201
      }));
    };
    wsSysRef.current.onmessage = (event) => {
      try {
        const data = JSON.parse(event.data);
        if (data.result) {
          setSystemInfo(prev => ({
            ...prev,
            kernel: data.result.kernel || prev.kernel,
            uptime: `${data.result.uptime || 0}s`,
            memory_total: data.result.memory_total || prev.memory_total,
            memory_free: data.result.memory_free || prev.memory_free,
            memory: `${Math.round(((data.result.memory_total || 2048) - (data.result.memory_free || 1024)) / 1024 / 1024)}MB / ${Math.round((data.result.memory_total || 2048) / 1024 / 1024)}MB`
          }));
        }
      } catch (e) {
        console.error('System message parse error', e);
      }
    };

    return () => {
      wsCmdRef.current?.close();
      wsUiRef.current?.close();
      wsSysRef.current?.close();
    };
  }, []);

  // Poll system processes periodically
  useEffect(() => {
    const interval = setInterval(() => {
      if (windows.find(w => w.id === 'monitor' && w.isOpen && !w.isMinimized)) {
        fetchProcesses();
      }
      // Periodically refresh memory metrics
      if (wsSysRef.current && wsSysRef.current.readyState === WebSocket.OPEN) {
        wsSysRef.current.send(JSON.stringify({
          jsonrpc: '2.0',
          method: 'info',
          id: Date.now()
        }));
      }
    }, 4000);
    return () => clearInterval(interval);
  }, [windows]);

  // Fetch directory contents via MCP Command WS using standard shell ls
  const fetchFiles = (path) => {
    if (!wsCmdRef.current || wsCmdRef.current.readyState !== WebSocket.OPEN) return;
    setLoadingFiles(true);
    setSelectedFile(null);
    
    // We run ls -p to append a '/' slash to directory basenames for easy parsing
    wsCmdRef.current.send(JSON.stringify({
      jsonrpc: '2.0',
      method: 'run',
      params: { cmd: `ls -p ${path}` },
      id: `files_list_${path}`
    }));
  };

  const parseFilesOutput = (stdout, path) => {
    setLoadingFiles(false);
    setCurrentPath(path);
    if (!stdout.trim()) {
      setFilesList([]);
      return;
    }

    const items = stdout.trim().split('\n').map(item => {
      const isDir = item.endsWith('/');
      return {
        name: isDir ? item.slice(0, -1) : item,
        isDir: isDir,
        size: isDir ? 0 : Math.floor(Math.random() * 450) + 12 // Simulated size for UI
      };
    });

    // Sort: directories first, then files alphabetically
    items.sort((a, b) => {
      if (a.isDir && !b.isDir) return -1;
      if (!a.isDir && b.isDir) return 1;
      return a.name.localeCompare(b.name);
    });

    setFilesList(items);
  };

  // Fetch Processes list via MCP Command WS
  const fetchProcesses = () => {
    if (!wsCmdRef.current || wsCmdRef.current.readyState !== WebSocket.OPEN) return;
    wsCmdRef.current.send(JSON.stringify({
      jsonrpc: '2.0',
      method: 'run',
      params: { cmd: 'ps' },
      id: 'monitor_ps'
    }));
  };

  const parseProcessesOutput = (stdout) => {
    if (!stdout.trim()) return;
    const lines = stdout.trim().split('\n');
    if (lines.length < 2) return;

    // Parse ps output. Structure: PID   EUID  EGID  STAT  CPU   AFFINITY   TIME        PRIVATE SHARED  NAME
    const parsed = [];
    // Skip headers
    for (let i = 1; i < lines.length; i++) {
      const cols = lines[i].trim().split(/\s+/);
      if (cols.length >= 10) {
        parsed.push({
          pid: cols[0],
          euid: cols[1],
          stat: cols[3],
          cpu: cols[4],
          time: cols[6],
          private_mem: cols[7],
          name: cols.slice(9).join(' ')
        });
      }
    }
    setProcesses(parsed);
  };

  // Kill Process Action
  const killProcess = (pid) => {
    if (!pid || !wsCmdRef.current || wsCmdRef.current.readyState !== WebSocket.OPEN) return;
    
    // Add to terminal logging
    setTerminalLines(prev => [...prev, `root:~# kill -9 ${pid}`]);
    
    wsCmdRef.current.send(JSON.stringify({
      jsonrpc: '2.0',
      method: 'run',
      params: { cmd: `kill -9 ${pid}` },
      id: Date.now()
    }));

    // Trigger AI Guardian alert
    setGuardianStatus('ALERT');
    setGuardianLogs(prev => [
      {
        id: Date.now(),
        type: 'alert',
        text: `CRITICAL: Root process termination requested on PID: ${pid} (${processes.find(p => p.pid === pid)?.name || 'unknown'})`
      },
      ...prev
    ]);

    setTimeout(() => {
      setGuardianStatus('SECURE');
      fetchProcesses();
    }, 1200);

    setSelectedPid(null);
  };

  // Navigate filesystem folders
  const handleFileDoubleClick = (item) => {
    if (item.isDir) {
      const separator = currentPath === '/' ? '' : '/';
      const newPath = `${currentPath}${separator}${item.name}`;
      fetchFiles(newPath);
    } else {
      setSelectedFile(item);
    }
  };

  const navigateBack = () => {
    if (currentPath === '/') return;
    const parts = currentPath.split('/');
    parts.pop();
    const newPath = parts.join('/') || '/';
    fetchFiles(newPath);
  };

  // Terminal Autocomplete Logic
  const handleTerminalInputChange = (val) => {
    setTerminalInput(val);
    if (!val.trim()) {
      setSuggestions([]);
      return;
    }

    // Filter matching commands
    const query = val.toLowerCase();
    const matches = ACOS_COMMANDS.filter(c => c.cmd.toLowerCase().startsWith(query));
    setSuggestions(matches);
    setSelectedSuggestionIndex(0);
  };

  const handleTerminalInputKeyDown = (e) => {
    if (suggestions.length > 0) {
      if (e.key === 'ArrowDown') {
        e.preventDefault();
        setSelectedSuggestionIndex(prev => (prev + 1) % suggestions.length);
      } else if (e.key === 'ArrowUp') {
        e.preventDefault();
        setSelectedSuggestionIndex(prev => (prev - 1 + suggestions.length) % suggestions.length);
      } else if (e.key === 'Tab') {
        e.preventDefault();
        acceptSuggestion(suggestions[selectedSuggestionIndex].cmd);
      } else if (e.key === 'Enter') {
        e.preventDefault();
        const completedCmd = suggestions[selectedSuggestionIndex].cmd;
        setTerminalInput(completedCmd);
        setSuggestions([]);
        executeCommand(completedCmd);
      } else if (e.key === 'Escape') {
        setSuggestions([]);
      }
    } else if (e.key === 'Enter') {
      executeCommand(terminalInput);
    }
  };

  const acceptSuggestion = (cmd) => {
    setTerminalInput(cmd);
    setSuggestions([]);
  };

  // Handle command execution
  const executeCommand = (cmdText) => {
    if (!cmdText.trim()) return;
    
    setTerminalLines(prev => [...prev, `root:~# ${cmdText}`]);
    setSuggestions([]);
    
    // Check if it's a theme switch shortcut
    if (cmdText.startsWith('ui theme set ')) {
      const themeName = cmdText.replace('ui theme set ', '').trim().replace(/['"{}name:]/g, '');
      changeTheme(themeName);
      setTerminalInput('');
      return;
    }
    
    // Send standard command to ACOS command service
    if (wsCmdRef.current && wsCmdRef.current.readyState === WebSocket.OPEN) {
      wsCmdRef.current.send(JSON.stringify({
        jsonrpc: '2.0',
        method: 'run',
        params: { cmd: cmdText },
        id: Date.now()
      }));
      
      // Simulate Guardian security checks
      triggerGuardianCheck(cmdText);
    } else {
      setTerminalLines(prev => [...prev, '[ERROR] Offline. WebSocket server not responding.', '']);
    }
    
    setTerminalInput('');
  };

  // Trigger simulated Guardian capabilities verification on every command
  const triggerGuardianCheck = (cmd) => {
    const isDangerous = cmd.includes('rm') || cmd.includes('delete') || cmd.includes('kill') || cmd.includes('reboot');
    
    setGuardianStatus(isDangerous ? 'ALERT' : 'SECURE');
    
    setTimeout(() => {
      setGuardianLogs(prev => [
        {
          id: Date.now(),
          type: isDangerous ? 'alert' : 'info',
          text: isDangerous 
            ? `WARNING: Capability probe blocked potential risk on command: "${cmd}"`
            : `Audit: Command "${cmd.split(' ')[0]}" verified & signed by policy supervisor.`
        },
        ...prev
      ]);
    }, 400);
  };

  // Handle dynamic theme switching
  const changeTheme = (name) => {
    if (wsUiRef.current && wsUiRef.current.readyState === WebSocket.OPEN) {
      wsUiRef.current.send(JSON.stringify({
        jsonrpc: '2.0',
        method: 'theme',
        params: { action: 'set', name: name },
        id: Date.now()
      }));
      setActiveTheme(name);
    }
  };

  // Render content of active applications
  const renderWindowContent = (id) => {
    switch (id) {
      case 'terminal':
        return (
          <div className="terminal-window" style={{ height: '100%', position: 'relative' }}>
            <div style={{ flex: 1, overflowY: 'auto' }}>
              {terminalLines.map((line, idx) => (
                <div key={idx} className="terminal-line">
                  {line.startsWith('root:~#') ? (
                    <span>
                      <span className="terminal-prompt">root:~#</span> {line.replace('root:~#', '')}
                    </span>
                  ) : line}
                </div>
              ))}
              <div ref={terminalEndRef} />
            </div>
            
            <div className="terminal-input-line" style={{ position: 'relative' }}>
              <span className="terminal-prompt">root:~#</span>
              <div className="terminal-input-container">
                <input
                  type="text"
                  className="terminal-input"
                  value={terminalInput}
                  onChange={(e) => handleTerminalInputChange(e.target.value)}
                  onKeyDown={handleTerminalInputKeyDown}
                  autoFocus={focusedWindowId === 'terminal'}
                  placeholder="Type command (autocomplete active, Tab to select)..."
                />
                
                {/* Autocomplete dropdown overlay */}
                {suggestions.length > 0 && (
                  <div className="autocomplete-popover glass glow-accent">
                    <div className="suggestions-list">
                      {suggestions.map((s, idx) => (
                        <div 
                          key={idx}
                          onClick={() => acceptSuggestion(s.cmd)}
                          onMouseEnter={() => setSelectedSuggestionIndex(idx)}
                          className={`suggestion-item ${idx === selectedSuggestionIndex ? 'active' : ''}`}
                        >
                          {s.cmd}
                        </div>
                      ))}
                    </div>
                    <div className="suggestion-tooltip glass">
                      <h5>{suggestions[selectedSuggestionIndex].cmd}</h5>
                      <p>{suggestions[selectedSuggestionIndex].desc}</p>
                      <div className="usage-code">Ex: {suggestions[selectedSuggestionIndex].usage}</div>
                    </div>
                  </div>
                )}
              </div>
              <button onClick={() => executeCommand(terminalInput)} style={{ background: 'transparent', border: 'none', cursor: 'pointer', color: 'var(--secondary-color)', paddingLeft: '8px' }}>
                <Send size={16} />
              </button>
            </div>
          </div>
        );

      case 'files':
        return (
          <div className="files-app">
            <div className="files-toolbar">
              <button 
                onClick={navigateBack} 
                disabled={currentPath === '/'}
                className="taskbar-btn" 
                style={{ width: '32px', height: '32px', borderRadius: '6px', opacity: currentPath === '/' ? 0.3 : 1 }}
              >
                <ArrowLeft size={16} />
              </button>
              <div className="path-breadcrumbs">{currentPath}</div>
              <button onClick={() => fetchFiles(currentPath)} className="taskbar-btn" style={{ width: '32px', height: '32px', borderRadius: '6px' }}>
                <RefreshCw size={14} />
              </button>
            </div>
            
            <div className="window-body">
              {loadingFiles ? (
                <div style={{ display: 'flex', flex: 1, alignItems: 'center', justifyContent: 'center', fontSize: '0.9rem', gap: '8px' }}>
                  <RefreshCw size={18} className="animate-spin" /> Fetching Redox files...
                </div>
              ) : (
                <div className="files-grid">
                  {filesList.map((item, idx) => (
                    <div 
                      key={idx} 
                      onDoubleClick={() => handleFileDoubleClick(item)}
                      onClick={() => setSelectedFile(item)}
                      className={`file-item ${selectedFile?.name === item.name ? 'selected' : ''}`}
                    >
                      {item.isDir ? (
                        <Folder size={38} style={{ color: 'var(--primary-color)' }} />
                      ) : (
                        <File size={38} style={{ color: 'var(--secondary-color)' }} />
                      )}
                      <span className="file-label">{item.name}</span>
                    </div>
                  ))}
                  {filesList.length === 0 && (
                    <div style={{ gridColumn: '1/-1', textAlign: 'center', opacity: 0.5, padding: '40px', fontSize: '0.85rem' }}>
                      Directory is empty
                    </div>
                  )}
                </div>
              )}
            </div>

            {selectedFile && (
              <div style={{ padding: '8px 16px', background: 'rgba(0,0,0,0.15)', borderTop: '1px solid var(--glass-border)', fontSize: '0.75rem', display: 'flex', justifyBetween: 'space-between' }}>
                <div><strong>Selected:</strong> {selectedFile.name} {selectedFile.isDir ? '(Directory)' : '(File)'}</div>
                {!selectedFile.isDir && <div style={{marginLeft: 'auto'}}><strong>Size:</strong> {selectedFile.size} bytes</div>}
              </div>
            )}
          </div>
        );

      case 'monitor':
        // Calculate RAM usage bar
        const memoryUsed = systemInfo.memory_total - systemInfo.memory_free;
        const memoryPercentage = Math.round((memoryUsed / systemInfo.memory_total) * 100) || 50;

        return (
          <div className="monitor-app">
            <div className="monitor-widgets">
              <div className="monitor-widget">
                <div style={{ fontSize: '0.75rem', opacity: 0.6, display: 'flex', alignItems: 'center', gap: '6px' }}>
                  <Cpu size={14} style={{ color: 'var(--accent-color)' }} /> SYSTEM KERNEL
                </div>
                <div style={{ fontWeight: '800', marginTop: '4px', fontSize: '0.95rem' }}>{systemInfo.kernel}</div>
              </div>
              
              <div className="monitor-widget">
                <div style={{ fontSize: '0.75rem', opacity: 0.6, display: 'flex', alignItems: 'center', gap: '6px' }}>
                  <HardDrive size={14} style={{ color: 'var(--secondary-color)' }} /> MEMORY UTILS (RAM)
                </div>
                <div style={{ display: 'flex', alignItems: 'center', gap: '10px', marginTop: '6px' }}>
                  <div style={{ flex: 1, height: '8px', background: 'rgba(255,255,255,0.05)', borderRadius: '4px', overflow: 'hidden' }}>
                    <div style={{ width: `${memoryPercentage}%`, height: '100%', background: 'var(--secondary-color)' }} />
                  </div>
                  <span style={{ fontSize: '0.8rem', fontFamily: 'var(--font-mono)' }}>{memoryPercentage}%</span>
                </div>
              </div>
            </div>

            <div className="processes-section">
              <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '8px' }}>
                <span style={{ fontSize: '0.8rem', fontWeight: '700', textTransform: 'uppercase', opacity: 0.7, display: 'flex', alignItems: 'center', gap: '6px' }}>
                  <Activity size={14} /> Active Redox Processes
                </span>
                <div style={{ display: 'flex', gap: '8px' }}>
                  <button 
                    onClick={() => killProcess(selectedPid)} 
                    disabled={!selectedPid}
                    className="taskbar-btn" 
                    style={{ width: 'auto', height: '28px', padding: '0 12px', borderRadius: '6px', fontSize: '0.75rem', background: selectedPid ? 'rgba(255,0,85,0.15)' : 'rgba(255,255,255,0.02)', borderColor: selectedPid ? 'var(--accent-color)' : 'var(--glass-border)' }}
                    title="Terminate Selected Task"
                  >
                    <Ban size={12} style={{ display: 'inline', marginRight: '4px' }} /> End Task
                  </button>
                  <button onClick={fetchProcesses} className="taskbar-btn" style={{ width: '28px', height: '28px', borderRadius: '6px' }}>
                    <RefreshCw size={12} />
                  </button>
                </div>
              </div>

              <div className="process-table-container">
                <table className="process-table">
                  <thead>
                    <tr>
                      <th>PID</th>
                      <th>Name</th>
                      <th>State</th>
                      <th>CPU</th>
                      <th>Private RAM</th>
                    </tr>
                  </thead>
                  <tbody>
                    {processes.map((proc, idx) => (
                      <tr 
                        key={idx} 
                        onClick={() => setSelectedPid(proc.pid)}
                        className={selectedPid === proc.pid ? 'selected' : ''}
                        style={{ cursor: 'pointer' }}
                      >
                        <td>{proc.pid}</td>
                        <td style={{ fontWeight: '600', color: proc.pid === '38' || proc.pid === '39' ? 'var(--fg-color)' : 'inherit' }}>
                          {proc.name}
                        </td>
                        <td>{proc.stat}</td>
                        <td>{proc.cpu}</td>
                        <td style={{ fontFamily: 'var(--font-mono)' }}>{proc.private_mem}</td>
                      </tr>
                    ))}
                    {processes.length === 0 && (
                      <tr>
                        <td colSpan="5" style={{ textAlign: 'center', opacity: 0.5, padding: '30px' }}>
                          No active processes loaded
                        </td>
                      </tr>
                    )}
                  </tbody>
                </table>
              </div>
            </div>
          </div>
        );

      case 'guardian':
        return (
          <div className="card-content" style={{ display: 'flex', flexDirection: 'column', gap: '16px', height: '100%', overflowY: 'auto' }}>
            <div className="guardian-status">
              <div className={`status-indicator ${guardianStatus === 'ALERT' ? 'danger' : ''}`} />
              <div>
                <div style={{ fontSize: '0.75rem', opacity: 0.6 }}>SYSTEM SECURITY STATE</div>
                <div style={{ fontWeight: '800', color: guardianStatus === 'ALERT' ? 'var(--accent-color)' : 'var(--secondary-color)' }}>
                  {guardianStatus}
                </div>
              </div>
            </div>
            
            <h4 style={{ fontSize: '0.8rem', opacity: 0.7, textTransform: 'uppercase', display: 'flex', alignItems: 'center', gap: '6px' }}>
              <Shield size={14} /> Active Security Audits
            </h4>
            
            <div className="guardian-logs" style={{ flex: 1, overflowY: 'auto' }}>
              <AnimatePresence>
                {guardianLogs.map(log => (
                  <motion.div
                    key={log.id}
                    initial={{ opacity: 0, y: -10 }}
                    animate={{ opacity: 1, y: 0 }}
                    className={`log-entry ${log.type === 'alert' ? 'alert' : ''}`}
                  >
                    {log.type === 'alert' && <AlertTriangle size={14} style={{ display: 'inline', marginRight: '6px', color: 'var(--accent-color)' }} />}
                    {log.text}
                  </motion.div>
                ))}
              </AnimatePresence>
            </div>
          </div>
        );
      default:
        return null;
    }
  };

  return (
    <div className={`theme-${activeTheme}`} style={{ height: '100%', display: 'flex', flexDirection: 'column', overflow: 'hidden', position: 'relative' }}>
      
      {/* Top Menu Bar */}
      <header className="top-bar glass glow-secondary" style={{ zIndex: 99999 }}>
        <div className="system-title">
          <Layers size={22} style={{ color: 'var(--accent-color)' }} />
          ACOS Horizon Desktop
        </div>
        <div className="top-bar-stats">
          <div style={{ display: 'flex', alignItems: 'center', gap: '4px' }}>
            <Cpu size={14} style={{ color: 'var(--primary-color)' }} />
            <span>MEM: {systemInfo.memory}</span>
          </div>
          <div style={{ display: 'flex', alignItems: 'center', gap: '4px', borderLeft: '1px solid var(--glass-border)', paddingLeft: '12px' }}>
            <Clock size={14} />
            <span>{currentTime}</span>
          </div>
        </div>
      </header>

      {/* Main Desktop Workspace */}
      <div className="horizon-desktop" ref={desktopRef}>
        
        {/* Desktop Application Icons Grid */}
        <div className="desktop-grid">
          <div className="desktop-icon" onDoubleClick={() => openWindow('terminal')}>
            <div className="taskbar-btn" style={{ border: 'none', background: 'rgba(0, 240, 255, 0.1)', color: 'var(--fg-color)' }}>
              <Terminal size={22} />
            </div>
            <span className="desktop-icon-label">Terminal</span>
          </div>
          
          <div className="desktop-icon" onDoubleClick={() => openWindow('files')}>
            <div className="taskbar-btn" style={{ border: 'none', background: 'rgba(255, 251, 0, 0.1)', color: 'var(--primary-color)' }}>
              <FolderOpen size={22} />
            </div>
            <span className="desktop-icon-label">Files</span>
          </div>

          <div className="desktop-icon" onDoubleClick={() => openWindow('monitor')}>
            <div className="taskbar-btn" style={{ border: 'none', background: 'rgba(0, 255, 102, 0.1)', color: 'var(--secondary-color)' }}>
              <Cpu size={22} />
            </div>
            <span className="desktop-icon-label">Monitor</span>
          </div>

          <div className="desktop-icon" onDoubleClick={() => openWindow('guardian')}>
            <div className="taskbar-btn" style={{ border: 'none', background: 'rgba(255, 0, 85, 0.1)', color: 'var(--accent-color)' }}>
              <Shield size={22} />
            </div>
            <span className="desktop-icon-label">Guardian</span>
          </div>
        </div>

        {/* Windows Rendering Area */}
        <AnimatePresence>
          {windows.map(win => {
            if (!win.isOpen) return null;
            return (
              <motion.div
                key={win.id}
                initial={{ opacity: 0, scale: 0.95 }}
                animate={{ 
                  opacity: win.isMinimized ? 0 : 1, 
                  scale: win.isMinimized ? 0.8 : 1,
                  y: win.isMinimized ? 500 : 0,
                  transition: { duration: 0.25 }
                }}
                drag={!win.isMaximized}
                dragConstraints={desktopRef}
                dragHandleClassName="window-titlebar"
                dragMomentum={false}
                dragElastic={0}
                onDragStart={() => focusWindow(win.id)}
                onClick={() => focusWindow(win.id)}
                style={{ 
                  zIndex: win.zIndex, 
                  left: win.isMaximized ? 0 : win.x, 
                  top: win.isMaximized ? 60 : win.y, 
                  width: win.isMaximized ? '100vw' : win.w, 
                  height: win.isMaximized ? 'calc(100vh - 124px)' : win.h,
                  display: win.isMinimized ? 'none' : 'flex'
                }}
                className={`floating-window glass ${focusedWindowId === win.id ? 'focused glow-accent' : ''} ${win.isMaximized ? 'maximized' : ''}`}
              >
                {/* Title Bar */}
                <div className="window-titlebar" onMouseDown={() => focusWindow(win.id)}>
                  <div className="window-title-info">
                    <win.icon size={15} style={{ color: focusedWindowId === win.id ? 'var(--fg-color)' : 'inherit' }} />
                    <span>{win.title}</span>
                  </div>
                  <div className="window-controls">
                    <button className="win-btn minimize" onClick={(e) => minimizeWindow(win.id, e)} title="Minimize">_</button>
                    <button className="win-btn maximize" onClick={(e) => toggleMaximizeWindow(win.id, e)} title={win.isMaximized ? "Restore" : "Maximize"}>⬜</button>
                    <button className="win-btn close" onClick={(e) => closeWindow(win.id, e)} title="Close">X</button>
                  </div>
                </div>
                
                {/* Window Body */}
                <div className="window-body">
                  {renderWindowContent(win.id)}
                </div>
              </motion.div>
            );
          })}
        </AnimatePresence>

      </div>

      {/* Start Menu Overlay (ACOS Orb Menu) */}
      <AnimatePresence>
        {isStartOpen && (
          <motion.div
            initial={{ opacity: 0, y: 50, scale: 0.95 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: 50, scale: 0.95 }}
            className="start-menu glass glow-accent"
          >
            <div className="start-menu-header">
              <Search size={16} style={{ color: 'var(--fg-color)' }} />
              <input
                type="text"
                placeholder="Search files, apps or MCP commands..."
                className="start-search"
                value={startSearch}
                onChange={(e) => setStartSearch(e.target.value)}
                autoFocus
              />
            </div>

            <div className="start-apps-list">
              <h5 style={{ fontSize: '0.75rem', opacity: 0.5, marginBottom: '8px', paddingLeft: '8px', textTransform: 'uppercase' }}>System Applications</h5>
              
              <div className="start-app-item" onClick={() => openWindow('terminal')}>
                <Terminal size={18} style={{ color: 'var(--fg-color)' }} />
                <div>
                  <div style={{ fontWeight: 600, fontSize: '0.85rem' }}>ACOS Terminal</div>
                  <div style={{ fontSize: '0.7rem', opacity: 0.6 }}>Interactive root ion shell</div>
                </div>
              </div>

              <div className="start-app-item" onClick={() => openWindow('files')}>
                <FolderOpen size={18} style={{ color: 'var(--primary-color)' }} />
                <div>
                  <div style={{ fontWeight: 600, fontSize: '0.85rem' }}>File Explorer</div>
                  <div style={{ fontSize: '0.7rem', opacity: 0.6 }}>Redox filesystem visual navigation</div>
                </div>
              </div>

              <div className="start-app-item" onClick={() => openWindow('monitor')}>
                <Cpu size={18} style={{ color: 'var(--secondary-color)' }} />
                <div>
                  <div style={{ fontWeight: 600, fontSize: '0.85rem' }}>System Monitor</div>
                  <div style={{ fontSize: '0.7rem', opacity: 0.6 }}>RAM and process live dashboard</div>
                </div>
              </div>

              <div className="start-app-item" onClick={() => openWindow('guardian')}>
                <Shield size={18} style={{ color: 'var(--accent-color)' }} />
                <div>
                  <div style={{ fontWeight: 600, fontSize: '0.85rem' }}>AI Guardian Hub</div>
                  <div style={{ fontSize: '0.7rem', opacity: 0.6 }}>Security auditing capabilities</div>
                </div>
              </div>
            </div>

            <div className="start-menu-footer">
              <div>System: <strong>Redox OS</strong></div>
              <div>User: <strong>root</strong></div>
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      {/* Dock / Taskbar */}
      <footer className="taskbar glass glow-accent" style={{ zIndex: 99999 }}>
        <div className="taskbar-icons">
          
          {/* Start Orb */}
          <button 
            onClick={() => setIsStartOpen(!isStartOpen)} 
            className="taskbar-btn start-orb" 
            title="ACOS Menu"
          >
            <Layers size={22} style={{ color: '#fff' }} />
          </button>
          
          <div style={{ width: '1px', height: '24px', background: 'var(--glass-border)', margin: '0 4px' }} />

          {/* Quick theme changers */}
          <button onClick={() => changeTheme('cyberpunk')} className={`taskbar-btn ${activeTheme === 'cyberpunk' ? 'active' : ''}`} title="Cyberpunk Theme">
            <Palette size={20} style={{ color: '#ff0055' }} />
          </button>
          <button onClick={() => changeTheme('matrix')} className={`taskbar-btn ${activeTheme === 'matrix' ? 'active' : ''}`} title="Matrix Theme">
            <Palette size={20} style={{ color: '#00ff41' }} />
          </button>
          <button onClick={() => changeTheme('brutalism')} className={`taskbar-btn ${activeTheme === 'brutalism' ? 'active' : ''}`} title="Brutalism Theme">
            <Palette size={20} style={{ color: '#0000ff' }} />
          </button>
          <button onClick={() => changeTheme('dark')} className={`taskbar-btn ${activeTheme === 'dark' ? 'active' : ''}`} title="Dracula Theme">
            <Palette size={20} style={{ color: '#bd93f9' }} />
          </button>
          
          <div style={{ width: '1px', height: '24px', background: 'var(--glass-border)', margin: '0 4px' }} />

          {/* Quick Window Launchers */}
          <button onClick={() => openWindow('terminal')} className={`taskbar-btn ${windows.find(w => w.id === 'terminal')?.isOpen ? 'active' : ''}`} title="ACOS Terminal">
            <Terminal size={20} />
          </button>
          <button onClick={() => openWindow('files')} className={`taskbar-btn ${windows.find(w => w.id === 'files')?.isOpen ? 'active' : ''}`} title="File Explorer">
            <FolderOpen size={20} />
          </button>
          <button onClick={() => openWindow('monitor')} className={`taskbar-btn ${windows.find(w => w.id === 'monitor')?.isOpen ? 'active' : ''}`} title="System Monitor">
            <Cpu size={20} />
          </button>
          <button onClick={() => openWindow('guardian')} className={`taskbar-btn ${windows.find(w => w.id === 'guardian')?.isOpen ? 'active' : ''}`} title="AI Guardian">
            <Shield size={20} />
          </button>
        </div>
      </footer>

    </div>
  );
}

export default App;
