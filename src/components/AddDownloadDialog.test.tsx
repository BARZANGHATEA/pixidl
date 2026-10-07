import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AddDownloadDialog, isAcceptableUrl, isInside } from "./AddDownloadDialog";
import { useUi } from "../stores/ui";
import { useSettings } from "../stores/settings";
import { api } from "../services/api";
import type { Settings, UrlInspection } from "../types";

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(() => Promise.resolve(null)) }));
vi.mock("../services/api", () => ({
  api: {
    inspectUrl: vi.fn(),
    addDownload: vi.fn(),
    readTorrentFile: vi.fn(),
  },
}));

const settings = { defaultDownloadDir: "/home/me/Downloads", extraDownloadDirs: [], askForDestination: false } as unknown as Settings;

function inspection(over: Partial<UrlInspection> = {}): UrlInspection {
  return { url: "", finalUrl: null, engine: "http", alternatives: [], filename: "file.zip", totalBytes: 2048, contentType: "application/zip", resumable: true, category: "Archives", video: null, torrent: null, warning: null, ...over };
}

describe("AddDownloadDialog", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useSettings.setState({ settings, categories: [{ name: "General", extensions: "", subfolder: "General", builtin: true, sortOrder: 0 }, { name: "Archives", extensions: "zip", subfolder: "Archives", builtin: true, sortOrder: 5 }] });
  });

  it("validates URLs", () => {
    expect(isAcceptableUrl("https://example.com/a.zip")).toBe(true);
    expect(isAcceptableUrl("magnet:?xt=urn:btih:abc")).toBe(true);
    expect(isAcceptableUrl("magnet:?dn=x")).toBe(false);
    expect(isAcceptableUrl("javascript:alert(1)")).toBe(false);
    expect(isAcceptableUrl("file:///etc/passwd")).toBe(false);
    expect(isInside("C:\\Users\\me\\Downloads\\sub", ["C:\\Users\\me\\Downloads"])).toBe(true);
    expect(isInside("/home/me/Downloads2", ["/home/me/Downloads"])).toBe(false);
  });

  it("inspects the URL and submits a real request", async () => {
    vi.mocked(api.inspectUrl).mockResolvedValue(inspection());
    vi.mocked(api.addDownload).mockResolvedValue({ filename: "file.zip" } as never);
    useUi.setState({ addDialog: { open: true, url: "https://example.com/file.zip", engine: null } });
    render(<AddDownloadDialog />);
    await waitFor(() => expect(screen.getByDisplayValue("file.zip")).toBeInTheDocument());
    expect(screen.getByText(/2\.0 KB/)).toBeInTheDocument();
    expect(screen.getByText(/Resumable/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Download" }));
    await waitFor(() => expect(api.addDownload).toHaveBeenCalled());
    const req = vi.mocked(api.addDownload).mock.calls[0][0];
    expect(req).toMatchObject({ url: "https://example.com/file.zip", engine: "http", category: "Archives", priority: "normal", startPaused: false });
    expect(req.filename).toBeUndefined(); // the detected name is not forced
    expect(useUi.getState().addDialog.open).toBe(false);
  });

  it("offers video qualities from the extractor", async () => {
    vi.mocked(api.inspectUrl).mockResolvedValue(
      inspection({
        engine: "video",
        filename: "Clip",
        video: {
          id: "x", title: "Clip", thumbnail: null, durationSeconds: 75, uploader: "Someone", extractor: "Generic", webpageUrl: "", isLive: false, ffmpegAvailable: true, formats: [],
          presets: [
            { selector: "bv*+ba/b", label: "Best quality", height: 1080, audioOnly: false, approxSize: 5000 },
            { selector: "bv*[height<=720]+ba/b[height<=720]", label: "720p", height: 720, audioOnly: false, approxSize: 3000 },
            { selector: "ba/b", label: "Audio only", height: null, audioOnly: true, approxSize: 300 },
          ],
        },
      }),
    );
    vi.mocked(api.addDownload).mockResolvedValue({ filename: "Clip" } as never);
    useUi.setState({ addDialog: { open: true, url: "https://video.example/watch?v=1", engine: null } });
    render(<AddDownloadDialog />);
    await waitFor(() => expect(screen.getByRole("button", { name: /720p/ })).toBeInTheDocument());
    expect(screen.getByText(/1:15/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: /Audio only/ }));
    await userEvent.click(screen.getByRole("button", { name: "Download" }));
    await waitFor(() => expect(api.addDownload).toHaveBeenCalled());
    expect(vi.mocked(api.addDownload).mock.calls[0][0]).toMatchObject({ engine: "video", engineOptions: { formatId: "ba/b", audioOnly: true } });
  });

  it("lets the user choose torrent files", async () => {
    vi.mocked(api.inspectUrl).mockResolvedValue(
      inspection({ engine: "torrent", filename: "pack", torrent: { name: "pack", infoHash: "aa", totalBytes: 300, files: [{ index: 0, path: "a.bin", size: 100 }, { index: 1, path: "b.bin", size: 200 }] } }),
    );
    vi.mocked(api.addDownload).mockResolvedValue({ filename: "pack" } as never);
    useUi.setState({ addDialog: { open: true, url: "magnet:?xt=urn:btih:aaaa", engine: null } });
    render(<AddDownloadDialog />);
    await waitFor(() => expect(screen.getByText("a.bin")).toBeInTheDocument());
    await userEvent.click(screen.getByRole("checkbox", { name: /a\.bin/ }));
    await userEvent.click(screen.getByRole("button", { name: "Download" }));
    await waitFor(() => expect(api.addDownload).toHaveBeenCalled());
    expect(vi.mocked(api.addDownload).mock.calls[0][0].engineOptions.torrentFiles).toEqual([1]);
  });

  it("shows inspection errors without blocking", async () => {
    vi.mocked(api.inspectUrl).mockResolvedValue(inspection({ filename: null, totalBytes: null, warning: { kind: "not_found", message: "File not found", detail: "HTTP status 404" } }));
    useUi.setState({ addDialog: { open: true, url: "https://example.com/missing", engine: null } });
    render(<AddDownloadDialog />);
    await waitFor(() => expect(screen.getByText(/File not found/)).toBeInTheDocument());
  });

  it("rejects invalid input", async () => {
    useUi.setState({ addDialog: { open: true, url: "", engine: null } });
    render(<AddDownloadDialog />);
    await userEvent.type(screen.getByLabelText("Paste URL"), "not a url");
    expect(screen.getByText("Enter a valid http(s) link or magnet link.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Download" })).toBeDisabled();
    expect(api.inspectUrl).not.toHaveBeenCalled();
  });
});
