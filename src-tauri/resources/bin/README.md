Optional bundled engines.

`scripts/fetch-engines.mjs` places the official, checksum-verified yt-dlp
binary here before packaging. FFmpeg is not bundled by default (see BUILD.md).
Files in this folder are installed to `<install dir>/bin` and searched first.
