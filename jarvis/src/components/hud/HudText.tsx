import { useLayoutEffect, useRef } from "react";

/** Keep existing words mounted so streaming updates only fade in new/revised words. */
export function HudText({
  text,
  kind,
}: {
  text: string;
  kind: "transcript" | "match" | "action" | "error";
}) {
  const viewportRef = useRef<HTMLDivElement>(null);
  const trackRef = useRef<HTMLDivElement>(null);

  useLayoutEffect(() => {
    const viewport = viewportRef.current;
    const track = trackRef.current;
    if (!viewport || !track) return;
    if (kind !== "transcript") {
      track.style.removeProperty("transform");
      return;
    }

    const followTail = () => {
      const overflow = Math.max(0, track.scrollHeight - viewport.clientHeight);
      track.style.transform = `translateY(${-overflow}px)`;
    };
    followTail();
    const observer = new ResizeObserver(followTail);
    observer.observe(viewport);
    observer.observe(track);
    return () => observer.disconnect();
  }, [text, kind]);

  return (
    <div
      ref={viewportRef}
      className={`hud-text hud-text--${kind}`}
      role={kind === "error" ? "alert" : undefined}
    >
      <div ref={trackRef} className="hud-text-track">
        <span className="hud-text-content">
          {(text.match(/\S+\s*/gu) ?? []).map((word, index) => (
            <span className="hud-text-word" key={`${index}-${word.trimEnd()}`}>
              {word}
            </span>
          ))}
        </span>
      </div>
    </div>
  );
}
