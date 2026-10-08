// Inline SVG icons for the extension pages (built with createElementNS, no HTML strings).

const SVG_NS = "http://www.w3.org/2000/svg";

/** Stroke paths on a 24x24 grid. `dot:` entries are filled circles "cx cy r". */
const PATHS = {
  video: ["M3 7a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z", "M10 9.2v5.6l4.6-2.8z"],
  audio: ["M9 18V6l10-2v12", "dot:6.5 18 2.5", "dot:16.5 16 2.5"],
  archive: ["M3 5.5A1.5 1.5 0 0 1 4.5 4h15A1.5 1.5 0 0 1 21 5.5V9H3z", "M5 9v9a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V9", "M10 13h4"],
  program: ["M3 6.5A2.5 2.5 0 0 1 5.5 4h13A2.5 2.5 0 0 1 21 6.5v11a2.5 2.5 0 0 1-2.5 2.5h-13A2.5 2.5 0 0 1 3 17.5z", "M3 9h18", "M8 13l2.5 2.5L8 18", "M13 18h3"],
  document: ["M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z", "M14 3v5h5", "M9 13h6", "M9 17h6"],
  image: ["M3 6.5A2.5 2.5 0 0 1 5.5 4h13A2.5 2.5 0 0 1 21 6.5v11a2.5 2.5 0 0 1-2.5 2.5h-13A2.5 2.5 0 0 1 3 17.5z", "dot:9 10 1.8", "M21 15.5l-5-5L6.5 20"],
  torrent: ["M6 4v8a6 6 0 0 0 12 0V4", "M6 8.5h4", "M14 8.5h4", "M10 4v8a2 2 0 0 0 4 0V4"],
  other: ["M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z", "M14 3v5h5"],
  pause: ["M9 6v12", "M15 6v12"],
  resume: ["M8 5.5v13l10.5-6.5z"],
  cancel: ["M6.5 6.5l11 11", "M17.5 6.5l-11 11"],
  search: ["dot-stroke:11 11 6.5", "M16 16l4.5 4.5"],
  check: ["M5 12.5l4.5 4.5L19 7.5"],
  alert: ["M12 8v5", "dot:12 16.5 1.2", "M10.3 3.9L2.6 17.5A2 2 0 0 0 4.3 20.5h15.4a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0z"],
};

/** Colors match the pixidl app's file-type palette. */
export const TYPE_COLORS = Object.freeze({
  video: "#ef4444",
  audio: "#8b5cf6",
  archive: "#f59e0b",
  program: "#64748b",
  document: "#3b82f6",
  image: "#14b8a6",
  torrent: "#22c55e",
  other: "#a1a1aa",
});

export function icon(name, size = 18) {
  const svg = document.createElementNS(SVG_NS, "svg");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("width", String(size));
  svg.setAttribute("height", String(size));
  svg.setAttribute("aria-hidden", "true");
  svg.setAttribute("focusable", "false");
  svg.setAttribute("fill", "none");
  svg.setAttribute("stroke", "currentColor");
  svg.setAttribute("stroke-width", "1.8");
  svg.setAttribute("stroke-linecap", "round");
  svg.setAttribute("stroke-linejoin", "round");
  for (const spec of PATHS[name] ?? PATHS.other) {
    if (spec.startsWith("dot")) {
      const [cx, cy, r] = spec.slice(spec.indexOf(":") + 1).split(" ");
      const circle = document.createElementNS(SVG_NS, "circle");
      circle.setAttribute("cx", cx);
      circle.setAttribute("cy", cy);
      circle.setAttribute("r", r);
      if (spec.startsWith("dot:")) {
        circle.setAttribute("fill", "currentColor");
        circle.setAttribute("stroke", "none");
      }
      svg.append(circle);
    } else {
      const path = document.createElementNS(SVG_NS, "path");
      path.setAttribute("d", spec);
      svg.append(path);
    }
  }
  return svg;
}

/** A file-type badge: the type icon on a tinted rounded square. */
export function typeBadge(type, size = 32) {
  const badge = document.createElement("span");
  badge.className = "ftype";
  badge.dataset.type = type;
  badge.style.setProperty("--type-color", TYPE_COLORS[type] ?? TYPE_COLORS.other);
  badge.style.width = badge.style.height = `${size}px`;
  badge.append(icon(type, Math.round(size * 0.56)));
  return badge;
}
