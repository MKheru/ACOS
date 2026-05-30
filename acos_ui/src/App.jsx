import React, { useState, useEffect, useRef } from 'react';
import { Terminal, Shield, Cpu, RefreshCw, Layers, Layout, Palette, FolderOpen, AlertTriangle, Send } from 'lucide-react';
import { motion, AnimatePresence } from 'framer-motion';

function App() {
  const [activeTheme, setActiveTheme] = useState('cyberpunk');
  const [terminalInput, setTerminalInput] = useState('');
  const [terminalLines, setTerminalLines] = useState([
    'Welcome to ACOS — Agent-Centric Operating System v0.9.0',
    'MCP Systems active. WebSocket bridge online at ws://localhost:8000',
    'Type standard shell commands below (e.g. ls, ps, cat /etc/hostname)',
    'Or query MCP services directly (e.g. ui theme list)',
    ''
  ]);
  
  const [guardianStatus, setGuardianStatus] = useState('SECURE');
  const [guardianLogs, setGuardianLogs] = useState([
    { id: 1, type: 'info', text: 'ACOS Security Guardian auto-started.' },
    { id: 2, type: 'info', text: 'Kernel capability policies loaded (deny-by-default).' },
    { id: 3, type: 'info', text: 'Active monitoring on all 16 MCP namespaces.' }
  ]);
  
  const [systemInfo, setSystemInfo] = useState({
    kernel: 'ACOS-Redox-0.5.12',
    uptime: 'calculating...',
    memory: '2048 MB'
  });

  const terminalEndRef = useRef(null);
  
  // WebSockets instances
  const wsCmdRef = useRef(null);
  const wsUiRef = useRef(null);
  const wsSysRef = useRef(null);

  // Auto-scroll terminal
  useEffect(() => {
    terminalEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [terminalLines]);

  // Connect to ACOS mcpd system WebSockets
  useEffect(() => {
    // 1. Command WebSocket
    wsCmdRef.current = new WebSocket('ws://localhost:8000/command');
    wsCmdRef.current.onopen = () => {
      console.log('Connected to ACOS command service');
    };
    wsCmdRef.current.onmessage = (event) => {
      try {
        const data = JSON.parse(event.data);
        if (data.result && (data.result.stdout !== undefined || data.result.stderr !== undefined)) {
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
            memory: `${Math.round((data.result.memory_total - data.result.memory_free) / 1024 / 1024)}MB / ${Math.round(data.result.memory_total / 1024 / 1024)}MB`
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

  // Handle command execution
  const executeCommand = (cmdText) => {
    if (!cmdText.trim()) return;
    
    setTerminalLines(prev => [...prev, `root:~# ${cmdText}`]);
    
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
    const isDangerous = cmd.includes('rm') || cmd.includes('delete') || cmd.includes('sudo');
    
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

  return (
    <div className={`theme-${activeTheme}`} style={{ height: '100%', display: 'flex', flexDirection: 'column', gap: '20px' }}>
      
      {/* Top Bar / Header */}
      <header className="top-bar glass glow-secondary">
        <div className="system-title">
          <Layers size={24} style={{ color: 'var(--accent-color)' }} />
          ACOS Semantic UI Canvas
        </div>
        <div className="top-bar-stats">
          <div><span style={{ color: 'var(--accent-color)' }}>KERNEL:</span> {systemInfo.kernel}</div>
          <div><span style={{ color: 'var(--primary-color)' }}>UPTIME:</span> {systemInfo.uptime}</div>
          <div><span style={{ color: 'var(--secondary-color)' }}>MEM:</span> {systemInfo.memory}</div>
        </div>
      </header>

      {/* Main Bento Grid Workspace */}
      <main className="bento-grid">
        
        {/* Left Card: Full interactive ACOS shell */}
        <section className="bento-card glass">
          <div className="card-header">
            <span className="card-title">
              <Terminal size={18} style={{ color: 'var(--secondary-color)' }} />
              ACOS Interactive Shell (ion)
            </span>
            <span style={{ fontSize: '0.8rem', color: 'var(--primary-color)' }}>ROOT SESSION</span>
          </div>
          <div className="card-content" style={{ background: 'rgba(0,0,0,0.15)', padding: '0' }}>
            <div className="terminal-window">
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
              
              <div className="terminal-input-line">
                <span className="terminal-prompt">root:~#</span>
                <input
                  type="text"
                  className="terminal-input"
                  value={terminalInput}
                  onChange={(e) => setTerminalInput(e.target.value)}
                  onKeyDown={(e) => e.key === 'Enter' && executeCommand(terminalInput)}
                  autoFocus
                  placeholder="Type shell command..."
                />
                <button onClick={() => executeCommand(terminalInput)} style={{ background: 'transparent', border: 'none', cursor: 'pointer', color: 'var(--secondary-color)' }}>
                  <Send size={18} />
                </button>
              </div>
            </div>
          </div>
        </section>

        {/* Right Card: AI Supervisor / Guardian Panel */}
        <section className="bento-card glass">
          <div className="card-header">
            <span className="card-title">
              <Shield size={18} style={{ color: 'var(--accent-color)' }} />
              AI Guardian Supervisor
            </span>
          </div>
          <div className="card-content">
            <div className="guardian-status">
              <div className={`status-indicator ${guardianStatus === 'ALERT' ? 'danger' : ''}`} />
              <div>
                <div style={{ fontSize: '0.75rem', opacity: 0.6 }}>SYSTEM STATE</div>
                <div style={{ fontWeight: '800', color: guardianStatus === 'ALERT' ? 'var(--accent-color)' : 'var(--secondary-color)' }}>
                  {guardianStatus}
                </div>
              </div>
            </div>
            
            <h4 style={{ fontSize: '0.8rem', opacity: 0.7, marginBottom: '12px', textTransform: 'uppercase' }}>Active Security Audits</h4>
            
            <div className="guardian-logs">
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
        </section>
        
      </main>

      {/* Dock / Taskbar */}
      <footer className="taskbar glass glow-accent">
        <div className="taskbar-icons">
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
          <div style={{ width: '1px', height: '24px', background: 'var(--glass-border)', margin: '0 8px' }} />
          {/* Quick actions */}
          <button onClick={() => executeCommand('ls -la')} className="taskbar-btn" title="List Directory Files">
            <FolderOpen size={20} />
          </button>
          <button onClick={() => executeCommand('mcp-query process list')} className="taskbar-btn" title="View Process List">
            <Cpu size={20} />
          </button>
          <button onClick={() => {
            if (wsSysRef.current && wsSysRef.current.readyState === WebSocket.OPEN) {
              wsSysRef.current.send(JSON.stringify({ jsonrpc: '2.0', method: 'info', id: Date.now() }));
            }
          }} className="taskbar-btn" title="Refresh System Stats">
            <RefreshCw size={20} />
          </button>
        </div>
      </footer>

    </div>
  );
}

export default App;
