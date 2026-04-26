import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  Activity, Play, StopCircle, Cpu, HardDrive,
  TerminalSquare, Settings, CheckCircle2,
  XCircle, AlertTriangle, Info, Loader2,
} from "lucide-react";
import "./App.css";

interface SessionReq {
  binary: string;
  args: string[];
  cpu_quota_secs: number | null;
  timeout_secs: number | null;
  mem_limit_kb: number | null;
}

interface ProgressPayload {
  cpu_used: number;
  cpu_quota: number | null;
  mem_kb: number;
  mem_limit_kb: number | null;
}

interface RunSummary {
  status: string;
  cpu_user: number;
  cpu_sys: number;
  peak_mem_kb: number;
}

type LogLevel = "info" | "success" | "warning" | "error" | "metric";

interface LogEntry {
  id: number;
  ts: string;
  level: LogLevel;
  message: string;
  detail?: string;
}

let logId = 0;
function makeLog(level: LogLevel, message: string, detail?: string): LogEntry {
  return {
    id: ++logId,
    ts: new Date().toLocaleTimeString("en-US", { hour12: false }),
    level,
    message,
    detail,
  };
}

const LEVEL_STYLES: Record<LogLevel, { text: string; icon: React.ReactNode }> = {
  info:    { text: "text-cyan-400",    icon: <Info size={12} /> },
  success: { text: "text-emerald-400", icon: <CheckCircle2 size={12} /> },
  warning: { text: "text-amber-400",   icon: <AlertTriangle size={12} /> },
  error:   { text: "text-red-400",     icon: <XCircle size={12} /> },
  metric:  { text: "text-white/50",    icon: <Activity size={12} /> },
};

function LogLine({ entry }: { entry: LogEntry }) {
  const { text, icon } = LEVEL_STYLES[entry.level];
  return (
    <div className="flex items-start gap-2 py-0.5 group animate-[fadeIn_0.2s_ease-out]">
      <span className="text-white/25 shrink-0 w-16 text-[10px] pt-0.5 font-mono">{entry.ts}</span>
      <span className={`shrink-0 mt-0.5 ${text}`}>{icon}</span>
      <div className="flex-1 min-w-0">
        <span className={`${text} font-medium`}>{entry.message}</span>
        {entry.detail && (
          <span className="text-white/35 ml-2 text-[11px]">{entry.detail}</span>
        )}
      </div>
    </div>
  );
}

const MAX_LOGS = 200;

export default function App() {
  const [billingMode, setBillingMode] = useState<"prepaid" | "postpaid">("prepaid");
  const [binary, setBinary] = useState("");
  const [argsStr, setArgsStr] = useState("");
  const [cpuQuota, setCpuQuota] = useState("");
  const [timeoutSecs, setTimeoutSecs] = useState("");
  const [memLimitMb, setMemLimitMb] = useState("");

  const [isRunning, setIsRunning] = useState(false);
  const [metrics, setMetrics] = useState<ProgressPayload | null>(null);
  const [logs, setLogs] = useState<LogEntry[]>([
    makeLog("info", "FMS Engine standing by.", "Waiting for session configuration."),
  ]);
  const [errorMsg, setErrorMsg] = useState("");

  const logEndRef = useRef<HTMLDivElement>(null);

  const pushLog = (level: LogLevel, message: string, detail?: string) =>
    setLogs(prev => [...prev, makeLog(level, message, detail)].slice(-MAX_LOGS));

  // Auto-scroll log to bottom on new entries
  useEffect(() => {
    logEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [logs]);

  useEffect(() => {
    let lastMetricLog = 0;
    const unlisten = listen<ProgressPayload>("fms-progress", (event) => {
      const p = event.payload;
      setMetrics(p);

      // Throttle metric log entries to every 1 second to avoid overflow
      const now = Date.now();
      if (now - lastMetricLog > 1000) {
        lastMetricLog = now;
        const cpuPct = p.cpu_quota
          ? `${((p.cpu_used / p.cpu_quota) * 100).toFixed(1)}% of quota`
          : "no quota limit";
        const memMb = (p.mem_kb / 1024).toFixed(1);
        const memDesc = p.mem_limit_kb
          ? `${((p.mem_kb / p.mem_limit_kb) * 100).toFixed(1)}% of limit`
          : "no mem limit";
        pushLog(
          "metric",
          `Tick — CPU ${p.cpu_used.toFixed(3)}s (${cpuPct})`,
          `RAM ${memMb} MB (${memDesc})`
        );
      }
    });

    return () => { unlisten.then(f => f()); };
  }, []);

  const handleLaunch = async (e: React.FormEvent) => {
    e.preventDefault();
    setErrorMsg("");
    setMetrics(null);
    setIsRunning(true);

    const args = argsStr.trim() ? argsStr.split(/\s+/) : [];
    const binaryLabel = `${binary.trim()}${args.length ? " " + args.join(" ") : ""}`;

    pushLog("info", "Session initiated.", `Billing mode: ${billingMode}`);
    pushLog("info", `Spawning subprocess…`, binaryLabel);

    if (billingMode === "prepaid" && cpuQuota) {
      pushLog("info", `CPU quota enforced.`, `${cpuQuota}s of CPU time allocated.`);
    } else {
      pushLog("info", "Postpaid mode active.", "CPU usage will be measured and billed after completion.");
    }
    if (timeoutSecs) pushLog("info", `Wall-clock timeout set.`, `Process will be terminated after ${timeoutSecs}s of real time.`);
    if (memLimitMb)  pushLog("info", `Memory cap configured.`, `Hard limit of ${memLimitMb} MB — exceeding it will kill the process.`);

    const req: SessionReq = {
      binary: binary.trim(),
      args,
      cpu_quota_secs: billingMode === "prepaid" && cpuQuota ? parseFloat(cpuQuota) : null,
      timeout_secs: timeoutSecs ? parseFloat(timeoutSecs) : null,
      mem_limit_kb: memLimitMb ? parseFloat(memLimitMb) * 1024 : null,
    };

    try {
      const result: RunSummary = await invoke<RunSummary>("spawn_fms_session", { req });

      const totalCpu = (result.cpu_user + result.cpu_sys).toFixed(3);
      const peakMem  = (result.peak_mem_kb / 1024).toFixed(1);

      if (result.status === "KilledUser") {
        pushLog("warning", "Process killed manually by user.",
          `CPU consumed before kill: ${totalCpu}s  |  Peak RAM: ${peakMem} MB`);
        return;
      }

      if (result.status === "Exited") {
        pushLog("success", "Process completed successfully.", `Total CPU: ${totalCpu}s  |  Peak RAM: ${peakMem} MB`);
        pushLog("success", `User mode CPU: ${result.cpu_user.toFixed(3)}s`, `Kernel mode CPU: ${result.cpu_sys.toFixed(3)}s`);
      } else if (result.status === "KilledTimeout") {
        pushLog("warning", "Process terminated — wall-clock timeout reached.",
          `The process exceeded the configured real-time limit and was force-killed by FMS.`);
        pushLog("info", `CPU consumed before kill: ${totalCpu}s`, `Peak RAM before kill: ${peakMem} MB`);
      } else if (result.status === "KilledMemory") {
        pushLog("error", "Process terminated — memory limit exceeded.",
          `Peak allocation of ${peakMem} MB breached the configured ceiling. Process tree killed.`);
        pushLog("info", `CPU consumed before kill: ${totalCpu}s`);
      }

      if (billingMode === "postpaid") {
        pushLog("warning", `[POSTPAID] Bill generated.`, `You owe ${totalCpu} CPU-seconds for this session.`);
      } else if (req.cpu_quota_secs !== null) {
        const remaining = Math.max(0, req.cpu_quota_secs - parseFloat(totalCpu));
        pushLog("info", `Remaining CPU quota: ${remaining.toFixed(3)}s`);
      }

    } catch (err: any) {
      const raw = err.toString();
      // Ignore the expected error when user intentionally kills the session
      if (!raw.includes("channel") && !raw.includes("recv")) {
        setErrorMsg(raw);
        pushLog("error", "Failed to launch subprocess.", raw);
      }
    } finally {
      setIsRunning(false);
      pushLog("info", "Session closed. Engine returning to standby.");
    }
  };

  const handleKill = async () => {
    try {
      await invoke("kill_fms_session");
      pushLog("warning", "Kill signal sent.", "Waiting for process tree to terminate…");
    } catch (err: any) {
      pushLog("error", "Kill failed.", err.toString());
    }
  };

  return (
    <main className="min-h-screen p-8 max-w-5xl mx-auto flex flex-col gap-8">
      {/* Header */}
      <header className="flex items-center justify-between">
        <div className="flex items-center gap-3">
          <div className="p-3 bg-accent-gold/20 text-accent-light rounded-2xl shadow-[0_0_20px_rgba(202,138,4,0.3)]">
            <Activity size={28} />
          </div>
          <div>
            <h1 className="text-3xl font-bold tracking-tight text-transparent bg-clip-text bg-gradient-to-r from-accent-light to-white">
              FMS Hub
            </h1>
            <p className="text-white/50 text-sm">Fat Management System Engine</p>
          </div>
        </div>
        <div className={`px-4 py-1.5 rounded-full text-sm font-medium border flex items-center gap-2 ${isRunning ? "bg-emerald-500/10 border-emerald-500/20 text-emerald-400" : "bg-white/5 border-white/10 text-white/40"}`}>
          <div className={`w-2 h-2 rounded-full ${isRunning ? "bg-emerald-400 animate-pulse" : "bg-white/20"}`} />
          {isRunning ? "Process Active" : "Standby"}
        </div>
      </header>

      <div className="grid grid-cols-1 lg:grid-cols-12 gap-8">
        {/* Left: Form */}
        <div className="lg:col-span-5 space-y-6">
          <form className="glass-panel p-6 space-y-5" onSubmit={handleLaunch}>
            <div className="space-y-4">
              <h2 className="flex items-center gap-2 text-lg font-semibold text-white/90 pb-2 border-b border-white/10">
                <Settings size={18} className="text-accent-gold" /> Billing Mode
              </h2>
              <div className="flex bg-primary-900/50 p-1 rounded-xl border border-white/5">
                {(["prepaid", "postpaid"] as const).map(mode => (
                  <button
                    key={mode}
                    type="button"
                    onClick={() => setBillingMode(mode)}
                    disabled={isRunning}
                    className={`flex-1 py-2 text-sm font-medium rounded-lg transition-all capitalize ${billingMode === mode ? "bg-accent-gold text-primary-900 shadow-md" : "text-white/50 hover:text-white"}`}
                  >
                    {mode}
                  </button>
                ))}
              </div>

              <h2 className="flex items-center gap-2 text-lg font-semibold text-white/90 pb-2 border-b border-white/10 pt-2">
                <TerminalSquare size={18} className="text-accent-gold" /> Execution Profile
              </h2>

              <div className="space-y-4">
                <div>
                  <label className="block text-xs font-medium text-white/50 uppercase tracking-wider mb-1.5">Binary Target</label>
                  <input type="text" className="glass-input" value={binary} onChange={e => setBinary(e.target.value)} placeholder="/bin/ls" required disabled={isRunning} />
                </div>
                <div>
                  <label className="block text-xs font-medium text-white/50 uppercase tracking-wider mb-1.5">Arguments (space-separated)</label>
                  <input type="text" className="glass-input" value={argsStr} onChange={e => setArgsStr(e.target.value)} placeholder="-la /var" disabled={isRunning} />
                </div>
              </div>

              <h2 className="flex items-center gap-2 text-lg font-semibold text-white/90 pb-2 border-b border-white/10 pt-2">
                <Settings size={18} className="text-accent-gold" /> Resource Constraints
              </h2>

              <div className="grid grid-cols-2 gap-4">
                <div className={billingMode === "postpaid" ? "opacity-30 pointer-events-none" : ""}>
                  <label className="block text-xs font-medium text-white/50 uppercase tracking-wider mb-1.5">CPU Quota (s)</label>
                  <input type="number" step="0.001" required={billingMode === "prepaid"} className="glass-input" value={cpuQuota} onChange={e => setCpuQuota(e.target.value)} placeholder={billingMode === "prepaid" ? "sec" : "Unlimited"} disabled={isRunning || billingMode === "postpaid"} />
                </div>
                <div>
                  <label className="block text-xs font-medium text-white/50 uppercase tracking-wider mb-1.5">Mem Max (MB)</label>
                  <input type="number" className="glass-input" value={memLimitMb} onChange={e => setMemLimitMb(e.target.value)} placeholder="Unlimited" disabled={isRunning} />
                </div>
                <div className="col-span-2">
                  <label className="block text-xs font-medium text-white/50 uppercase tracking-wider mb-1.5">Wall-clock Timeout (s)</label>
                  <input type="number" step="0.001" className="glass-input" value={timeoutSecs} onChange={e => setTimeoutSecs(e.target.value)} placeholder="Unlimited" disabled={isRunning} />
                </div>
              </div>
            </div>

            <div className="flex gap-3 mt-4">
              <button type="submit" disabled={isRunning || !binary} className="flex-1 glass-button">
                {isRunning
                  ? <><Loader2 className="animate-spin" size={20} /> Executing…</>
                  : <><Play size={20} /> Launch Process</>}
              </button>
              {isRunning && (
                <button
                  type="button"
                  onClick={handleKill}
                  className="px-4 py-3 bg-red-500/10 hover:bg-red-500/20 border border-red-500/30 text-red-400 rounded-xl transition-all flex items-center gap-2 font-medium"
                >
                  <StopCircle size={18} />
                  Kill
                </button>
              )}
            </div>

            {errorMsg && (
              <div className="p-3 bg-red-500/10 border border-red-500/20 text-red-400 text-sm rounded-lg">{errorMsg}</div>
            )}
          </form>
        </div>

        {/* Right: Telemetry */}
        <div className="lg:col-span-7 space-y-6 flex flex-col">
          <div className="glass-panel p-6 flex-1 flex flex-col min-h-0">
            <h2 className="flex items-center gap-2 text-lg font-semibold text-white/90 pb-4 shrink-0">
              <Activity size={18} className="text-accent-gold" /> Live Telemetry Stream
            </h2>

            {/* Metric gauges */}
            <div className="grid grid-cols-2 gap-4 mb-5 shrink-0">
              <div className="bg-primary-900/40 border border-white/5 p-4 rounded-xl">
                <div className="flex items-center gap-2 text-white/50 text-sm mb-2"><Cpu size={16} /> CPU Time</div>
                <div className="text-3xl font-light text-white">
                  {metrics ? metrics.cpu_used.toFixed(3) : "0.000"}<span className="text-white/30 text-lg">s</span>
                </div>
                {metrics?.cpu_quota && (
                  <div className="mt-2 h-1.5 w-full bg-white/10 rounded-full overflow-hidden">
                    <div className="h-full bg-accent-gold transition-all duration-300" style={{ width: `${Math.min((metrics.cpu_used / metrics.cpu_quota) * 100, 100)}%` }} />
                  </div>
                )}
              </div>
              <div className="bg-primary-900/40 border border-white/5 p-4 rounded-xl">
                <div className="flex items-center gap-2 text-white/50 text-sm mb-2"><HardDrive size={16} /> Memory RSS</div>
                <div className="text-3xl font-light text-white">
                  {metrics ? (metrics.mem_kb / 1024).toFixed(1) : "0.0"}<span className="text-white/30 text-lg">MB</span>
                </div>
                {metrics?.mem_limit_kb && (
                  <div className="mt-2 h-1.5 w-full bg-white/10 rounded-full overflow-hidden">
                    <div className="h-full bg-emerald-400 transition-all duration-300" style={{ width: `${Math.min((metrics.mem_kb / metrics.mem_limit_kb) * 100, 100)}%` }} />
                  </div>
                )}
              </div>
            </div>

            {/* Log stream */}
            <div className="flex-1 bg-black/40 border border-white/5 rounded-xl overflow-hidden flex flex-col min-h-0">
              <div className="flex items-center gap-2 px-4 py-2 border-b border-white/5 shrink-0">
                <div className="flex gap-1.5">
                  <div className="w-2.5 h-2.5 rounded-full bg-red-500/60" />
                  <div className="w-2.5 h-2.5 rounded-full bg-amber-400/60" />
                  <div className="w-2.5 h-2.5 rounded-full bg-emerald-400/60" />
                </div>
                <span className="text-white/20 text-xs font-mono ml-2">fms — session log</span>
                {isRunning && <span className="ml-auto text-cyan-400 text-xs font-mono animate-pulse">● LIVE</span>}
              </div>
              <div className="flex-1 overflow-y-auto p-4 font-mono text-xs leading-relaxed space-y-px scroll-smooth max-h-72">
                {logs.map(e => <LogLine key={e.id} entry={e} />)}
                <div ref={logEndRef} />
              </div>
            </div>
          </div>
        </div>
      </div>
    </main>
  );
}
