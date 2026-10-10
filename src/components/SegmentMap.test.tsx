import { act, render, screen } from "@testing-library/react";
import { DownloadSegments, SegmentMap } from "./SegmentMap";
import { api } from "../services/api";
import type { SegmentView } from "../types";

vi.mock("../services/api", () => ({ api: { getSegments: vi.fn() } }));

const MB = 1024 * 1024;
const view: SegmentView = {
  total: 8 * MB,
  connections: 2,
  segments: [
    { start: 0, end: 2 * MB, downloaded: 2 * MB, active: false },
    { start: 2 * MB, end: 4 * MB, downloaded: MB, active: true },
    { start: 4 * MB, end: 8 * MB, downloaded: MB, active: true },
  ],
};

describe("SegmentMap", () => {
  beforeEach(() => vi.clearAllMocks());
  afterEach(() => vi.useRealTimers());

  it("draws every part at its position with its received share", () => {
    render(<SegmentMap view={view} />);
    expect(screen.getByText(/3 parts/)).toBeInTheDocument();
    expect(screen.getByText(/Active connections: 2/)).toBeInTheDocument();
    expect(screen.getByRole("img")).toHaveAccessibleName("Download progress by part: 50% of 3 parts");
    const segs = screen.getAllByTestId("segment");
    expect(segs).toHaveLength(3);
    expect(segs[1]).toHaveClass("active");
    expect(segs[0]).not.toHaveClass("active");
    expect(parseFloat(segs[2].style.insetInlineStart)).toBe(50);
    expect(parseFloat(segs[2].style.width)).toBe(50);
    expect(parseFloat((segs[2].firstChild as HTMLElement).style.width)).toBe(25);
    expect(parseFloat((segs[0].firstChild as HTMLElement).style.width)).toBe(100);
  });

  it("polls while the download is active", async () => {
    vi.useFakeTimers();
    vi.mocked(api.getSegments).mockResolvedValue(view);
    const { unmount } = render(<DownloadSegments id="a" live />);
    await act(async () => {});
    expect(api.getSegments).toHaveBeenCalledTimes(1);
    expect(screen.getAllByTestId("segment")).toHaveLength(3);
    await act(async () => {
      vi.advanceTimersByTime(1500);
    });
    expect(api.getSegments).toHaveBeenCalledTimes(3);
    unmount();
    vi.advanceTimersByTime(2000);
    expect(api.getSegments).toHaveBeenCalledTimes(3);
  });

  it("reads once and renders nothing for single-stream downloads", async () => {
    vi.useFakeTimers();
    vi.mocked(api.getSegments).mockResolvedValue(null);
    const { container } = render(<DownloadSegments id="b" live={false} />);
    await act(async () => {
      vi.advanceTimersByTime(3000);
    });
    expect(api.getSegments).toHaveBeenCalledTimes(1);
    expect(container).toBeEmptyDOMElement();
  });
});
