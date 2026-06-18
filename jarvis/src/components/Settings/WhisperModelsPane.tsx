import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";
import {
  formatByteSize,
  normalizeLocalWhisperModel,
  type LocalWhisperModelId,
  whisperModelShortTitle,
  whisperModelSubtitle,
} from "../editor/SettingsPanel.logic";
import { formatUserError } from "../../utils/userErrors";
import { EditorSpinner } from "../editor/EditorSpinner";

export type WhisperModelEntry = {
  id: string;
  label: string;
  installed: boolean;
  sizeBytes: number | null;
  sizeBytesApprox: number;
  active: boolean;
  downloading: boolean;
  downloadProgressPct: number | null;
  managed: boolean;
};

type ListWhisperModelsPayload = {
  models: WhisperModelEntry[];
};

type WhisperModelDownloadEvent = {
  modelId: string;
  phase: "started" | "progress" | "done" | "error" | string;
  progressPct?: number | null;
  message?: string | null;
};

type WhisperModelsPaneProps = {
  activeModelId: LocalWhisperModelId;
  onSelectModel: (modelId: LocalWhisperModelId) => Promise<void>;
  onNotice: (text: string) => void;
};

export function WhisperModelsPane({
  activeModelId,
  onSelectModel,
  onNotice,
}: WhisperModelsPaneProps) {
  const [models, setModels] = useState<WhisperModelEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [busyId, setBusyId] = useState<string | null>(null);

  const refreshModels = useCallback(async () => {
    const payload = await invoke<ListWhisperModelsPayload>("list_whisper_models");
    setModels(payload.models);
  }, []);

  useEffect(() => {
    let mounted = true;
    void refreshModels()
      .catch((err: unknown) => {
        if (!mounted) return;
        onNotice(formatUserError(err, "Could not load speech models."));
      })
      .finally(() => {
        if (mounted) setLoading(false);
      });
    return () => {
      mounted = false;
    };
  }, [onNotice, refreshModels]);

  useEffect(() => {
    const unlisten = listen<WhisperModelDownloadEvent>("whisper-model-download", (event) => {
      const { modelId, phase, progressPct, message } = event.payload;
      setModels((prev) =>
        prev.map((m) => {
          if (m.id !== modelId) return m;
          if (phase === "started" || phase === "progress") {
            return {
              ...m,
              downloading: true,
              downloadProgressPct:
                progressPct ?? m.downloadProgressPct ?? 0,
            };
          }
          if (phase === "done") {
            return {
              ...m,
              downloading: false,
              downloadProgressPct: 100,
              installed: true,
              managed: true,
            };
          }
          if (phase === "error") {
            return {
              ...m,
              downloading: false,
              downloadProgressPct: null,
            };
          }
          return m;
        }),
      );
      if (phase === "done") {
        void refreshModels();
        if (message) onNotice(message);
      } else if (phase === "error" && message) {
        onNotice(message);
      }
    });
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, [onNotice, refreshModels]);

  const onDownload = (modelId: string) => {
    setBusyId(modelId);
    void invoke("download_whisper_model", { modelId })
      .then(() => refreshModels())
      .catch((err: unknown) => {
        onNotice(formatUserError(err, "Download failed."));
      })
      .finally(() => setBusyId(null));
  };

  const onDelete = (modelId: string) => {
    setBusyId(modelId);
    void invoke<ListWhisperModelsPayload>("delete_whisper_model", { modelId })
      .then((payload) => {
        setModels(payload.models);
        onNotice(`${modelId} removed.`);
      })
      .catch((err: unknown) => {
        onNotice(formatUserError(err, "Could not delete model."));
      })
      .finally(() => setBusyId(null));
  };

  if (loading) {
    return (
      <p className="editor-settings-hint editor-whisper-models-loading" role="status">
        <EditorSpinner /> Loading speech models…
      </p>
    );
  }

  return (
    <div className="editor-whisper-models">
      <p className="editor-settings-hint editor-whisper-models-lede">
        Downloads start automatically when you select a model or begin listening.
      </p>
      <ul
        className="editor-whisper-models-grid"
        role="radiogroup"
        aria-label="Whisper speech models"
      >
        {models.map((model) => {
          const id = normalizeLocalWhisperModel(model.id);
          const isActive = activeModelId === id;
          const isBusy = busyId === model.id || model.downloading;
          const sizeLabel = model.installed
            ? formatByteSize(model.sizeBytes)
            : `~${formatByteSize(model.sizeBytesApprox)}`;

          return (
            <li
              key={model.id}
              className={`editor-whisper-model-card${isActive ? " is-active" : ""}${model.downloading ? " is-downloading" : ""}`}
            >
              <label className="editor-whisper-model-card-surface">
                <input
                  type="radio"
                  name="whisper-model"
                  className="editor-whisper-model-radio"
                  checked={isActive}
                  disabled={isBusy}
                  onChange={() => void onSelectModel(id)}
                />
                <span className="editor-whisper-model-card-head">
                  <span className="editor-whisper-model-card-title">
                    {whisperModelShortTitle(id)}
                  </span>
                  <span className="editor-whisper-model-badges">
                    {isActive && (
                      <span className="editor-whisper-model-badge editor-whisper-model-badge--active">
                        Active
                      </span>
                    )}
                    {model.installed && !model.managed && (
                      <span className="editor-whisper-model-badge">Bundled</span>
                    )}
                    {model.installed && model.managed && !isActive && (
                      <span className="editor-whisper-model-badge">Installed</span>
                    )}
                  </span>
                </span>
                <span className="editor-whisper-model-card-desc">
                  {whisperModelSubtitle(id)}
                </span>
                <span className="editor-whisper-model-card-size">{sizeLabel}</span>
                {model.downloading && (
                  <span className="editor-whisper-model-progress" role="status">
                    <span className="editor-whisper-model-progress-track">
                      <span
                        className="editor-whisper-model-progress-fill"
                        style={{ transform: `scaleX(${(model.downloadProgressPct ?? 0) / 100})` }}
                      />
                    </span>
                    <span className="editor-whisper-model-progress-label">
                      Downloading {model.downloadProgressPct ?? 0}%
                    </span>
                  </span>
                )}
                <span className="editor-whisper-model-card-actions">
                  {!model.installed && (
                    <button
                      type="button"
                      className="editor-btn editor-btn--primary editor-whisper-model-action"
                      disabled={isBusy}
                      onClick={(e) => {
                        e.preventDefault();
                        e.stopPropagation();
                        onDownload(model.id);
                      }}
                    >
                      {model.downloading ? "Downloading…" : "Download"}
                    </button>
                  )}
                  {model.managed && (
                    <button
                      type="button"
                      className="editor-btn editor-whisper-model-action"
                      disabled={isBusy || isActive}
                      title={
                        isActive
                          ? "Select another model before deleting the active one."
                          : undefined
                      }
                      onClick={(e) => {
                        e.preventDefault();
                        e.stopPropagation();
                        onDelete(model.id);
                      }}
                    >
                      Delete
                    </button>
                  )}
                </span>
              </label>
            </li>
          );
        })}
      </ul>
    </div>
  );
}
