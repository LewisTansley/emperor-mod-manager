import type { Settings } from "./types";

type AccountConnectionsProps = {
  settings: Settings | null;
  apiKeyInput: string;
  onApiKeyInput: (value: string) => void;
  onSaveApiKey: () => void;
  onClearApiKey: () => void;
  modioKeyInput: string;
  onModioKeyInput: (value: string) => void;
  onSaveModioApiKey: () => void;
  onClearModioApiKey: () => void;
  signInDisabled: boolean;
  onSignIn: () => void;
  onClearWebsiteSession: () => void;
};

export function AccountConnections({
  settings,
  apiKeyInput,
  onApiKeyInput,
  onSaveApiKey,
  onClearApiKey,
  modioKeyInput,
  onModioKeyInput,
  onSaveModioApiKey,
  onClearModioApiKey,
  signInDisabled,
  onSignIn,
  onClearWebsiteSession,
}: AccountConnectionsProps) {
  return (
    <>
      <h2>Nexus API key</h2>
      <p>
        Paste your personal API key from{" "}
        <a
          href="https://www.nexusmods.com/users/myaccount?tab=api"
          target="_blank"
          rel="noreferrer"
        >
          Account → API Access
        </a>
        . The API key authenticates browsing and Premium downloads.
      </p>
      {settings?.user && (
        <div className="status-card">
          Signed in as <strong>{settings.user.name}</strong>
          {settings.user.is_premium ? (
            <span className="pill ok">Premium</span>
          ) : (
            <span className="pill warn">Free — Download Assist</span>
          )}
        </div>
      )}
      <div className="row">
        <input
          type="password"
          placeholder="Nexus API key"
          value={apiKeyInput}
          onChange={(e) => onApiKeyInput(e.target.value)}
        />
        <button onClick={onSaveApiKey} disabled={!apiKeyInput.trim()}>
          Save & validate
        </button>
      </div>
      {settings?.has_api_key && (
        <button className="danger" onClick={onClearApiKey}>
          Clear Nexus API key
        </button>
      )}

      <h2 style={{ marginTop: "1.5rem" }}>Nexus website sign-in</h2>
      <p>
        Use Stay signed in. The session is stored separately from the API key.
      </p>
      {settings?.nexus_website_signed_in && (
        <div className="status-card">
          <span className="pill ok">Website session saved</span>
        </div>
      )}
      <div className="actions">
        <button
          type="button"
          disabled={signInDisabled}
          title={
            signInDisabled
              ? "Finish or cancel the Download Assist queue before signing in."
              : undefined
          }
          onClick={onSignIn}
        >
          Log in to Nexus
        </button>
        <button type="button" onClick={onClearWebsiteSession}>
          Clear Nexus website session
        </button>
      </div>

      <h2 style={{ marginTop: "1.5rem" }}>mod.io</h2>
      <p>
        Optional read-only API key from{" "}
        <a href="https://mod.io/me/access" target="_blank" rel="noreferrer">
          mod.io → Access
        </a>
        . Used to browse and download mods for games with a mod.io game ID.
        OAuth, uploads, and ratings are not supported.
      </p>
      {settings?.has_modio_api_key && (
        <div className="status-card">
          <span className="pill ok">mod.io API key saved</span>
        </div>
      )}
      <div className="row">
        <input
          type="password"
          placeholder="mod.io API key"
          value={modioKeyInput}
          onChange={(e) => onModioKeyInput(e.target.value)}
        />
        <button onClick={onSaveModioApiKey} disabled={!modioKeyInput.trim()}>
          Save & validate
        </button>
      </div>
      {settings?.has_modio_api_key && (
        <button className="danger" onClick={onClearModioApiKey}>
          Clear mod.io API key
        </button>
      )}
    </>
  );
}
