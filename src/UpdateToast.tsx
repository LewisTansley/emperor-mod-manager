import { Reveal } from "./motion";

type UpdateToastProps = {
  show: boolean;
  version: string;
  progress: string | null;
  canInstall: boolean;
  checking: boolean;
  installing: boolean;
  onUpdate: () => void;
  onOpenRelease: () => void;
  onLater: () => void;
};

/** Small offer to install a newer app release, or open the release page. */
export function UpdateToast({
  show,
  version,
  progress,
  canInstall,
  checking,
  installing,
  onUpdate,
  onOpenRelease,
  onLater,
}: UpdateToastProps) {
  return (
    <Reveal show={show} className="update-toast-slot">
      <div className="update-toast" role="status">
        <p className="update-toast-title">Update available</p>
        <p className="update-toast-version">v{version}</p>
        {progress ? <p className="update-toast-progress">{progress}</p> : null}
        <div className="update-toast-actions">
          {canInstall ? (
            <button type="button" disabled={checking || installing} onClick={onUpdate}>
              {installing ? "Updating…" : "Update"}
            </button>
          ) : (
            <button type="button" disabled={installing} onClick={onOpenRelease}>
              Open release
            </button>
          )}
          <button
            type="button"
            className="update-toast-later"
            disabled={installing}
            onClick={onLater}
          >
            Later
          </button>
        </div>
      </div>
    </Reveal>
  );
}
