import { createPortal } from "react-dom";
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyEvent,
} from "react";

export type EditorSelectOption = {
  value: string;
  label: string;
  disabled?: boolean;
};

export type EditorSelectProps = {
  id: string;
  value: string;
  onChange: (value: string) => void;
  options: EditorSelectOption[];
  placeholder?: string;
  disabled?: boolean;
  className?: string;
};

type MenuPos = { top: number; left: number; width: number; maxHeight: number };

export function EditorSelect({
  id,
  value,
  onChange,
  options,
  placeholder = "Select…",
  disabled,
  className,
}: EditorSelectProps) {
  const [open, setOpen] = useState(false);
  const [pos, setPos] = useState<MenuPos | null>(null);
  const [highlightedIndex, setHighlightedIndex] = useState(0);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLUListElement>(null);

  const enabledIndices = useMemo(
    () => options.map((o, i) => (!o.disabled ? i : -1)).filter((i) => i >= 0),
    [options],
  );

  const selectedLabel = useMemo(() => options.find((o) => o.value === value)?.label, [options, value]);

  const syncPosition = useCallback(() => {
    const el = triggerRef.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    const margin = 8;
    const gap = 4;
    const top = r.bottom + gap;
    const maxHeight = Math.max(120, Math.min(320, window.innerHeight - top - margin));
    const width = Math.min(r.width, window.innerWidth - margin * 2);
    const left = Math.min(Math.max(margin, r.left), window.innerWidth - margin - width);
    setPos({ top, left, width, maxHeight });
  }, []);

  useLayoutEffect(() => {
    if (!open) {
      setPos(null);
      return;
    }
    syncPosition();
    window.addEventListener("resize", syncPosition);
    window.addEventListener("scroll", syncPosition, true);
    const el = triggerRef.current;
    const ro = el ? new ResizeObserver(() => queueMicrotask(syncPosition)) : null;
    if (el && ro) ro.observe(el);
    return () => {
      window.removeEventListener("resize", syncPosition);
      window.removeEventListener("scroll", syncPosition, true);
      ro?.disconnect();
    };
  }, [open, syncPosition]);

  useEffect(() => {
    if (!open) return;
    const idx = options.findIndex((o) => o.value === value && !o.disabled);
    const firstEnabled = enabledIndices[0];
    if (idx >= 0) setHighlightedIndex(idx);
    else if (firstEnabled !== undefined) setHighlightedIndex(firstEnabled);
  }, [open, value, options, enabledIndices]);

  useEffect(() => {
    if (!open) return;
    const onDocMouseDown = (e: MouseEvent) => {
      const t = e.target as Node;
      if (triggerRef.current?.contains(t)) return;
      if (menuRef.current?.contains(t)) return;
      setOpen(false);
    };
    document.addEventListener("mousedown", onDocMouseDown);
    return () => document.removeEventListener("mousedown", onDocMouseDown);
  }, [open]);

  const moveHighlight = useCallback(
    (delta: 1 | -1) => {
      if (enabledIndices.length === 0) return;
      const posIn = enabledIndices.indexOf(highlightedIndex);
      if (posIn < 0) {
        setHighlightedIndex(
          delta > 0 ? enabledIndices[0]! : enabledIndices[enabledIndices.length - 1]!,
        );
        return;
      }
      const nextPos = (posIn + delta + enabledIndices.length) % enabledIndices.length;
      setHighlightedIndex(enabledIndices[nextPos]!);
    },
    [enabledIndices, highlightedIndex],
  );

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        setOpen(false);
        queueMicrotask(() => triggerRef.current?.focus());
        return;
      }
      if (e.key === "Tab") {
        setOpen(false);
        return;
      }
      if (e.key === "ArrowDown") {
        e.preventDefault();
        moveHighlight(1);
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        moveHighlight(-1);
        return;
      }
      if (e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        const opt = options[highlightedIndex];
        if (opt && !opt.disabled) {
          onChange(opt.value);
          setOpen(false);
          queueMicrotask(() => triggerRef.current?.focus());
        }
      }
    };
    document.addEventListener("keydown", onKey, true);
    return () => document.removeEventListener("keydown", onKey, true);
  }, [open, highlightedIndex, moveHighlight, options, onChange]);

  const toggle = () => {
    if (disabled) return;
    setOpen((o) => !o);
  };

  const displayText = selectedLabel ?? placeholder;
  const showPlaceholder = !selectedLabel;

  return (
    <div className={`editor-select-wrap${className ? ` ${className}` : ""}`}>
      <button
        ref={triggerRef}
        type="button"
        id={id}
        className={`editor-select-trigger${open ? " is-open" : ""}`}
        disabled={disabled}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={`${id}-listbox`}
        onClick={toggle}
        onKeyDown={(e: ReactKeyEvent) => {
          if (!open && (e.key === "ArrowDown" || e.key === "ArrowUp")) {
            e.preventDefault();
            setOpen(true);
          }
        }}
      >
        <span
          className={`editor-select-value${showPlaceholder ? " editor-select-placeholder" : ""}`}
        >
          {displayText}
        </span>
        <span className="editor-select-chevron" aria-hidden />
      </button>
      {open &&
        pos &&
        createPortal(
          <ul
            ref={menuRef}
            id={`${id}-listbox`}
            role="listbox"
            aria-labelledby={id}
            className="editor-select-menu editor-select-menu--portal"
            style={{
              position: "fixed",
              top: pos.top,
              left: pos.left,
              width: pos.width,
              maxHeight: pos.maxHeight,
              zIndex: 90,
            }}
          >
            {options.map((opt, i) => (
              <li
                key={opt.value}
                role="option"
                aria-selected={value === opt.value}
                aria-disabled={opt.disabled || undefined}
                className={`editor-select-option${value === opt.value ? " is-selected" : ""}${
                  i === highlightedIndex ? " is-highlighted" : ""
                }${opt.disabled ? " is-disabled" : ""}`}
                onMouseEnter={() => {
                  if (!opt.disabled) setHighlightedIndex(i);
                }}
                onMouseDown={(e) => {
                  e.preventDefault();
                  if (!opt.disabled) {
                    onChange(opt.value);
                    setOpen(false);
                    queueMicrotask(() => triggerRef.current?.focus());
                  }
                }}
              >
                {opt.label}
              </li>
            ))}
          </ul>,
          document.body,
        )}
    </div>
  );
}
