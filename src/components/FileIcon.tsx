import { AppWindow, Disc3, File, FileArchive, FileText, Film, Image, Magnet, Music } from "lucide-react";
import { fileKind, type FileKind } from "../lib/fileType";
import type { Download } from "../types";

const ICONS: Record<FileKind, typeof File> = {
  video: Film,
  audio: Music,
  archive: FileArchive,
  program: AppWindow,
  document: FileText,
  image: Image,
  torrent: Magnet,
  disc: Disc3,
  generic: File,
};

export function FileIcon({ download, size }: { download: Pick<Download, "filename" | "engine">; size?: "sm" }) {
  const kind = fileKind(download);
  const Icon = ICONS[kind];
  return (
    <div className={`ficon ${kind}`} aria-hidden="true" data-kind={kind} style={size === "sm" ? { width: 30, height: 30, borderRadius: 8 } : undefined}>
      <Icon />
    </div>
  );
}
