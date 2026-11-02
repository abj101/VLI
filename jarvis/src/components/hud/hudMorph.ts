/** Match the surface to actual host bounds, including frames delayed by WebView2 resizing. */
export function dictationMorphGeometry(width: number, height: number, sourceWidth: number, sourceRadius = 12) {
  // Caret-sized hosts are 20–36px including the transparent edge allowance.
  const progress = sourceWidth <= 64 ? 1 : Math.max(0, Math.min(1, (sourceWidth - width) / (sourceWidth - 40)));
  const inset = 2 * progress;
  const circleRadius = Math.max(0, Math.min(width, height) / 2 - inset);
  return { inset, radius: sourceRadius + (circleRadius - sourceRadius) * progress };
}
