import { invoke } from "@tauri-apps/api/core";
import { useState } from "react";
import { useSettingsStore } from "../../store/settingsStore";
import { formatUserError } from "../../utils/userErrors";

type AppIndexPaneProps = {
  onNotice: (text: string) => void;
};

type AppIndexPhase = "waiting" | "scanning" | "ready";

function deriveAppIndexPhase(count: number | null, scanning: boolean): AppIndexPhase {
  if (scanning) return "scanning";
  if (count === null) return "waiting";
  return "ready";
}

function phaseLabel(phase: AppIndexPhase): string {
  switch (phase) {
    case "scanning":
      return "Scanning";
    case "waiting":
      return "Starting up";
    case "ready":
      return "Ready";
  }
}

function formatIndexCount(count: number | null): string {
  if (count === null) return "—";
  return count.toLocaleString();
}

export function AppIndexPane({ onNotice }: AppIndexPaneProps) {
  const appIndexCount = useSettingsStore((s) => s.appIndexCount);
  const appIndexScanning = useSettingsStore((s) => s.appIndexScanning);
  const [rescanning, setRescanning] = useState(false);

  const isBusy = rescanning || appIndexScanning;
  const phase = deriveAppIndexPhase(appIndexCount, isBusy);

  const onRescan = () => {
    setRescanning(true);
    void invoke("rescan_app_index")
      .catch((err: unknown) => {
        onNotice(formatUserError(err, "Rescan failed."));
      })
      .finally(() => setRescanning(false));
  };

  return (
    <div
      className="editor-settings-content editor-settings-content--app-index"
      aria-labelledby="editor-settings-nav-app-index"
    >
      <h3 className="editor-app-index-title">App Index</h3>

      <section className="editor-app-index-panel" aria-live="polite">
        <div className="editor-app-index-status">
          <div className="editor-app-index-count-block">
            <span className="editor-app-index-count" aria-label="Indexed applications">
              {formatIndexCount(appIndexCount)}
            </span>
            <div className="editor-app-index-count-meta">
              <span className="editor-app-index-count-caption">applications indexed</span>
              <span className="editor-app-index-phase" role="status">
                <span
                  className={`editor-app-index-phase-dot editor-app-index-phase-dot--${phase}`}
                  aria-hidden
                />
                <span className="editor-app-index-phase-label">{phaseLabel(phase)}</span>
                {phase === "scanning" && (
                  <span className="editor-settings-spinner" aria-hidden />
                )}
              </span>
            </div>
          </div>

          <button
            type="button"
            className={`editor-btn editor-btn--primary editor-hotkey-record-btn editor-app-index-rescan-btn${isBusy ? " editor-hotkey-record-btn--active" : ""}`}
            disabled={isBusy}
            onClick={onRescan}
            aria-busy={isBusy}
          >
            {isBusy ? "Rescanning…" : "Rescan"}
          </button>
        </div>
      </section>
    </div>
  );
}
