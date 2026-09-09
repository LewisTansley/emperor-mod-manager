import { Component, useCallback, useEffect, useState, type ReactNode } from "react";
import { api } from "./api";
import type {
  AutoHdrVkConfig,
  AutoHdrVkGlobal,
  GameToolOverrides,
  GameToolsResponse,
  LsfgVkConfig,
  LsfgVkGlobal,
  LsfgVkProfile,
  ManagedGame,
  ToolsStatusResponse,
  ToolGameState,
  ToolStatus,
} from "./types";

const defaultToolState = (): ToolGameState => ({
  enabled: false,
  use_global_settings: true,
});

const defaultOverrides = (): GameToolOverrides => ({
  lsfg_vk: defaultToolState(),
  autohdr_vk: defaultToolState(),
  executables: [],
});

const defaultLsfgGlobal = (): LsfgVkGlobal => ({
  dll: null,
  allow_fp16: true,
});

const defaultLsfgProfile = (): LsfgVkProfile => ({
  name: "Default",
  active_in: [],
  multiplier: 2,
  target_fps: 0,
  flow_scale: 1,
  performance_mode: false,
  pacing: "none",
  gpu: null,
});

const defaultHdrGlobal = (): AutoHdrVkGlobal => ({
  intensity: 0.5,
  color_intensity: 0.33,
  expansion_shape: 0.55,
  black_floor: 0,
  highlight_stretch: 0.45,
  encoding: "auto",
  set_hdr_metadata: true,
  prefer_hdr_swapchain: true,
  enabled: true,
});

function normalizeLsfgConfig(raw: unknown): LsfgVkConfig {
  const r = raw as Record<string, unknown> | null | undefined;
  const profiles = (r?.profiles ?? r?.profile) as LsfgVkProfile[] | undefined;
  return {
    global: { ...defaultLsfgGlobal(), ...(r?.global as LsfgVkGlobal | undefined) },
    profiles:
      Array.isArray(profiles) && profiles.length > 0
        ? profiles
        : [defaultLsfgProfile()],
  };
}

function normalizeHdrConfig(raw: unknown): AutoHdrVkConfig {
  const r = raw as Record<string, unknown> | null | undefined;
  const profiles = (r?.profiles ?? r?.profile) as AutoHdrVkConfig["profiles"] | undefined;
  return {
    global: { ...defaultHdrGlobal(), ...(r?.global as AutoHdrVkGlobal | undefined) },
    profiles: Array.isArray(profiles) ? profiles : [],
  };
}

type ToolsWorkspaceState = "loading" | "error" | "not_linux" | "ready";

export class ToolsErrorBoundary extends Component<
  { children: ReactNode },
  { error: string | null }
> {
  state = { error: null as string | null };

  static getDerivedStateFromError(error: unknown) {
    return { error: String(error) };
  }

  render() {
    if (this.state.error) {
      return (
        <section className="panel">
          <div className="panel-head">
            <h1>Tools</h1>
          </div>
          <div className="detail-body">
            <p className="error-banner">Tools panel crashed: {this.state.error}</p>
            <button type="button" onClick={() => this.setState({ error: null })}>
              Try again
            </button>
          </div>
        </section>
      );
    }
    return this.props.children;
  }
}

function ToolInstallCard({
  label,
  status,
  repo,
  onInstall,
  onCheckUpdates,
  busy,
  checkingUpdates,
}: {
  label: string;
  status: ToolStatus;
  repo: string;
  onInstall: () => void;
  onCheckUpdates: () => void;
  busy: boolean;
  checkingUpdates: boolean;
}) {
  return (
    <div className="panel tool-card">
      <div className="panel-head">
        <h2>{label}</h2>
        <div className="panel-head-actions">
          <button type="button" onClick={onCheckUpdates} disabled={busy || checkingUpdates}>
            {checkingUpdates ? "Checking…" : "Check updates"}
          </button>
          <button type="button" onClick={onInstall} disabled={busy || checkingUpdates}>
            {status.installed ? "Update" : "Install"}
          </button>
        </div>
      </div>
      <div className="detail-body">
        <p>
          <strong>Status:</strong>{" "}
          {status.installed
            ? `Installed${status.version ? ` (${status.version})` : ""}`
            : "Not installed"}
        </p>
        {status.latest_version && (
          <p>
            <strong>Latest:</strong> {status.latest_version}
            {status.update_available ? " (update available)" : ""}
          </p>
        )}
        <p className="muted">
          <a href={`https://github.com/${repo}`} target="_blank" rel="noreferrer">
            github.com/{repo}
          </a>
        </p>
      </div>
    </div>
  );
}

function LsfgGlobalSettings({
  config,
  onChange,
  onSave,
  saving,
}: {
  config: LsfgVkConfig;
  onChange: (cfg: LsfgVkConfig) => void;
  onSave: () => void;
  saving: boolean;
}) {
  const global = config.global ?? defaultLsfgGlobal();
  const profile = config.profiles?.[0] ?? defaultLsfgProfile();
  const setProfile = (patch: Partial<LsfgVkProfile>) => {
    const profiles = [...(config.profiles ?? [defaultLsfgProfile()])];
    profiles[0] = { ...profile, ...patch };
    onChange({ ...config, global, profiles });
  };
  return (
    <div className="panel">
      <div className="panel-head">
        <h2>lsfg-vk global settings</h2>
        <div className="panel-head-actions">
          <button type="button" onClick={onSave} disabled={saving}>
            Save
          </button>
        </div>
      </div>
      <div className="detail-body tool-settings">
        <label>
          Lossless.dll path
          <input
            value={global.dll ?? ""}
            onChange={(e) =>
              onChange({
                ...config,
                global: { ...global, dll: e.target.value || null },
                profiles: config.profiles ?? [defaultLsfgProfile()],
              })
            }
          />
        </label>
        <label className="checkbox">
          <input
            type="checkbox"
            checked={global.allow_fp16 ?? true}
            onChange={(e) =>
              onChange({
                ...config,
                global: { ...global, allow_fp16: e.target.checked },
                profiles: config.profiles ?? [defaultLsfgProfile()],
              })
            }
          />
          Allow half-precision (FP16)
        </label>
        <label>
          Multiplier
          <input
            type="number"
            min={2}
            value={profile.multiplier ?? 2}
            onChange={(e) => setProfile({ multiplier: Number(e.target.value) || 2 })}
          />
        </label>
        <label>
          Flow scale
          <input
            type="number"
            min={0.25}
            max={1}
            step={0.05}
            value={profile.flow_scale ?? 1}
            onChange={(e) => setProfile({ flow_scale: Number(e.target.value) || 1 })}
          />
        </label>
        <label>
          Target FPS (0 = off)
          <input
            type="number"
            min={0}
            value={profile.target_fps ?? 0}
            onChange={(e) => setProfile({ target_fps: Number(e.target.value) || 0 })}
          />
        </label>
        <label className="checkbox">
          <input
            type="checkbox"
            checked={profile.performance_mode ?? false}
            onChange={(e) => setProfile({ performance_mode: e.target.checked })}
          />
          Performance mode
        </label>
        <label>
          GPU
          <input
            value={profile.gpu ?? ""}
            onChange={(e) => setProfile({ gpu: e.target.value || null })}
          />
        </label>
      </div>
    </div>
  );
}

function AutoHdrGlobalSettings({
  config,
  onChange,
  onSave,
  saving,
}: {
  config: AutoHdrVkConfig;
  onChange: (cfg: AutoHdrVkConfig) => void;
  onSave: () => void;
  saving: boolean;
}) {
  const g = config.global ?? defaultHdrGlobal();
  const setGlobal = (patch: Partial<AutoHdrVkGlobal>) =>
    onChange({
      ...config,
      global: { ...g, ...patch },
      profiles: config.profiles ?? [],
    });
  return (
    <div className="panel">
      <div className="panel-head">
        <h2>AutoHDR-VK global settings</h2>
        <div className="panel-head-actions">
          <button type="button" onClick={onSave} disabled={saving}>
            Save
          </button>
        </div>
      </div>
      <div className="detail-body tool-settings">
        <label>
          Intensity
          <input
            type="range"
            min={0}
            max={1}
            step={0.01}
            value={g.intensity ?? 0.5}
            onChange={(e) => setGlobal({ intensity: Number(e.target.value) })}
          />
          <span>{(g.intensity ?? 0).toFixed(2)}</span>
        </label>
        <label>
          Color intensity
          <input
            type="range"
            min={0}
            max={1}
            step={0.01}
            value={g.color_intensity ?? 0.33}
            onChange={(e) => setGlobal({ color_intensity: Number(e.target.value) })}
          />
        </label>
        <label>
          Expansion shape
          <input
            type="range"
            min={0}
            max={1}
            step={0.01}
            value={g.expansion_shape ?? 0.55}
            onChange={(e) => setGlobal({ expansion_shape: Number(e.target.value) })}
          />
        </label>
        <label>
          Black floor
          <input
            type="range"
            min={0}
            max={1}
            step={0.01}
            value={g.black_floor ?? 0}
            onChange={(e) => setGlobal({ black_floor: Number(e.target.value) })}
          />
        </label>
        <label>
          Highlight stretch
          <input
            type="range"
            min={0}
            max={2}
            step={0.05}
            value={g.highlight_stretch ?? 0.45}
            onChange={(e) => setGlobal({ highlight_stretch: Number(e.target.value) })}
          />
        </label>
        <label>
          Encoding
          <select
            value={g.encoding ?? "auto"}
            onChange={(e) => setGlobal({ encoding: e.target.value })}
          >
            <option value="auto">auto</option>
            <option value="pq">pq</option>
            <option value="scrgb">scrgb</option>
            <option value="sdr_preview">sdr_preview</option>
          </select>
        </label>
        <label className="checkbox">
          <input
            type="checkbox"
            checked={g.prefer_hdr_swapchain ?? true}
            onChange={(e) => setGlobal({ prefer_hdr_swapchain: e.target.checked })}
          />
          Prefer HDR swapchain
        </label>
        <label className="checkbox">
          <input
            type="checkbox"
            checked={g.set_hdr_metadata ?? true}
            onChange={(e) => setGlobal({ set_hdr_metadata: e.target.checked })}
          />
          Set HDR metadata
        </label>
      </div>
    </div>
  );
}

export function ToolsWorkspace() {
  const [uiState, setUiState] = useState<ToolsWorkspaceState>("loading");
  const [status, setStatus] = useState<ToolsStatusResponse | null>(null);
  const [lsfgConfig, setLsfgConfig] = useState<LsfgVkConfig | null>(null);
  const [hdrConfig, setHdrConfig] = useState<AutoHdrVkConfig | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [checkingUpdates, setCheckingUpdates] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setError(null);
    setUiState("loading");
    try {
      const [s, lsfg, hdr] = await Promise.all([
        api.getToolsStatus(),
        api.getLsfgVkConfig(),
        api.getAutohdrVkConfig(),
      ]);
      setStatus(s);
      setLsfgConfig(normalizeLsfgConfig(lsfg));
      setHdrConfig(normalizeHdrConfig(hdr));
      setUiState(s.platform_linux ? "ready" : "not_linux");
    } catch (e) {
      setError(String(e));
      setUiState("error");
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  async function checkUpdates() {
    setCheckingUpdates(true);
    setError(null);
    try {
      const updates = await api.checkToolUpdates();
      setStatus((prev) =>
        prev
          ? {
              ...prev,
              lsfg_vk: { ...prev.lsfg_vk, ...updates.lsfg_vk },
              autohdr_vk: { ...prev.autohdr_vk, ...updates.autohdr_vk },
            }
          : prev,
      );
    } catch (e) {
      setError(String(e));
    } finally {
      setCheckingUpdates(false);
    }
  }

  async function install(tool: "lsfg_vk" | "autohdr_vk") {
    setBusy(tool);
    setError(null);
    try {
      const s = await api.installTool(tool);
      setStatus(s);
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  }

  if (uiState === "loading") {
    return (
      <section className="panel">
        <div className="panel-head">
          <h1>Tools</h1>
        </div>
        <div className="detail-body">
          <p>Loading tools…</p>
        </div>
      </section>
    );
  }

  if (uiState === "error") {
    return (
      <section className="panel">
        <div className="panel-head">
          <h1>Tools</h1>
        </div>
        <div className="detail-body">
          {error && <p className="error-banner">{error}</p>}
          <button type="button" onClick={() => void refresh()}>
            Retry
          </button>
        </div>
      </section>
    );
  }

  if (uiState === "not_linux" || !status) {
    return (
      <section className="panel">
        <div className="panel-head">
          <h1>Tools</h1>
        </div>
        <div className="detail-body">
          <p>Vulkan tools (lsfg-vk, AutoHDR-VK) are only available on Linux.</p>
        </div>
      </section>
    );
  }

  return (
    <section className="tools-workspace">
      <div className="panel-head page-head">
        <h1>Tools</h1>
        <p className="muted">
          Install and configure lsfg-vk frame generation and AutoHDR-VK for all games.
        </p>
      </div>
      {error && <p className="error-banner">{error}</p>}
      <div className="tools-grid">
        <ToolInstallCard
          label="lsfg-vk"
          status={status.lsfg_vk}
          repo={status.lsfg_vk_repo}
          onInstall={() => install("lsfg_vk")}
          onCheckUpdates={() => void checkUpdates()}
          busy={busy === "lsfg_vk"}
          checkingUpdates={checkingUpdates}
        />
        <ToolInstallCard
          label="AutoHDR-VK"
          status={status.autohdr_vk}
          repo={status.autohdr_vk_repo}
          onInstall={() => install("autohdr_vk")}
          onCheckUpdates={() => void checkUpdates()}
          busy={busy === "autohdr_vk"}
          checkingUpdates={checkingUpdates}
        />
      </div>
      {lsfgConfig && (
        <LsfgGlobalSettings
          config={lsfgConfig}
          onChange={setLsfgConfig}
          saving={busy === "save_lsfg"}
          onSave={async () => {
            if (!lsfgConfig) return;
            setBusy("save_lsfg");
            try {
              await api.setLsfgVkConfig(lsfgConfig);
            } catch (e) {
              setError(String(e));
            } finally {
              setBusy(null);
            }
          }}
        />
      )}
      {hdrConfig && (
        <AutoHdrGlobalSettings
          config={hdrConfig}
          onChange={setHdrConfig}
          saving={busy === "save_hdr"}
          onSave={async () => {
            if (!hdrConfig) return;
            setBusy("save_hdr");
            try {
              await api.setAutohdrVkConfig(hdrConfig);
            } catch (e) {
              setError(String(e));
            } finally {
              setBusy(null);
            }
          }}
        />
      )}
    </section>
  );
}

export function GameToolsPanel({ game }: { game: ManagedGame }) {
  const [data, setData] = useState<GameToolsResponse | null>(null);
  const [overrides, setOverrides] = useState<GameToolOverrides>(defaultOverrides());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setError(null);
    try {
      const res = await api.getGameTools(game.id);
      setData(res);
      setOverrides(res.overrides ?? defaultOverrides());
    } catch (e) {
      setError(String(e));
    }
  }, [game.id]);

  useEffect(() => {
    void load();
  }, [load]);

  async function save(patch?: Partial<GameToolOverrides>) {
    const next = { ...overrides, ...patch };
    setOverrides(next);
    setBusy(true);
    setError(null);
    try {
      const res = await api.setGameTools(game.id, next);
      setData(res);
      setOverrides(res.overrides ?? defaultOverrides());
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  function toggleTool(tool: "lsfg_vk" | "autohdr_vk", enabled: boolean) {
    void save({
      [tool]: { ...(overrides[tool] ?? defaultToolState()), enabled },
    } as Partial<GameToolOverrides>);
  }

  function toggleGlobal(tool: "lsfg_vk" | "autohdr_vk", useGlobal: boolean) {
    void save({
      [tool]: { ...(overrides[tool] ?? defaultToolState()), use_global_settings: useGlobal },
    } as Partial<GameToolOverrides>);
  }

  return (
    <div className="detail-body game-tools">
      {error && <p className="error-banner">{error}</p>}
      <div className="panel">
        <h3>Executables</h3>
        <p className="muted">Matched against lsfg-vk profiles and AutoHDR-VK per-exe overrides.</p>
        <ul>
          {(data?.executables ?? []).map((exe) => (
            <li key={exe}>{exe}</li>
          ))}
          {(data?.executables ?? []).length === 0 && (
            <li className="muted">No .exe files detected</li>
          )}
        </ul>
        {data?.steam_app_id && (
          <p className="muted">Steam App ID: {data.steam_app_id}</p>
        )}
        {data?.launch_options && (
          <p className="muted launch-options-preview">Launch options: {data.launch_options}</p>
        )}
        <button type="button" onClick={() => void load()} disabled={busy}>
          Refresh detection
        </button>
        <button
          type="button"
          onClick={() => void api.syncGameTools(game.id).then(load)}
          disabled={busy}
        >
          Re-sync Steam launch options
        </button>
      </div>

      <div className="panel tool-toggle-panel">
        <h3>lsfg-vk</h3>
        <label className="checkbox">
          <input
            type="checkbox"
            checked={overrides.lsfg_vk?.enabled ?? false}
            onChange={(e) => toggleTool("lsfg_vk", e.target.checked)}
            disabled={busy}
          />
          Enable for this game
        </label>
        <label className="checkbox">
          <input
            type="checkbox"
            checked={overrides.lsfg_vk?.use_global_settings ?? true}
            onChange={(e) => toggleGlobal("lsfg_vk", e.target.checked)}
            disabled={busy || !overrides.lsfg_vk?.enabled}
          />
          Use global settings
        </label>
      </div>

      <div className="panel tool-toggle-panel">
        <h3>AutoHDR-VK</h3>
        <label className="checkbox">
          <input
            type="checkbox"
            checked={overrides.autohdr_vk?.enabled ?? false}
            onChange={(e) => toggleTool("autohdr_vk", e.target.checked)}
            disabled={busy}
          />
          Enable for this game
        </label>
        <label className="checkbox">
          <input
            type="checkbox"
            checked={overrides.autohdr_vk?.use_global_settings ?? true}
            onChange={(e) => toggleGlobal("autohdr_vk", e.target.checked)}
            disabled={busy || !overrides.autohdr_vk?.enabled}
          />
          Use global settings
        </label>
      </div>
    </div>
  );
}
