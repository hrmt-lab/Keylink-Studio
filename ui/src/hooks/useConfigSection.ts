import { useEffect, useState } from "react";
import { saveConfig } from "../api";
import type { AppConfig } from "../types";

interface Options<T> {
  config: AppConfig;
  setConfig: (c: AppConfig) => void;
  /** Pick the editable slice from the full config. */
  select: (c: AppConfig) => T;
  /** Merge an edited slice back into the full config. */
  apply: (c: AppConfig, draft: T) => AppConfig;
}

interface Result<T> {
  draft: T;
  setDraft: React.Dispatch<React.SetStateAction<T>>;
  isDirty: boolean;
  saving: boolean;
  error: string | null;
  setError: (value: string | null) => void;
  save: () => Promise<void>;
}

/**
 * Shared draft/dirty/save/error state for settings pages. The draft is reset
 * whenever the upstream config changes (e.g. after a save or reload).
 */
export function useConfigSection<T>({ config, setConfig, select, apply }: Options<T>): Result<T> {
  const [draft, setDraft] = useState<T>(() => select(config));
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setDraft(select(config));
    // select is treated as stable; reset only when config changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [config]);

  const isDirty = JSON.stringify(draft) !== JSON.stringify(select(config));

  const save = async () => {
    setSaving(true);
    setError(null);
    try {
      const updated = apply(config, draft);
      await saveConfig(updated);
      setConfig(updated);
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  return { draft, setDraft, isDirty, saving, error, setError, save };
}
