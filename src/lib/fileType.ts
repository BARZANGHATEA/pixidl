import type { Download } from "../types";

export type FileKind = "video" | "audio" | "archive" | "program" | "document" | "image" | "torrent" | "disc" | "generic";

const EXT: Record<string, FileKind> = {};
const add = (kind: FileKind, list: string) => list.split(" ").forEach((e) => (EXT[e] = kind));
add("video", "mp4 mkv webm avi mov wmv flv m4v mpg mpeg 3gp ts");
add("audio", "mp3 flac wav aac ogg opus m4a wma");
add("archive", "zip rar 7z tar gz bz2 xz zst cab");
add("program", "exe msi msix appx dmg pkg deb rpm appimage apk bat");
add("document", "pdf doc docx xls xlsx ppt pptx odt ods txt rtf epub csv md");
add("image", "jpg jpeg png gif webp svg bmp tif tiff heic avif");
add("disc", "iso img");
add("torrent", "torrent");

export function extensionOf(name: string): string {
  const i = name.lastIndexOf(".");
  return i > 0 ? name.slice(i + 1).toLowerCase() : "";
}

export function fileKind(d: Pick<Download, "filename" | "engine">): FileKind {
  const byExt = EXT[extensionOf(d.filename)];
  if (byExt) return byExt;
  if (d.engine === "torrent") return "torrent";
  if (d.engine === "video") return "video";
  return "generic";
}
