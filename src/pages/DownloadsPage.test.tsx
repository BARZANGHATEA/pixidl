import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { DownloadsPage } from "./DownloadsPage";
import { useDownloads } from "../stores/downloads";
import { useUi } from "../stores/ui";
import { makeDownload } from "../test/fixtures";

vi.mock("../services/api", () => ({ api: { pauseAll: vi.fn(() => Promise.resolve()), resumeAll: vi.fn(() => Promise.resolve()) } }));

function seed() {
  const list = [
    makeDownload({ filename: "alpha.zip", status: "downloading" }),
    makeDownload({ filename: "beta.mp4", status: "completed", engine: "video", category: "Videos" }),
    makeDownload({ filename: "gamma.exe", status: "failed", category: "Programs" }),
  ];
  useDownloads.setState({ byId: Object.fromEntries(list.map((d) => [d.id, d])), loaded: true });
}

describe("DownloadsPage", () => {
  beforeEach(() => {
    useUi.setState({ view: "downloads", scope: { kind: "all" }, status: "all", query: "", sort: "name" });
  });

  it("shows an empty state with zero downloads", () => {
    useDownloads.setState({ byId: {}, loaded: true });
    render(<DownloadsPage />);
    expect(screen.getByText("No downloads yet")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Add a download" })).toBeInTheDocument();
  });

  it("filters with the tabs", async () => {
    seed();
    render(<DownloadsPage />);
    const list = () => within(screen.getByRole("list"));
    expect(list().getAllByRole("listitem")).toHaveLength(3);
    await userEvent.click(screen.getByRole("tab", { name: /Completed/ }));
    expect(list().getAllByRole("listitem")).toHaveLength(1);
    expect(list().getByText("beta.mp4")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("tab", { name: /Failed/ }));
    expect(list().getByText("gamma.exe")).toBeInTheDocument();
  });

  it("searches", async () => {
    seed();
    render(<DownloadsPage />);
    await userEvent.click(screen.getByRole("button", { name: "Search downloads" }));
    await userEvent.type(screen.getByRole("textbox", { name: "Search downloads" }), "programs");
    expect(within(screen.getByRole("list")).getAllByRole("listitem")).toHaveLength(1);
    await userEvent.clear(screen.getByRole("textbox", { name: "Search downloads" }));
    await userEvent.type(screen.getByRole("textbox", { name: "Search downloads" }), "nothing-matches");
    expect(screen.getByText("Nothing here")).toBeInTheDocument();
  });

  it("scopes to video", () => {
    seed();
    useUi.setState({ scope: { kind: "engine", engine: "video" } });
    render(<DownloadsPage />);
    expect(screen.getByRole("heading", { name: "Video" })).toBeInTheDocument();
    expect(within(screen.getByRole("list")).getAllByRole("listitem")).toHaveLength(1);
  });
});
